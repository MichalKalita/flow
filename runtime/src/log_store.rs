//! Stores server logs outside the application database, in bounded chunks.
//! Supports filtered reads and retention. Does not export logs to remote storage.

use crate::observability::Observability;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{File, Metadata},
    io::{Read, Seek, SeekFrom},
    path::Path,
};
const SCAN_BUDGET: usize = 4 * 1024 * 1024;
const CHUNK: usize = 64 * 1024;

fn file_id(metadata: &Metadata) -> u64 {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        metadata.ino()
    }
    #[cfg(not(unix))]
    {
        metadata
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0)
    }
}
// The cursor names an inode and a byte offset. Rotation can rename an archive
// without invalidating pagination, and a bounded scan resumes rather than
// rescanning the same newest four MiB on every page.
fn reverse_file(
    file: &mut File,
    mut position: u64,
    budget: &mut usize,
    mut visit: impl FnMut(Value, u64) -> bool,
) -> std::io::Result<(bool, u64)> {
    let mut pending = Vec::new();
    while position > 0 && *budget > 0 {
        let length = (position as usize).min(CHUNK).min(*budget);
        position -= length as u64;
        file.seek(SeekFrom::Start(position))?;
        let mut bytes = vec![0; length];
        file.read_exact(&mut bytes)?;
        *budget -= length;
        bytes.extend_from_slice(&pending);
        while let Some(index) = bytes.iter().rposition(|b| *b == b'\n') {
            if let Ok(value) = serde_json::from_slice::<Value>(&bytes[index + 1..])
                && visit(value, position + index as u64 + 1)
            {
                return Ok((true, position + index as u64 + 1));
            }
            bytes.truncate(index);
        }
        pending = if bytes.len() <= 4 * CHUNK {
            bytes
        } else {
            Vec::new()
        };
    }
    if position == 0
        && let Ok(value) = serde_json::from_slice::<Value>(&pending)
        && visit(value, 0)
    {
        return Ok((true, 0));
    }
    Ok((
        false,
        if position == 0 {
            0
        } else {
            position + pending.len() as u64
        },
    ))
}

pub fn query(
    observer: &Observability,
    filters: &BTreeMap<String, String>,
) -> std::io::Result<Value> {
    let scope = observer.log_scope();
    let histogram_mode = filters
        .get("histogram")
        .is_some_and(|value| value == "true");
    let now = chrono::Utc::now().timestamp();
    let histogram_start = filters
        .get("since")
        .and_then(|value| value.parse::<i64>().ok())
        .unwrap_or(now - 3600)
        .clamp(now - 21600, now);
    let mut histogram = BTreeMap::<i64, u64>::new();
    let cursor = filters.get("before").map(String::as_str).unwrap_or("");
    let mut parts = cursor.split('/');
    let before = parts
        .next()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(u64::MAX);
    let resume = parts
        .next()
        .and_then(|s| s.parse::<u64>().ok())
        .zip(parts.next().and_then(|s| s.parse::<u64>().ok()));
    let limit = filters
        .get("limit")
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(100)
        .clamp(1, 200);
    let search = filters
        .get("search")
        .map(|s| s.chars().take(256).collect::<String>().to_lowercase())
        .unwrap_or_default();
    let since = filters.get("since").and_then(|s| s.parse::<i64>().ok());
    let matches = |event: &Value| {
        if scope
            .as_ref()
            .is_some_and(|project| event["project"].as_str() != Some(project.as_str()))
        {
            return false;
        }
        for key in ["kind", "level", "endpoint", "server"] {
            if let Some(value) = filters.get(key).filter(|v| !v.is_empty())
                && event[key].as_str() != Some(value)
            {
                return false;
            }
        }
        if let Some(status) = filters.get("status").filter(|v| !v.is_empty()) {
            let code = event["status"].as_u64().unwrap_or(0);
            let matches = match status.as_str() {
                "errors" => code >= 400,
                "2xx" => (200..300).contains(&code),
                "4xx" => (400..500).contains(&code),
                "5xx" => code >= 500,
                _ => status.parse::<u64>().ok() == Some(code),
            };
            if !matches {
                return false;
            }
        }
        if let Some(since) = since
            && event["time"]
                .as_str()
                .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                .is_none_or(|t| t.timestamp() < since)
        {
            return false;
        }
        search.is_empty() || event.to_string().to_lowercase().contains(&search)
    };
    let (memory, directory) = observer.log_source();
    let mut rows = Vec::new();
    let mut seen = BTreeSet::new();
    let mut next = None;
    let mut visit = |mut event: Value, location: Option<(u64, u64)>| {
        let sequence = event["sequence"]
            .as_u64()
            .or_else(|| {
                event["time"]
                    .as_str()
                    .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                    .map(|t| t.timestamp_micros().max(0) as u64)
            })
            .unwrap_or(0);
        if sequence >= before || !seen.insert(sequence) {
            return false;
        }
        event["sequence"] = json!(sequence);
        if scope.is_some() && event["local_endpoint"].is_string() {
            event["endpoint"] = event["local_endpoint"].clone();
        }
        if matches(&event) {
            if histogram_mode {
                if let Some(time) = event["time"]
                    .as_str()
                    .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
                    .map(|time| time.timestamp())
                    && (histogram_start..=now).contains(&time)
                {
                    *histogram.entry(time / 60).or_default() += 1;
                }
            } else {
                rows.push(event);
            }
        }
        next = Some(
            location
                .map(|(id, offset)| format!("{sequence}/{id}/{offset}"))
                .unwrap_or_else(|| sequence.to_string()),
        );
        rows.len() >= limit
    };
    let mut full = false;
    if resume.is_none() {
        for event in memory {
            if visit(event, None) {
                full = true;
                break;
            }
        }
    }
    let mut budget = SCAN_BUDGET;
    let mut found = resume.is_none();
    let mut continuation = None;
    if !full && let Some(directory) = directory {
        let directories = std::iter::once((directory, None)).chain(
            observer
                .legacy_directory()
                .map(|path| (path, scope.clone())),
        );
        for (directory, legacy_project) in directories {
            if full || budget == 0 {
                break;
            }
            for path in crate::observability::log_files(&directory)? {
                let mut file = match File::open(Path::new(&path)) {
                    Ok(f) => f,
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                    Err(e) => return Err(e),
                };
                let metadata = file.metadata()?;
                let id = file_id(&metadata);
                if !found && resume.is_some_and(|(wanted, _)| wanted != id) {
                    continue;
                }
                let position = if !found {
                    found = true;
                    resume.unwrap().1.min(metadata.len())
                } else {
                    metadata.len()
                };
                let (stopped, position) =
                    reverse_file(&mut file, position, &mut budget, |mut event, offset| {
                        if let Some(project) = &legacy_project {
                            event["project"] = json!(project);
                        }
                        visit(event, Some((id, offset)))
                    })?;
                if stopped {
                    full = true;
                    break;
                }
                if budget == 0 {
                    continuation = Some(format!("{before}/{id}/{position}"));
                    break;
                }
            }
        }
    }
    let scan_limited = budget == 0 && !full;
    let next = if scan_limited {
        continuation
    } else if full {
        next
    } else {
        None
    };
    Ok(
        json!({"entries":rows,"next_cursor":next,"scan_limited":scan_limited,"cursor_expired":!found,"scanned_bytes":SCAN_BUDGET-budget,"source":"memory and rotating JSONL files","histogram":histogram.into_iter().map(|(minute,count)|json!({"minute":minute,"count":count})).collect::<Vec<_>>(),"histogram_since":histogram_start,"histogram_until":now,"partial":scan_limited || !found}),
    )
}
