use crate::engine::Runtime;
use serde_json::{Value, json};
use std::{fs, path::Path};

pub(crate) fn heap_bytes(value: &Value) -> usize {
    match value {
        Value::String(s) => s.capacity(),
        Value::Array(items) => {
            items.capacity() * std::mem::size_of::<Value>()
                + items.iter().map(heap_bytes).sum::<usize>()
        }
        // BTreeMap node overhead is an estimate; allocator bookkeeping and fragmentation are excluded.
        Value::Object(fields) => fields
            .iter()
            .map(|(key, value)| {
                key.capacity()
                    + std::mem::size_of::<String>()
                    + std::mem::size_of::<Value>()
                    + 3 * std::mem::size_of::<usize>()
                    + heap_bytes(value)
            })
            .sum(),
        _ => 0,
    }
}
pub(crate) fn file_bytes(path: impl AsRef<Path>) -> Option<u64> {
    match fs::metadata(path) {
        Ok(m) => Some(m.len()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Some(0),
        Err(_) => None,
    }
}
pub fn process_memory() -> Value {
    #[cfg(target_os = "linux")]
    {
        let status = fs::read_to_string("/proc/self/status").unwrap_or_default();
        let read = |field: &str| {
            status.lines().find_map(|line| {
                line.strip_prefix(field)
                    .and_then(|value| value.split_whitespace().next())
                    .and_then(|v| v.parse::<u64>().ok())
                    .map(|n| n * 1024)
            })
        };
        json!({
            "rss_bytes": read("VmRSS:"),
            "peak_rss_bytes": read("VmHWM:"),
            "virtual_bytes": read("VmSize:"),
            "source": "/proc/self/status"
        })
    }
    #[cfg(target_os = "macos")]
    {
        let rss = std::process::Command::new("/bin/ps")
            .args(["-o", "rss=", "-p", &std::process::id().to_string()])
            .output()
            .ok()
            .filter(|o| o.status.success())
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .and_then(|s| s.trim().parse::<u64>().ok())
            .map(|n| n * 1024);
        json!({
            "rss_bytes": rss,
            "peak_rss_bytes": null,
            "virtual_bytes": null,
            "source": "ps rss (macOS)"
        })
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        json!({
            "rss_bytes": null,
            "peak_rss_bytes": null,
            "virtual_bytes": null,
            "source": "unavailable on this platform"
        })
    }
}
impl Runtime {
    pub fn storage_resources(&self) -> Value {
        let read = |pragma| {
            self.db
                .query_row(pragma, [], |r| r.get::<_, i64>(0))
                .ok()
                .and_then(|n| u64::try_from(n).ok())
        };
        let logical = read("PRAGMA page_count")
            .zip(read("PRAGMA page_size"))
            .map(|(a, b)| a * b);
        let path = self.db.path().filter(|p| !p.is_empty());
        let main = path.and_then(file_bytes);
        let wal = path.and_then(|p| file_bytes(format!("{p}-wal")));
        let shm = path.and_then(|p| file_bytes(format!("{p}-shm")));
        let total = main.zip(wal).zip(shm).map(|((a, b), c)| a + b + c);
        let status = |op| {
            let mut current = 0;
            let mut peak = 0;
            // SAFETY: the connection stays alive, access is serialized by Runtime's
            // owner, and SQLite only writes to the two valid local output pointers.
            let result = unsafe {
                rusqlite::ffi::sqlite3_db_status(self.db.handle(), op, &mut current, &mut peak, 0)
            };
            if result == rusqlite::ffi::SQLITE_OK {
                Some(current)
            } else {
                None
            }
        };
        json!({
            "sqlite": {
                "main_bytes": main,
                "wal_bytes": wal,
                "shm_bytes": shm,
                "total_disk_bytes": total,
                "logical_bytes": logical,
                "in_memory": path.is_none(),
                "page_cache_bytes": status(rusqlite::ffi::SQLITE_DBSTATUS_CACHE_USED),
                "schema_bytes": status(rusqlite::ffi::SQLITE_DBSTATUS_SCHEMA_USED),
                "prepared_statements_bytes": status(rusqlite::ffi::SQLITE_DBSTATUS_STMT_USED)
            }
        })
    }
}
