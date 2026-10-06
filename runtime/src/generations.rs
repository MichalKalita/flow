//! Records the active program source with the schema it activated.
//! A failed activation keeps the previous generation.

use crate::{Error, Result};
use rusqlite::{Connection, OptionalExtension};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::path::Path;

pub(crate) fn install(database: &Connection, source: &str, manifest: &Value) -> Result<()> {
    if source.len() > 4 * 1024 * 1024 || manifest.to_string().len() > 65536 {
        return Err(Error::new(
            "limit",
            "Active program metadata exceeds its limit",
        ));
    }
    database.execute_batch("CREATE TABLE IF NOT EXISTS _flow_active_program (id INTEGER PRIMARY KEY CHECK(id=1), source TEXT NOT NULL, manifest TEXT NOT NULL, hash TEXT NOT NULL);")?;
    database.execute("INSERT INTO _flow_active_program(id,source,manifest,hash) VALUES(1,?1,?2,?3) ON CONFLICT(id) DO UPDATE SET source=excluded.source,manifest=excluded.manifest,hash=excluded.hash",rusqlite::params![source,manifest.to_string(),format!("numeric-v1:{:x}",Sha256::digest(source))])?;
    Ok(())
}

pub(crate) fn read(database: &Connection) -> Result<Option<Value>> {
    let exists:bool=database.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE name='_flow_active_program' AND type='table')",[],|row|row.get(0))?;
    if !exists {
        return Ok(None);
    }
    let row:Option<(String,String,String)>=database.query_row("SELECT source,manifest,hash FROM _flow_active_program WHERE id=1 AND length(CAST(source AS BLOB))<=4194304 AND length(CAST(manifest AS BLOB))<=65536",[],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?))).optional()?;
    let Some((source, manifest, hash)) = row else {
        return Err(Error::new(
            "configuration",
            "Active program metadata is missing or oversized",
        ));
    };
    let stored: String =
        database.query_row("SELECT hash FROM _flow_schema WHERE id=1", [], |row| {
            row.get(0)
        })?;
    if hash != stored || hash != format!("numeric-v1:{:x}", Sha256::digest(source.as_bytes())) {
        return Err(Error::new(
            "configuration",
            "Active program metadata does not match the committed schema",
        ));
    }
    Ok(Some(
        json!({"source":source,"manifest":serde_json::from_str::<Value>(&manifest)?}),
    ))
}
pub(crate) fn read_file(path: &Path) -> Result<Option<Value>> {
    if !path.exists() {
        return Ok(None);
    }
    let database = Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    read(&database)
}
