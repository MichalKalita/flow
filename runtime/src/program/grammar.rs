//! Reads surface forms into types, identifiers, and literal values.
//! Rejects malformed declarations. Does not evaluate expressions or touch storage.

use crate::{
    Error, Result,
    syntax::Node,
    value::{Number, Value, number},
};
use num_traits::ToPrimitive;
use std::collections::BTreeMap;

use super::model::*;

pub(in crate::program) fn fail(s: impl Into<String>) -> Error {
    Error::new("invalid_program", s)
}
pub fn ident(s: &str) -> Result<String> {
    if !s.is_empty()
        && s.bytes()
            .enumerate()
            .all(|(i, b)| b.is_ascii_alphabetic() || b == b'_' || (i > 0 && b.is_ascii_digit()))
    {
        Ok(s.into())
    } else {
        Err(fail(format!("Invalid identifier {s}")))
    }
}
pub(in crate::program) fn unique<T>(
    map: &mut BTreeMap<String, T>,
    key: String,
    value: T,
) -> Result<()> {
    if map.insert(key.clone(), value).is_some() {
        Err(fail(format!("Duplicate {key}")))
    } else {
        Ok(())
    }
}
pub(in crate::program) fn opt<'a>(node: &'a Node, name: &str) -> Result<&'a Node> {
    node.option(name)
        .ok_or_else(|| fail(format!("Missing {name} in {}", node.head())))
}
pub(in crate::program) fn constant(n: &Node) -> Result<Number> {
    if let Node::Number(s) = n {
        return number(s);
    };
    let a = constant(n.arg(0)?)?;
    let b = constant(n.arg(1)?)?;
    let value = match n.head() {
        "add" => a + b,
        "sub" => a - b,
        "mul" => a * b,
        "pow" => {
            let e = b
                .to_integer()
                .to_i32()
                .filter(|e| b.is_integer() && *e >= 0 && *e <= 100)
                .ok_or_else(|| fail("Invalid exponent"))?;
            a.pow(e)
        }
        _ => return Err(fail("Invalid numeric constant")),
    };
    crate::value::decimal(&value)?;
    Ok(value)
}
pub(in crate::program) fn size(n: &Node) -> Result<usize> {
    constant(n)?
        .to_integer()
        .to_usize()
        .filter(|_| constant(n).unwrap().is_integer())
        .filter(|n| *n <= 100000)
        .ok_or_else(|| fail("Invalid collection limit"))
}
pub(in crate::program) fn fields(nodes: &[Node]) -> Result<BTreeMap<String, Field>> {
    let mut fields = BTreeMap::new();
    for f in nodes {
        if f.head() != "field" {
            return Err(fail("Expected field"));
        };
        let name = ident(f.arg(0)?.text()?)?;
        let ty = ty(f.arg(1)?)?;
        let relation = f
            .option("inverse")
            .or_else(|| f.option("stream"))
            .map(|r| {
                let path = r.arg(0)?.text()?;
                let (entity, field) = path
                    .split_once('.')
                    .ok_or_else(|| fail("Relation needs Entity.field"))?;
                Ok::<_, Error>((ident(entity)?, ident(field)?))
            })
            .transpose()?;
        unique(
            &mut fields,
            name,
            Field {
                ty,
                relation,
                unique: f.option("unique").is_some(),
                generated: f.option("receivedAt").is_some(),
            },
        )?;
    }
    Ok(fields)
}
pub(in crate::program) fn ty(n: &Node) -> Result<Type> {
    if let Node::Symbol(s) = n {
        return Ok(match s.as_str() {
            "String" => Type::String,
            "Bool" => Type::Bool,
            "DateTime" => Type::DateTime,
            _ => Type::Named(ident(s)?),
        });
    };
    Ok(match n.head() {
        "string" => {
            let min_bytes = n
                .option("minBytes")
                .map(|n| size(n.arg(0)?))
                .transpose()?
                .unwrap_or(0);
            let max_bytes = size(opt(n, "maxBytes")?.arg(0)?)?;
            if min_bytes > max_bytes || max_bytes > 1024 * 1024 {
                return Err(fail("Invalid string byte limits"));
            }
            Type::Text {
                min_bytes,
                max_bytes,
            }
        }
        "image" => {
            let max_bytes = constant(opt(n, "maxBytes")?.arg(0)?)?
                .to_integer()
                .to_usize()
                .filter(|v| *v > 0 && *v <= 64 * 1024 * 1024)
                .ok_or_else(|| fail("Invalid image byte limit"))?;
            let width = size(opt(n, "maxWidth")?.arg(0)?)? as u32;
            let height = size(opt(n, "maxHeight")?.arg(0)?)? as u32;
            if width == 0 || height == 0 {
                return Err(fail("Image dimensions must be positive"));
            };
            Type::Image {
                max_bytes,
                width,
                height,
            }
        }
        "id" => Type::Id(ident(n.arg(0)?.text()?)?),
        "integer" | "decimal" => {
            let range = opt(n, "range")?;
            let min = constant(range.arg(0)?)?;
            let max = constant(range.arg(1)?)?;
            let scale = if n.head() == "integer" {
                0
            } else {
                size(opt(n, "scale")?.arg(0)?)? as u32
            };
            if min > max || scale > 100 || (scale == 0 && (!min.is_integer() || !max.is_integer()))
            {
                return Err(fail("Invalid range/scale"));
            };
            Type::Number { min, max, scale }
        }
        "list" => Type::List(
            Box::new(ty(n.arg(0)?)?),
            n.option("min")
                .map(|n| size(n.arg(0)?))
                .transpose()?
                .unwrap_or(0),
            n.option("max").map(|n| size(n.arg(0)?)).transpose()?,
        ),
        "optional" => Type::Optional(Box::new(ty(n.arg(0)?)?)),
        "record" => Type::Record(fields(n.args()?)?),
        _ => return Err(fail(format!("Unsupported type {}", n.head()))),
    })
}

pub fn valid_id(value: Value) -> Result<i64> {
    use num_traits::ToPrimitive;
    match value {
        Value::Num(n) if n.is_integer() => n
            .to_integer()
            .to_i64()
            .filter(|id| (1..=9_007_199_254_740_991).contains(id))
            .ok_or_else(|| Error::new("invalid_input", "ID must be a positive safe integer")),
        _ => Err(Error::new(
            "invalid_input",
            "ID must be a positive safe integer",
        )),
    }
}

pub fn literal(n: &Node) -> Result<Value> {
    Ok(match n {
        Node::Number(s) => Value::Num(number(s)?),
        Node::String(s) => Value::Str(s.clone()),
        Node::Symbol(s) => match s.as_str() {
            "true" => Value::Bool(true),
            "false" => Value::Bool(false),
            "null" => Value::Null,
            _ => Value::Str(s.clone()),
        },
        Node::List(_) => match n.head() {
            "list" => Value::List(n.args()?.iter().map(literal).collect::<Result<_>>()?),
            "record" => {
                let mut values = BTreeMap::new();
                for f in n.args()? {
                    unique(&mut values, ident(f.head())?, literal(f.arg(0)?)?)?;
                }
                Value::record(values)
            }
            _ => return Err(fail("Expected literal")),
        },
    })
}
