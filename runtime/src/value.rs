//! Runtime values: records, lists, exact decimals, and branded positive ids.
//! Ids stay numeric. Type safety does not use text prefixes.

use crate::{Error, Result};
use num_bigint::BigInt;
use num_rational::BigRational;
use num_traits::{One, ToPrimitive, Zero};
use std::collections::BTreeMap;

pub type Number = BigRational;
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Image(crate::media::Image),
    Null,
    Bool(bool),
    Str(String),
    Num(Number),
    BrandedNumber(String, Number),
    Id(String, i64),
    Ref {
        entity: String,
        id: i64,
        before: bool,
    },
    List(Vec<Value>),
    Record {
        fields: BTreeMap<String, Value>,
        kind: Option<String>,
        parent: Option<Box<Value>>,
    },
}
impl Value {
    pub fn record(fields: BTreeMap<String, Value>) -> Self {
        Self::Record {
            fields,
            kind: None,
            parent: None,
        }
    }
    pub fn reference(entity: &str, id: &i64) -> Self {
        Self::Ref {
            entity: entity.into(),
            id: *id,
            before: false,
        }
    }
    pub fn id(&self) -> Result<i64> {
        match self {
            Self::Id(_, id) | Self::Ref { id, .. } => Ok(*id),
            value => crate::program::valid_id(value.clone()),
        }
    }
    pub fn text(&self) -> Result<String> {
        match self {
            Self::Str(s) => Ok(s.clone()),
            Self::Id(_, id) | Self::Ref { id, .. } => Ok(id.to_string()),
            Self::Num(n) | Self::BrandedNumber(_, n) => decimal(n),
            _ => Err(Error::new("invalid_input", "Expected text")),
        }
    }
    pub fn number(&self) -> Result<&Number> {
        match self {
            Self::Num(n) | Self::BrandedNumber(_, n) => Ok(n),
            _ => Err(Error::new("invalid_input", "Expected number")),
        }
    }
    pub fn list(&self) -> Result<&[Value]> {
        match self {
            Self::List(v) => Ok(v),
            _ => Err(Error::new("invalid_input", "Expected list")),
        }
    }
    pub fn fields(&self) -> Result<&BTreeMap<String, Value>> {
        match self {
            Self::Record { fields, .. } => Ok(fields),
            _ => Err(Error::new("invalid_input", "Expected record")),
        }
    }
    pub fn truth(&self) -> Result<bool> {
        match self {
            Self::Bool(v) => Ok(*v),
            _ => Err(Error::new("invalid_input", "Expected boolean")),
        }
    }
    pub fn pin(&self) -> Self {
        match self {
            Self::Ref { entity, id, .. } => Self::Ref {
                entity: entity.clone(),
                id: *id,
                before: true,
            },
            Self::List(v) => Self::List(v.iter().map(Self::pin).collect()),
            Self::Record {
                fields,
                kind,
                parent,
            } => Self::Record {
                fields: fields.iter().map(|(k, v)| (k.clone(), v.pin())).collect(),
                kind: kind.clone(),
                parent: parent.as_ref().map(|v| Box::new(v.pin())),
            },
            _ => self.clone(),
        }
    }
    pub fn json(&self) -> Result<serde_json::Value> {
        use serde_json::Value as J;
        Ok(match self {
            Self::Image(image) => {
                use base64::Engine as _;
                J::String(base64::engine::general_purpose::STANDARD.encode(&image.bytes))
            }
            Self::Null => J::Null,
            Self::Bool(v) => J::Bool(*v),
            Self::Str(s) => J::String(s.clone()),
            Self::Id(_, id) | Self::Ref { id, .. } => J::Number((*id).into()),
            Self::Num(n) | Self::BrandedNumber(_, n) => serde_json::from_str(&decimal(n)?)?,
            Self::List(v) => J::Array(v.iter().map(Self::json).collect::<Result<_>>()?),
            Self::Record { fields, .. } => J::Object(
                fields
                    .iter()
                    .map(|(k, v)| Ok((k.clone(), v.json()?)))
                    .collect::<Result<_>>()?,
            ),
        })
    }
    pub fn from_json(v: &serde_json::Value) -> Result<Self> {
        use serde_json::Value as J;
        Ok(match v {
            J::Null => Self::Null,
            J::Bool(v) => Self::Bool(*v),
            J::String(s) => Self::Str(s.clone()),
            J::Number(n) => Self::Num(number(&n.to_string())?),
            J::Array(v) => Self::List(v.iter().map(Self::from_json).collect::<Result<_>>()?),
            J::Object(v) => Self::record(
                v.iter()
                    .map(|(k, v)| Ok((k.clone(), Self::from_json(v)?)))
                    .collect::<Result<_>>()?,
            ),
        })
    }
}
pub fn number(text: &str) -> Result<Number> {
    if text.len() > 4096 {
        return Err(Error::new("invalid_input", "Number too large"));
    }
    let (mantissa, exponent) = text
        .split_once(['e', 'E'])
        .map(|(m, e)| {
            Ok::<_, Error>((
                m,
                e.parse::<i32>()
                    .map_err(|_| Error::new("invalid_input", "Invalid exponent"))?,
            ))
        })
        .unwrap_or(Ok((text, 0)))?;
    if exponent.unsigned_abs() > 1000 {
        return Err(Error::new("invalid_input", "Exponent out of bounds"));
    }
    let (digits, scale) = match mantissa.split_once('.') {
        Some((a, b)) => (format!("{a}{b}"), b.len() as i32),
        None => (mantissa.to_owned(), 0),
    };
    let numerator = digits
        .parse::<BigInt>()
        .map_err(|_| Error::new("invalid_input", "Invalid number"))?;
    let shift = exponent - scale;
    let power = BigInt::from(10).pow(shift.unsigned_abs());
    Ok(if shift >= 0 {
        Number::from_integer(numerator * power)
    } else {
        Number::new(numerator, power)
    })
}
pub fn decimal(n: &Number) -> Result<String> {
    if n.numer().bits() > 65536 || n.denom().bits() > 65536 {
        return Err(Error::new("limit", "Number precision limit"));
    }
    let mut denominator = n.denom().clone();
    let mut scale = 0u32;
    while denominator.clone() % 2 == BigInt::zero() {
        denominator /= 2;
        scale += 1;
    }
    let twos = scale;
    scale = 0;
    while denominator.clone() % 5 == BigInt::zero() {
        denominator /= 5;
        scale += 1;
    }
    if denominator != BigInt::one() {
        return Err(Error::new(
            "invalid_input",
            "Non-terminating decimal requires explicit rounding",
        ));
    }
    let scale = twos.max(scale);
    if scale > 1000 {
        return Err(Error::new("invalid_input", "Decimal precision limit"));
    }
    let numerator = (n * Number::from_integer(BigInt::from(10).pow(scale))).to_integer();
    let negative = numerator < BigInt::zero();
    let digits = numerator.to_string().trim_start_matches('-').to_owned();
    if scale == 0 {
        return Ok(numerator.to_string());
    }
    let padded = format!("{:0>width$}", digits, width = scale as usize + 1);
    let split = padded.len() - scale as usize;
    Ok(format!(
        "{}{}.{}",
        if negative { "-" } else { "" },
        &padded[..split],
        &padded[split..]
    ))
}
pub fn integer(v: usize) -> Value {
    Value::Num(Number::from_integer(v.into()))
}
pub fn count(v: &Value) -> Result<usize> {
    v.number()?
        .to_integer()
        .to_usize()
        .filter(|_| v.number().unwrap().is_integer())
        .ok_or_else(|| Error::new("invalid_input", "Invalid count"))
}
pub fn equal(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::BrandedNumber(a, x), Value::BrandedNumber(b, y)) => a == b && x == y,
        (Value::BrandedNumber(_, x), Value::Num(y))
        | (Value::Num(y), Value::BrandedNumber(_, x)) => x == y,
        (
            Value::Ref {
                entity: a, id: x, ..
            },
            Value::Ref {
                entity: b, id: y, ..
            },
        )
        | (Value::Id(a, x), Value::Id(b, y)) => a == b && x == y,
        (
            Value::Ref {
                entity: a, id: x, ..
            },
            Value::Id(b, y),
        )
        | (
            Value::Id(b, y),
            Value::Ref {
                entity: a, id: x, ..
            },
        ) => a == b && x == y,
        (Value::Record { fields: a, .. }, Value::Record { fields: b, .. }) => {
            a.len() == b.len() && a.iter().all(|(k, v)| b.get(k).is_some_and(|w| equal(v, w)))
        }
        (Value::List(a), Value::List(b)) => {
            a.len() == b.len() && a.iter().zip(b).all(|(a, b)| equal(a, b))
        }
        _ => a == b,
    }
}
