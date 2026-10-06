//! Turns stored text and nested JSON-like values into the program's runtime types.

use crate::{
    Result,
    program::{Program, Type},
    value::{Value, number},
};

pub(in crate::engine) fn wire(p: &Program, t: &Type, v: Value) -> Result<Value> {
    let t = p.resolve(t)?;
    Ok(match (t, v) {
        (Type::Number { .. }, Value::Str(s)) => Value::Num(number(&s)?),
        (Type::Optional(inner), v) if v != Value::Null => wire(p, inner, v)?,
        (Type::List(inner, _, _), Value::List(values)) => Value::List(
            values
                .into_iter()
                .map(|v| wire(p, inner, v))
                .collect::<Result<_>>()?,
        ),
        (Type::Record(fs), Value::Record { fields, .. }) => Value::record(
            fields
                .into_iter()
                .map(|(k, v)| {
                    Ok((
                        k.clone(),
                        match fs.get(&k) {
                            Some(f) => wire(p, &f.ty, v)?,
                            None => v,
                        },
                    ))
                })
                .collect::<Result<_>>()?,
        ),
        (_, v) => v,
    })
}
