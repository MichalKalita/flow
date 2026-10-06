//! Applies versioned field renames before a new program generation becomes active.
//! A failed step restores the previous generation and does not replay a finished step.

use crate::{Error, Result, program::Program, syntax::Node};
use rusqlite::{Connection, OptionalExtension};
use sha2::{Digest, Sha256};

#[derive(Clone, Debug)]
pub struct Rename {
    pub entity: String,
    pub from: String,
    pub to: String,
}
#[derive(Clone, Debug)]
pub struct Migration {
    pub id: String,
    pub from: u32,
    pub to: u32,
    pub renames: Vec<Rename>,
    pub checksum: String,
}
fn invalid(message: &str) -> Error {
    Error::new("invalid_program", message)
}
pub(crate) fn version(node: &Node) -> Result<u32> {
    node.text()?
        .parse::<u32>()
        .ok()
        .filter(|version| (1..=1000000).contains(version))
        .ok_or_else(|| invalid("Schema versions must be integers between 1 and 1000000"))
}
fn identifier(node: &Node) -> Result<String> {
    let value = node.text()?;
    if value.is_empty()
        || value.len() > 64
        || !value.bytes().enumerate().all(|(index, byte)| {
            byte.is_ascii_alphabetic() || byte == b'_' || index > 0 && byte.is_ascii_digit()
        })
    {
        return Err(invalid("Invalid migration identifier"));
    }
    Ok(value.to_owned())
}
pub(crate) fn parse(node: &Node) -> Result<Migration> {
    let id = identifier(node.arg(0)?)?;
    let mut from = None;
    let mut to = None;
    let mut renames = Vec::new();
    for option in node.args()?.iter().skip(1) {
        match option.head() {
            "from" if from.is_none() && option.args()?.len() == 1 => {
                from = Some(version(option.arg(0)?)?)
            }
            "to" if to.is_none() && option.args()?.len() == 1 => {
                to = Some(version(option.arg(0)?)?)
            }
            "rename" if option.args()?.len() == 3 && renames.len() < 128 => {
                let rename = Rename {
                    entity: identifier(option.arg(0)?)?,
                    from: identifier(option.arg(1)?)?,
                    to: identifier(option.arg(2)?)?,
                };
                if rename.from == "id" || rename.to == "id" || rename.from == rename.to {
                    return Err(invalid(
                        "Migration cannot rename IDs or rename a field to itself",
                    ));
                }
                renames.push(rename);
            }
            _ => return Err(invalid("Unknown, duplicate, or oversized migration option")),
        }
    }
    let from = from.ok_or_else(|| invalid("Migration requires a source version"))?;
    let to = to.ok_or_else(|| invalid("Migration requires a target version"))?;
    if to != from + 1 {
        return Err(invalid("Migration must advance one schema version"));
    }
    Ok(Migration {
        id,
        from,
        to,
        renames,
        checksum: format!("{:x}", Sha256::digest(format!("{node:?}"))),
    })
}
pub(crate) fn chain(program: &Program, from: u32) -> Result<Vec<&Migration>> {
    if from > program.schema_version {
        return Err(Error::new(
            "configuration",
            "Schema downgrade requires an explicit recovery procedure",
        ));
    }
    let mut chain = Vec::new();
    let mut version = from;
    while version < program.schema_version {
        if chain.len() >= 256 {
            return Err(Error::new("limit", "Migration chain exceeds its bound"));
        }
        let migration = program
            .migrations
            .iter()
            .find(|migration| migration.from == version)
            .ok_or_else(|| {
                Error::new(
                    "configuration",
                    "Migration chain does not reach the candidate schema",
                )
            })?;
        chain.push(migration);
        version = migration.to;
    }
    Ok(chain)
}
pub(crate) fn renamed(chain: &[&Migration], entity: &str, field: &str) -> String {
    let mut field = field.to_owned();
    for migration in chain {
        for rename in &migration.renames {
            if rename.entity == entity && rename.from == field {
                field = rename.to.clone();
            }
        }
    }
    field
}
fn q(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\"\""))
}
pub(crate) fn apply(database: &Connection, program: &Program, initialized: bool) -> Result<bool> {
    database.execute_batch("CREATE TABLE IF NOT EXISTS _flow_schema_version (id INTEGER PRIMARY KEY CHECK(id=1), version INTEGER NOT NULL CHECK(version>0)); CREATE TABLE IF NOT EXISTS _flow_migrations (target_version INTEGER PRIMARY KEY, migration_id TEXT NOT NULL UNIQUE, checksum TEXT NOT NULL, applied INTEGER NOT NULL, time TEXT NOT NULL);")?;
    let stored: Option<u32> = database
        .query_row(
            "SELECT version FROM _flow_schema_version WHERE id=1",
            [],
            |row| row.get(0),
        )
        .optional()?;
    let current = stored.unwrap_or(if initialized {
        1
    } else {
        program.schema_version
    });
    let mut history = database.prepare("SELECT migration_id,checksum FROM _flow_migrations")?;
    for entry in history.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })? {
        let (id, checksum) = entry?;
        if !program
            .migrations
            .iter()
            .any(|migration| migration.id == id && migration.checksum == checksum)
        {
            return Err(Error::new(
                "configuration",
                "Recorded migration definitions cannot be removed or changed",
            ));
        }
    }
    drop(history);
    let migrations = chain(program, current)?;
    for migration in &migrations {
        for rename in &migration.renames {
            database.execute_batch(&format!(
                "ALTER TABLE {} RENAME COLUMN {} TO {};",
                q(&rename.entity),
                q(&rename.from),
                q(&rename.to)
            ))?;
        }
        database.execute("INSERT INTO _flow_migrations(target_version,migration_id,checksum,applied,time) VALUES(?1,?2,?3,1,?4)",rusqlite::params![migration.to,migration.id,migration.checksum,chrono::Utc::now().to_rfc3339()])?;
    }
    if !initialized {
        for migration in &program.migrations {
            database.execute("INSERT INTO _flow_migrations(target_version,migration_id,checksum,applied,time) VALUES(?1,?2,?3,0,?4)",rusqlite::params![migration.to,migration.id,migration.checksum,chrono::Utc::now().to_rfc3339()])?;
        }
    }
    database.execute("INSERT INTO _flow_schema_version(id,version) VALUES(1,?1) ON CONFLICT(id) DO UPDATE SET version=excluded.version",[program.schema_version])?;
    Ok(!migrations.is_empty())
}

pub(crate) struct PauseLimit<'a> {
    database: &'a Connection,
    deadline: Box<std::time::Instant>,
}
impl<'a> PauseLimit<'a> {
    pub(crate) fn new(database: &'a Connection) -> Self {
        let mut limit = Self {
            database,
            deadline: Box::new(std::time::Instant::now() + std::time::Duration::from_secs(5)),
        };
        unsafe extern "C" fn expired(pointer: *mut std::ffi::c_void) -> std::ffi::c_int {
            // SAFETY: the pointer belongs to the live PauseLimit registered on
            // this serialized connection and is removed before the box is freed.
            let deadline = unsafe { &*(pointer.cast::<std::time::Instant>()) };
            i32::from(std::time::Instant::now() >= *deadline)
        }
        // SAFETY: the boxed deadline has a stable address. SQLite calls this
        // function synchronously and Drop unregisters it before releasing memory.
        unsafe {
            rusqlite::ffi::sqlite3_progress_handler(
                database.handle(),
                1000,
                Some(expired),
                (&mut *limit.deadline as *mut std::time::Instant).cast(),
            );
        }
        limit
    }
    pub(crate) fn check(&self) -> Result<()> {
        if std::time::Instant::now() >= *self.deadline {
            Err(Error::new(
                "limit",
                "Schema activation exceeded its five-second pause budget",
            ))
        } else {
            Ok(())
        }
    }
}
impl Drop for PauseLimit<'_> {
    fn drop(&mut self) {
        // SAFETY: the connection outlives the guard. Unregistration happens
        // before rollback, so the recovery transaction cannot inherit this limit.
        unsafe {
            rusqlite::ffi::sqlite3_progress_handler(
                self.database.handle(),
                0,
                None,
                std::ptr::null_mut(),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn interrupted_sqlite_work_removes_progress_callback_before_rollback() {
        let database = Connection::open_in_memory().unwrap();
        database.execute_batch("CREATE TABLE Item(id INTEGER PRIMARY KEY,title TEXT); INSERT INTO Item VALUES(1,'saved'); BEGIN IMMEDIATE; ALTER TABLE Item RENAME COLUMN title TO name;").unwrap();
        let mut limit = PauseLimit::new(&database);
        *limit.deadline = std::time::Instant::now() + std::time::Duration::from_millis(10);
        assert!(database.query_row("WITH RECURSIVE numbers(n) AS (VALUES(0) UNION ALL SELECT n+1 FROM numbers WHERE n<1000000000) SELECT sum(n) FROM numbers",[],|row|row.get::<_,i64>(0)).is_err());
        assert!(limit.check().is_err());
        drop(limit);
        database.execute_batch("ROLLBACK").unwrap();
        assert_eq!(
            database
                .query_row("SELECT title FROM Item WHERE id=1", [], |row| row
                    .get::<_, String>(0))
                .unwrap(),
            "saved"
        );
    }
}
