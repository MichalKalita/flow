use crate::{Result, engine::Runtime, program::Program};
use rusqlite::Connection;
use serde_json::{Value, json};

fn q(s: &str) -> String {
    format!("\"{}\"", s.replace('"', "\"\""))
}
fn lit(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

pub(crate) fn install(db: &Connection, program: &Program) -> Result<()> {
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS _flow_audit_context (
            id INTEGER PRIMARY KEY CHECK(id=1), transaction_id TEXT NOT NULL,
            operation TEXT NOT NULL, transport TEXT NOT NULL, actor TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS _flow_audit (
            id INTEGER PRIMARY KEY, time TEXT NOT NULL, transaction_id TEXT NOT NULL,
            operation TEXT NOT NULL, transport TEXT NOT NULL, actor TEXT NOT NULL,
            entity TEXT NOT NULL, entity_id TEXT NOT NULL, action TEXT NOT NULL,
            before_json TEXT, after_json TEXT
        );
        CREATE INDEX IF NOT EXISTS _flow_audit_transaction ON _flow_audit(transaction_id);",
    )?;
    context(db, "seed", "startup", &json!({"type": "system"}))?;
    for (name, entity) in &program.entities {
        let fields = entity
            .fields
            .iter()
            .filter(|(_, f)| f.relation.is_none())
            .map(|(f, _)| f)
            .collect::<Vec<_>>();
        let snapshot = |prefix: &str| {
            // Preserve SQL storage representations, including exact decimal values.
            let values = fields
                .iter()
                .map(|f| format!("{},{}.{}", lit(f), prefix, q(f)))
                .collect::<Vec<_>>()
                .join(",");
            format!("json_object({values})")
        };
        for (event, before, after, prefix) in [
            ("INSERT", "NULL".to_owned(), snapshot("NEW"), "NEW"),
            ("UPDATE", snapshot("OLD"), snapshot("NEW"), "NEW"),
            ("DELETE", snapshot("OLD"), "NULL".to_owned(), "OLD"),
        ] {
            let trigger = q(&format!("_flow_audit_{name}_{event}"));
            let table = q(name);
            let entity = lit(name);
            let action = lit(event);
            db.execute_batch(&format!(
                "CREATE TRIGGER IF NOT EXISTS {trigger} AFTER {event} ON {table} BEGIN
                    INSERT INTO _flow_audit (
                        time, transaction_id, operation, transport, actor, entity,
                        entity_id, action, before_json, after_json
                    )
                    SELECT strftime('%Y-%m-%dT%H:%M:%fZ','now'), transaction_id,
                        operation, transport, actor, {entity}, {prefix}.id,
                        {action}, {before}, {after}
                    FROM _flow_audit_context WHERE id=1;
                END;"
            ))?;
        }
    }
    if program.plugins.contains_key("Files.put") {
        // Binary contents stay out of the audit; record their size, including cascades.
        for (event, prefix) in [("INSERT", "NEW"), ("UPDATE", "NEW"), ("DELETE", "OLD")] {
            let before = if event == "INSERT" {
                "NULL"
            } else {
                "json_object('bytes',length(OLD.bytes))"
            };
            let after = if event == "DELETE" {
                "NULL"
            } else {
                "json_object('bytes',length(NEW.bytes))"
            };
            let trigger = q(&format!("_flow_audit_blobs_{event}"));
            db.execute_batch(&format!(
                "CREATE TRIGGER IF NOT EXISTS {trigger} AFTER {event} ON _flow_blobs BEGIN
                    INSERT INTO _flow_audit (
                        time, transaction_id, operation, transport, actor, entity,
                        entity_id, action, before_json, after_json
                    )
                    SELECT strftime('%Y-%m-%dT%H:%M:%fZ','now'), transaction_id,
                        operation, transport, actor, '_flow_blobs', {prefix}.file_id,
                        '{event}', {before}, {after}
                    FROM _flow_audit_context WHERE id=1;
                END;"
            ))?;
        }
    }
    Ok(())
}

pub(crate) fn context(
    db: &Connection,
    operation: &str,
    transport: &str,
    actor: &Value,
) -> Result<()> {
    db.execute(
        "INSERT INTO _flow_audit_context VALUES(1,?1,?2,?3,?4)
        ON CONFLICT(id) DO UPDATE SET transaction_id=excluded.transaction_id,
            operation=excluded.operation, transport=excluded.transport, actor=excluded.actor",
        rusqlite::params![
            uuid::Uuid::new_v4().to_string(),
            operation,
            transport,
            actor.to_string()
        ],
    )?;
    Ok(())
}

impl Runtime {
    pub fn audit(&self, before: i64, limit: usize) -> Result<Value> {
        let mut query = self.db.prepare(
            "SELECT id, time, transaction_id, operation, transport, actor, entity,
                entity_id, action, before_json, after_json
            FROM _flow_audit WHERE id < ?1 ORDER BY id DESC LIMIT ?2",
        )?;
        let rows = query
            .query_map(
                rusqlite::params![
                    if before <= 0 { i64::MAX } else { before },
                    limit.min(200) as i64
                ],
                |r| {
                    let parse = |i| -> rusqlite::Result<Value> {
                        let s: Option<String> = r.get(i)?;
                        Ok(s.and_then(|s| serde_json::from_str(&s).ok())
                            .unwrap_or(Value::Null))
                    };
                    Ok(json!({
                        "id": r.get::<_, i64>(0)?,
                        "time": r.get::<_, String>(1)?,
                        "transaction_id": r.get::<_, String>(2)?,
                        "operation": r.get::<_, String>(3)?,
                        "transport": r.get::<_, String>(4)?,
                        "actor": parse(5)?,
                        "entity": r.get::<_, String>(6)?,
                        "entity_id": r.get::<_, String>(7)?,
                        "action": r.get::<_, String>(8)?,
                        "before": parse(9)?,
                        "after": parse(10)?
                    }))
                },
            )?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(json!(rows))
    }
}
