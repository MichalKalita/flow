//! Maps entities onto SQLite rows and checks values already stored there.
//! Allocates positive numeric ids. Does not decide permissions or commit a transaction.

use crate::{
    Error, Result,
    program::{Program, Type},
    value::Value,
};
use chrono::Utc;
use rusqlite::{Connection, OptionalExtension};
use std::collections::BTreeMap;

use super::Config;

pub(in crate::engine) fn err(code: &'static str) -> Error {
    Error::new(code, code)
}
pub(in crate::engine) fn q(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}
pub(in crate::engine) fn stored_type(p: &Program, entity: &str) -> Result<Type> {
    let fields = &p
        .entities
        .get(entity)
        .ok_or_else(|| err("invalid_program"))?
        .fields;
    Ok(Type::Record(
        fields
            .iter()
            .filter(|(_, f)| f.relation.is_none())
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect(),
    ))
}
pub(in crate::engine) fn wire_type(p: &Program, t: &Type) -> Result<Type> {
    Ok(match p.resolve(t)? {
        Type::Named(entity) if p.entities.contains_key(entity) => Type::Id(entity.clone()),
        Type::Record(fields) => Type::Record(
            fields
                .iter()
                .map(|(name, field)| {
                    let mut field = field.clone();
                    field.ty = wire_type(p, &field.ty)?;
                    Ok((name.clone(), field))
                })
                .collect::<Result<_>>()?,
        ),
        Type::List(inner, min, max) => Type::List(Box::new(wire_type(p, inner)?), *min, *max),
        Type::Optional(inner) => Type::Optional(Box::new(wire_type(p, inner)?)),
        t => t.clone(),
    })
}
pub(in crate::engine) fn request_id() -> i64 {
    use std::sync::atomic::{AtomicI64, Ordering};
    static LAST: AtomicI64 = AtomicI64::new(0);
    let now = Utc::now().timestamp_micros();
    LAST.try_update(Ordering::SeqCst, Ordering::SeqCst, |last| {
        Some(now.max(last + 1))
    })
    .unwrap()
    .max(now - 1)
        + 1
}
pub(in crate::engine) fn next_id(db: &Connection, entity: &str, floor: i64) -> Result<i64> {
    if entity == "Request" {
        return Ok(request_id());
    }
    let sql = format!(
        "INSERT INTO _flow_id_sequences(entity,value) VALUES(?1,(SELECT MAX(COALESCE(MAX(id),0),?2)+1 FROM {})) ON CONFLICT(entity) DO UPDATE SET value=MAX(value,(SELECT COALESCE(MAX(id),0) FROM {}),?2)+1 RETURNING value",
        q(entity),
        q(entity)
    );
    let id: i64 = db.query_row(&sql, rusqlite::params![entity, floor], |row| row.get(0))?;
    if id > 9_007_199_254_740_991 {
        return Err(err("limit"));
    }
    Ok(id)
}
pub(in crate::engine) fn exists(db: &Connection, entity: &str, id: &i64) -> Result<bool> {
    Ok(db
        .query_row(
            &format!("SELECT id FROM {} WHERE id=?1", q(entity)),
            [id],
            |_| Ok(()),
        )
        .optional()?
        .is_some())
}
pub(in crate::engine) fn storage(v: &Value) -> Result<Option<String>> {
    Ok(match v {
        Value::Null => None,
        Value::Ref { id, .. } | Value::Id(_, id) => Some(id.to_string()),
        _ => Some(v.json()?.to_string()),
    })
}
pub(in crate::engine) fn insert(
    db: &Connection,
    entity: &str,
    values: &BTreeMap<String, Value>,
) -> Result<()> {
    let columns = values.keys().map(|s| q(s)).collect::<Vec<_>>().join(",");
    let placeholders = (1..=values.len())
        .map(|i| format!("?{i}"))
        .collect::<Vec<_>>()
        .join(",");
    let params = values.values().map(storage).collect::<Result<Vec<_>>>()?;
    db.execute(
        &format!(
            "INSERT INTO {} ({columns}) VALUES ({placeholders})",
            q(entity)
        ),
        rusqlite::params_from_iter(params),
    )?;
    Ok(())
}
pub(in crate::engine) fn lookup(
    db: &Connection,
    auth: &crate::program::Auth,
    value: &str,
) -> Result<Option<Value>> {
    let encoded = serde_json::to_string(value)?;
    let id: Option<i64> = db
        .query_row(
            &format!(
                "SELECT id FROM {} WHERE {}=?1",
                q(&auth.entity),
                q(&auth.field)
            ),
            [encoded],
            |r| r.get(0),
        )
        .optional()?;
    Ok(id.map(|id| Value::reference(&auth.entity, &id)))
}
pub(in crate::engine) fn validate_stored_data(
    database: &Connection,
    program: &Program,
    pause_limit: Option<&crate::migrations::PauseLimit<'_>>,
) -> Result<()> {
    let integrity: String = database.query_row("PRAGMA integrity_check", [], |row| row.get(0))?;
    if integrity != "ok" {
        return Err(Error::new(
            "database",
            "Snapshot failed integrity validation",
        ));
    }
    let foreign_key_problem: bool = database
        .prepare("PRAGMA foreign_key_check")?
        .query([])?
        .next()?
        .is_some();
    if foreign_key_problem {
        return Err(Error::new(
            "database",
            "Snapshot contains invalid references",
        ));
    }
    for (name, entity) in &program.entities {
        let fields = entity
            .fields
            .iter()
            .filter(|(_, field)| field.relation.is_none())
            .map(|(name, _)| name)
            .collect::<Vec<_>>();
        let columns = fields
            .iter()
            .map(|field| q(field))
            .collect::<Vec<_>>()
            .join(",");
        let sizes = fields
            .iter()
            .map(|field| format!("coalesce(length({}),0)", q(field)))
            .collect::<Vec<_>>()
            .join("+");
        let oversized: i64 = database.query_row(
            &format!("SELECT count(*) FROM {} WHERE {sizes}>16777216", q(name)),
            [],
            |row| row.get(0),
        )?;
        if oversized > 0 {
            return Err(Error::new(
                "limit",
                "Stored record exceeds validation limits",
            ));
        }
        let mut statement = database.prepare(&format!("SELECT {columns} FROM {}", q(name)))?;
        let mut rows = statement.query([])?;
        while let Some(row) = rows.next()? {
            if let Some(limit) = pause_limit {
                limit.check()?;
            }
            let mut values = BTreeMap::new();
            for (index, field) in fields.iter().enumerate() {
                let value = match row.get_ref(index)? {
                    rusqlite::types::ValueRef::Integer(n) => {
                        Value::from_json(&serde_json::json!(n))?
                    }
                    rusqlite::types::ValueRef::Text(bytes) => {
                        Value::from_json(&serde_json::from_slice(bytes)?)?
                    }
                    rusqlite::types::ValueRef::Null => Value::Null,
                    _ => return Err(Error::new("database", "Invalid stored field encoding")),
                };
                values.insert((*field).clone(), value);
            }
            program.validate(&stored_type(program, name)?, Value::record(values), None)?;
        }
    }
    Ok(())
}
pub(in crate::engine) fn validate_program_configuration(
    program: &Program,
    config: &Config,
) -> Result<()> {
    for (alias, key) in &config.jwt_keys {
        if key.len() < 32
            || !program
                .auth
                .iter()
                .any(|a| a.alias == *alias && a.mode == "jwt")
        {
            return Err(Error::new(
                "configuration",
                "JWT key must have at least 32 bytes and match a declared adapter",
            ));
        };
    }
    for operation in &program.operations {
        if let Some(event) = &operation.event
            && !config
                .event_credentials
                .contains_key(&format!("{}:{}", event.adapter, event.actor_id))
        {
            return Err(Error::new(
                "configuration",
                format!(
                    "Missing verified event actor {}:{}",
                    event.adapter, event.actor_id
                ),
            ));
        }
    }
    Ok(())
}
