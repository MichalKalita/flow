//! Privileged row browser for the protected admin listener only.
//! Checks types, references, and versions, and audits in the same transaction as the write.

use crate::{Error, Result, value::Value};
use chrono::Utc;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

use super::Runtime;
use super::session::{Change, Session};
use super::store::{err, next_id, q, stored_type};

impl Runtime {
    pub fn admin_tables(&self) -> serde_json::Value {
        serde_json::json!(self.program.entities.iter().map(|(name,entity)| {
            serde_json::json!({"name":name,"stream":self.program.streams.contains_key(name),"fields":entity.fields.iter().map(|(name,f)|serde_json::json!({"name":name,"type":format!("{:?}",self.program.resolve(&f.ty).unwrap_or(&f.ty)),"relation":f.relation,"generated":f.generated,"unique":f.unique})).collect::<Vec<_>>()})
        }).collect::<Vec<_>>())
    }
    pub fn admin_rows(
        &self,
        entity: &str,
        after: i64,
        limit: usize,
        search: &str,
        exact: Option<i64>,
    ) -> Result<serde_json::Value> {
        let mut span = self.observability.span("io", "sqlite.admin_read");
        let result = self.admin_rows_inner(entity, after, limit, search, exact);
        if result.is_ok() {
            span.success();
        }
        result
    }
    fn admin_rows_inner(
        &self,
        entity: &str,
        after: i64,
        limit: usize,
        search: &str,
        exact: Option<i64>,
    ) -> Result<serde_json::Value> {
        self.require_available()?;
        let schema = self
            .program
            .entities
            .get(entity)
            .ok_or_else(|| err("not_found"))?;
        let fields = schema
            .fields
            .iter()
            .filter(|(_, f)| f.relation.is_none())
            .map(|(name, _)| name.clone())
            .collect::<Vec<_>>();
        let lengths = fields
            .iter()
            .map(|field| format!("COALESCE(length({}),0)", q(field)))
            .collect::<Vec<_>>()
            .join("+");
        let columns = fields
            .iter()
            .map(|field| {
                format!(
                    "CASE WHEN ({lengths})<=262144 THEN {} ELSE NULL END",
                    q(field)
                )
            })
            .collect::<Vec<_>>()
            .join(",");
        let searchable = fields
            .iter()
            .map(|field| format!("COALESCE(CAST({} AS TEXT),'')", q(field)))
            .collect::<Vec<_>>()
            .join("||' '||");
        let sql = format!(
            "SELECT id,({lengths})>262144,{columns} FROM {} WHERE (?1=0 OR id>?1) AND (?2 IS NULL OR id=?2) AND (?3='' OR instr(lower({searchable}),lower(?3))>0) ORDER BY id LIMIT ?4",
            q(entity)
        );
        let limit = limit.clamp(1, 100);
        let mut statement = self.db.prepare(&sql)?;
        let mut rows = statement.query(rusqlite::params![
            after,
            exact,
            search.chars().take(256).collect::<String>(),
            (limit + 1) as i64
        ])?;
        let mut output = vec![];
        while let Some(row) = rows.next()? {
            let id: i64 = row.get(0)?;
            let oversized: bool = row.get(1)?;
            let mut record = serde_json::Map::new();
            for (index, field) in fields.iter().enumerate() {
                use rusqlite::types::ValueRef;
                let value = match row.get_ref(index + 2)? {
                    ValueRef::Integer(n) => serde_json::json!(n),
                    ValueRef::Text(bytes) => serde_json::from_slice(bytes)?,
                    _ => serde_json::Value::Null,
                };
                record.insert(field.clone(), value);
            }
            record.insert("id".into(), serde_json::json!(id));
            let record = serde_json::Value::Object(record);
            output.push(serde_json::json!({"record":record,"etag":if oversized {None}else{Some(format!("{:x}",Sha256::digest(record.to_string())))},"oversized":oversized}));
        }
        let more = output.len() > limit;
        output.truncate(limit);
        let cursor = if more {
            output.last().map(|row| row["record"]["id"].clone())
        } else {
            None
        };
        Ok(serde_json::json!({"rows":output,"next_cursor":cursor,"entity":entity}))
    }
    pub fn admin_write(
        &mut self,
        action: &str,
        entity: &str,
        id: Option<i64>,
        input: serde_json::Value,
        expected: Option<&str>,
    ) -> Result<serde_json::Value> {
        let mut span = self.observability.span("io", "sqlite.admin_write");
        let result = self.admin_write_inner(action, entity, id, input, expected);
        if result.is_ok() {
            span.success();
        }
        result
    }
    fn admin_write_inner(
        &mut self,
        action: &str,
        entity: &str,
        id: Option<i64>,
        input: serde_json::Value,
        expected: Option<&str>,
    ) -> Result<serde_json::Value> {
        self.require_available()?;
        let gate = self.gate.clone();
        let _activity = gate
            .as_ref()
            .map(|gate| {
                gate.lock
                    .read()
                    .map_err(|_| Error::new("internal", "Runtime activity lock failed"))
            })
            .transpose()?;
        self.require_available()?;
        if !["CREATE", "UPDATE", "DELETE"].contains(&action) {
            return Err(err("invalid_input"));
        }
        if !self.program.entities.contains_key(entity) {
            return Err(err("not_found"));
        }
        if action != "DELETE" && !input.is_object() {
            return Err(err("invalid_input"));
        }
        self.db.execute_batch("BEGIN IMMEDIATE;")?;
        let result = (|| -> Result<serde_json::Value> {
            let mut json = input.as_object().cloned().unwrap_or_default();
            let id = if action == "CREATE" {
                match json.get("id") {
                    Some(value) => crate::program::valid_id(Value::from_json(value)?)?,
                    None => next_id(&self.db, entity, 0)?,
                }
            } else {
                id.ok_or_else(|| err("invalid_input"))?
            };
            if action != "CREATE" {
                let rows = self.admin_rows(entity, 0, 1, "", Some(id))?;
                let current = rows["rows"]
                    .as_array()
                    .and_then(|rows| rows.first())
                    .ok_or_else(|| err("not_found"))?;
                if expected.is_none() || current["etag"].as_str() != expected {
                    return Err(Error::new(
                        "conflict",
                        "Record changed; reload it before writing",
                    ));
                }
            }
            if action != "DELETE" {
                if json
                    .get("id")
                    .is_some_and(|value| value != &serde_json::json!(id))
                {
                    return Err(Error::new(
                        "invalid_input",
                        "Primary keys cannot be changed",
                    ));
                }
                json.insert("id".into(), serde_json::json!(id));
            }
            let reference = Value::reference(entity, &id);
            let after = if action == "DELETE" {
                BTreeMap::new()
            } else {
                if action == "CREATE" {
                    for (field, f) in &self.program.entities[entity].fields {
                        if f.generated && field != "id" && !json.contains_key(field) {
                            json.insert(field.clone(), serde_json::json!(Utc::now().to_rfc3339()));
                        }
                    }
                }
                self.program
                    .validate(
                        &stored_type(&self.program, entity)?,
                        Value::from_json(&serde_json::Value::Object(json))?,
                        Some(reference.clone()),
                    )?
                    .fields()?
                    .clone()
            };
            let mut session = Session {
                p: &self.program,
                db: &self.db,
                observer: &self.observability,
                base_path: &self.config.base_path,
                actor: Value::Null,
                actor_type: "admin".into(),
                changes: BTreeMap::new(),
                cache: BTreeMap::new(),
                permission_stack: BTreeSet::new(),
                steps: 0,
                now: Utc::now().to_rfc3339(),
                sql: vec![],
                invocations: vec![],
                blobs: BTreeMap::new(),
            };
            session.changes.insert(
                (entity.into(), id),
                Change {
                    reference: reference.clone(),
                    action: action.into(),
                    changed: after.keys().cloned().collect(),
                    after: after.clone(),
                },
            );
            for value in after.values() {
                session.references(value)?
            }
            crate::audit::context(
                &self.db,
                &format!("admin.{action}"),
                "admin",
                &serde_json::json!({"type":"admin"}),
            )?;
            session.apply()?;
            let events = if action == "CREATE" && self.program.streams.contains_key(entity) {
                self.run_events(vec![reference])?
            } else {
                (0, 0)
            };
            self.db.execute_batch("COMMIT;")?;
            self.observability.event("stream_events", events.0);
            self.observability.event("automation_runs", events.1);
            Ok(serde_json::json!({"id":id,"action":action,"entity":entity}))
        })();
        if result.is_err() {
            let _ = self.db.execute_batch("ROLLBACK;");
        }
        result
    }
}
