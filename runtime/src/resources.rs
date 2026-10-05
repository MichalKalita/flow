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
        static TICKS: std::sync::OnceLock<Option<f64>> = std::sync::OnceLock::new();
        let ticks = TICKS.get_or_init(|| {
            std::process::Command::new("getconf")
                .arg("CLK_TCK")
                .output()
                .ok()
                .filter(|o| o.status.success())
                .and_then(|o| String::from_utf8(o.stdout).ok())
                .and_then(|s| s.trim().parse::<f64>().ok())
                .filter(|n| *n > 0.)
        });
        let cpu = fs::read_to_string("/proc/self/stat").ok().and_then(|s| {
            let (_, fields) = s.rsplit_once(')')?;
            let fields = fields.split_whitespace().collect::<Vec<_>>();
            let total =
                fields.get(11)?.parse::<f64>().ok()? + fields.get(12)?.parse::<f64>().ok()?;
            Some(total / (*ticks)?)
        });
        json!({
            "cpu_seconds":cpu,
            "rss_bytes": read("VmRSS:"),
            "peak_rss_bytes": read("VmHWM:"),
            "virtual_bytes": read("VmSize:"),
            "source": "/proc/self/status"
        })
    }
    #[cfg(target_os = "macos")]
    {
        let output = std::process::Command::new("/bin/ps")
            .args(["-o", "rss=,time=", "-p", &std::process::id().to_string()])
            .output()
            .ok()
            .filter(|o| o.status.success())
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .unwrap_or_default();
        let mut fields = output.split_whitespace();
        let rss = fields
            .next()
            .and_then(|s| s.parse::<u64>().ok())
            .map(|n| n * 1024);
        let cpu = fields.next().and_then(|s| {
            s.split(':')
                .try_fold(0., |n, v| Some(n * 60. + v.parse::<f64>().ok()?))
        });
        json!({
            "cpu_seconds": cpu,
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
fn finite_bytes(n: u64) -> Option<u64> {
    (n > 0 && n < (1u64 << 60)).then_some(n)
}
#[cfg(target_os = "linux")]
fn meminfo_bytes(status: &str, field: &str) -> Option<u64> {
    status.lines().find_map(|line| {
        line.strip_prefix(field)
            .and_then(|value| value.split_whitespace().next())
            .and_then(|v| v.parse::<u64>().ok())
            .map(|n| n * 1024)
    })
}
#[cfg(target_os = "linux")]
fn cgroup_memory_limit() -> Option<u64> {
    let cgroup = fs::read_to_string("/proc/self/cgroup").ok()?;
    if let Some(path) = cgroup.lines().find_map(|line| line.strip_prefix("0::")) {
        let mut dir = std::path::PathBuf::from("/sys/fs/cgroup");
        let relative = path.trim().trim_start_matches('/');
        if !relative.is_empty() {
            dir.push(relative);
        }
        loop {
            if let Ok(value) = fs::read_to_string(dir.join("memory.max")) {
                let value = value.trim();
                if value != "max"
                    && let Ok(n) = value.parse::<u64>()
                    && let Some(n) = finite_bytes(n)
                {
                    return Some(n);
                }
            }
            if !dir.pop() || dir.as_os_str() == "/sys/fs" {
                break;
            }
        }
    }
    for line in cgroup.lines() {
        let mut parts = line.splitn(3, ':');
        let Some(_) = parts.next() else { continue };
        let Some(controllers) = parts.next() else {
            continue;
        };
        let Some(path) = parts.next() else { continue };
        if !controllers.split(',').any(|c| c == "memory") {
            continue;
        }
        let mut dir = std::path::PathBuf::from("/sys/fs/cgroup/memory");
        let relative = path.trim().trim_start_matches('/');
        if !relative.is_empty() {
            dir.push(relative);
        }
        loop {
            if let Ok(value) = fs::read_to_string(dir.join("memory.limit_in_bytes"))
                && let Ok(n) = value.trim().parse::<u64>()
                && let Some(n) = finite_bytes(n)
            {
                return Some(n);
            }
            if !dir.pop() || dir.as_os_str() == "/sys/fs/cgroup" {
                break;
            }
        }
    }
    None
}
pub fn host_memory() -> Value {
    #[cfg(target_os = "linux")]
    {
        let meminfo = fs::read_to_string("/proc/meminfo").unwrap_or_default();
        let physical = meminfo_bytes(&meminfo, "MemTotal:");
        let available = meminfo_bytes(&meminfo, "MemAvailable:");
        let cgroup = cgroup_memory_limit();
        let (limit, source) = match (cgroup, physical) {
            (Some(cgroup), Some(physical)) if cgroup < physical => {
                (Some(cgroup), "cgroup memory limit")
            }
            (Some(cgroup), None) => (Some(cgroup), "cgroup memory limit"),
            (_, Some(_)) => (physical, "/proc/meminfo MemTotal"),
            _ => (None, "unavailable"),
        };
        json!({
            "limit_bytes": limit,
            "physical_bytes": physical,
            "available_bytes": available,
            "source": source
        })
    }
    #[cfg(target_os = "macos")]
    {
        let physical = std::process::Command::new("/usr/sbin/sysctl")
            .args(["-n", "hw.memsize"])
            .output()
            .ok()
            .filter(|o| o.status.success())
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .and_then(|s| s.trim().parse::<u64>().ok())
            .and_then(finite_bytes);
        json!({
            "limit_bytes": physical,
            "physical_bytes": physical,
            "available_bytes": null,
            "source": "sysctl hw.memsize"
        })
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        json!({
            "limit_bytes": null,
            "physical_bytes": null,
            "available_bytes": null,
            "source": "unavailable on this platform"
        })
    }
}

const MIN_HTTP_ADMISSION: usize = 16;
const BASELINE_RAM_BYTES: u64 = 2 * 1024 * 1024 * 1024;

pub fn available_parallelism() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
        .max(1)
}

fn env_usize(name: &str) -> Option<usize> {
    std::env::var(name)
        .ok()?
        .parse::<usize>()
        .ok()
        .filter(|n| *n >= 1)
}

pub fn tokio_worker_threads() -> usize {
    env_usize("FLOW_TOKIO_WORKERS").unwrap_or_else(available_parallelism)
}

pub fn tokio_blocking_threads() -> usize {
    env_usize("FLOW_TOKIO_BLOCKING").unwrap_or_else(|| tokio_worker_threads().max(2))
}

pub fn host_memory_limit_bytes() -> Option<u64> {
    host_memory()
        .get("limit_bytes")
        .and_then(serde_json::Value::as_u64)
}

pub fn http_admission() -> usize {
    env_usize("FLOW_HTTP_ADMISSION")
        .unwrap_or_else(|| http_admission_from(available_parallelism(), host_memory_limit_bytes()))
}

pub fn http_admission_from(cores: usize, ram_bytes: Option<u64>) -> usize {
    let cores = cores.max(1);
    let by_cpu = cores.saturating_mul(16);
    let by_ram = match ram_bytes {
        Some(ram) if ram > 0 => {
            let chunks = ram.div_ceil(BASELINE_RAM_BYTES).max(1);
            usize::try_from(chunks)
                .unwrap_or(usize::MAX)
                .saturating_mul(16)
        }
        _ => by_cpu,
    };
    by_cpu.min(by_ram).max(MIN_HTTP_ADMISSION)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn admission_stays_at_sixteen_on_the_minimum_host() {
        assert_eq!(
            http_admission_from(1, Some(BASELINE_RAM_BYTES)),
            MIN_HTTP_ADMISSION
        );
    }

    #[test]
    fn admission_does_not_exceed_the_two_gib_budget_on_small_ram() {
        assert_eq!(
            http_admission_from(8, Some(BASELINE_RAM_BYTES)),
            MIN_HTTP_ADMISSION
        );
    }

    #[test]
    fn admission_scales_with_cpu_and_ram() {
        assert_eq!(http_admission_from(8, Some(8 * BASELINE_RAM_BYTES)), 128);
        assert_eq!(
            http_admission_from(1, Some(8 * BASELINE_RAM_BYTES)),
            MIN_HTTP_ADMISSION
        );
        assert_eq!(http_admission_from(4, None), 64);
    }
}
