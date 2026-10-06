//! In-process spans, counters, and log events for one server.
//! Keeps history bounded. Does not write credentials, tokens, or request bodies.

use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, VecDeque},
    fs::{self, OpenOptions},
    io::{BufRead, BufReader, BufWriter, Read, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, mpsc},
    time::{Duration, Instant},
};

const BOUNDS: [f64; 15] = [
    1., 2., 5., 10., 20., 50., 100., 200., 500., 1000., 2000., 5000., 10000., 30000., 60000.,
];
const HISTORY: usize = 360;
pub const LOG_QUEUE_LIMIT: usize = 512;
pub const LOG_BUFFER_LIMIT: usize = 200;
pub const LOG_ROTATE_BYTES: u64 = 256 * 1024 * 1024;
pub const LOG_ARCHIVE_COUNT: u32 = 3;
pub const LOG_DISK_LIMIT_BYTES: u64 = LOG_ROTATE_BYTES * (LOG_ARCHIVE_COUNT as u64 + 1);
pub const LOG_CHUNK_BYTES: u64 = 50 * 1024 * 1024;
pub const LOG_QUEUE_BYTES: usize = 8 * 1024 * 1024;
#[derive(Clone, Copy)]
pub struct LogPolicy {
    pub target_bytes: u64,
    pub chunk_bytes: u64,
}
impl Default for LogPolicy {
    fn default() -> Self {
        Self {
            target_bytes: LOG_DISK_LIMIT_BYTES,
            chunk_bytes: LOG_CHUNK_BYTES,
        }
    }
}
pub fn log_files(path: &Path) -> std::io::Result<Vec<PathBuf>> {
    let mut segments = Vec::new();
    match fs::read_dir(path) {
        Ok(entries) => {
            for entry in entries {
                let entry = entry?;
                let name = entry.file_name().to_string_lossy().into_owned();
                if name.starts_with("application.segment.")
                    && name.ends_with(".jsonl")
                    && entry.file_type()?.is_file()
                {
                    segments.push(entry.path());
                }
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error),
    }
    segments.sort_by(|a, b| b.cmp(a));
    let mut files = vec![path.join("application.jsonl")];
    files.extend(segments);
    files.extend((1..=LOG_ARCHIVE_COUNT).map(|n| path.join(format!("application.{n}.jsonl"))));
    Ok(files)
}
fn cleanup(path: &Path, policy: LogPolicy) -> std::io::Result<()> {
    let files = log_files(path)?;
    let mut total = 0u64;
    let mut closed = Vec::new();
    for file in files {
        match fs::metadata(&file) {
            Ok(metadata) => {
                total = total.saturating_add(metadata.len());
                if file.file_name() != Some(std::ffi::OsStr::new("application.jsonl")) {
                    closed.push((file, metadata.len()));
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
    }
    for (file, bytes) in closed.into_iter().rev() {
        if total <= policy.target_bytes {
            break;
        }
        fs::remove_file(file)?;
        total = total.saturating_sub(bytes);
    }
    Ok(())
}
fn oldest(path: &Path) -> Option<String> {
    for file in log_files(path).ok()?.into_iter().rev() {
        let Ok(file) = fs::File::open(file) else {
            continue;
        };
        let mut line = String::new();
        if BufReader::new(file.take(65536))
            .read_line(&mut line)
            .is_ok()
            && let Ok(event) = serde_json::from_str::<Value>(&line)
            && let Some(time) = event["time"].as_str()
        {
            return Some(time.to_owned());
        }
    }
    None
}
#[derive(Clone, Default)]
pub struct Observability(Arc<Mutex<Data>>);
enum Command {
    Log(Value),
    Flush(mpsc::Sender<std::io::Result<()>>),
}
#[derive(Default)]
struct Data {
    metrics: BTreeMap<String, Metric>,
    logs: VecDeque<Value>,
    writer: Option<mpsc::SyncSender<Command>>,
    writer_thread: Option<std::thread::JoinHandle<()>>,
    dropped: u64,
    storage_errors: u64,
    queued_bytes: usize,
    queued_events: usize,
    directory: Option<PathBuf>,
    sink: Option<(Observability, String)>,
    shared_logs: bool,
    identity: Option<(String, String)>,
    policy: LogPolicy,
    service: crate::telemetry::Service,
    work: BTreeMap<String, Metric>,
    log_sequence: u64,
}
#[derive(Default)]
struct Metric {
    count: u64,
    errors: u64,
    sum: f64,
    buckets: [u64; 16],
    history: VecDeque<Value>,
    minute: i64,
    minute_count: u64,
    minute_errors: u64,
    minute_sum: f64,
    minute_buckets: [u64; 16],
    statuses: [u64; 6],
}
fn percentile(buckets: &[u64; 16], count: u64, p: f64) -> Value {
    let rank = (count as f64 * p).ceil() as u64;
    if rank == 0 {
        return Value::Null;
    }
    let mut sum = 0;
    for (i, n) in buckets.iter().enumerate() {
        sum += n;
        if sum >= rank {
            return BOUNDS.get(i).map(|v| json!(v)).unwrap_or(json!(">60000"));
        }
    }
    Value::Null
}
impl Metric {
    fn minute_point(&self) -> Value {
        json!({
            "buckets": self.minute_buckets,
            "minute": self.minute, "count": self.minute_count, "errors": self.minute_errors,
            "mean_ms": if self.minute_count==0 {0.} else {self.minute_sum/self.minute_count as f64},
            "p50_ms": percentile(&self.minute_buckets, self.minute_count, 0.5),
            "p95_ms": percentile(&self.minute_buckets, self.minute_count, 0.95),
            "p99_ms": percentile(&self.minute_buckets, self.minute_count, 0.99)
        })
    }
    fn observe(&mut self, ms: f64, failed: bool, status: u16) {
        let minute = chrono::Utc::now().timestamp() / 60;
        if self.minute != minute && self.minute_count > 0 {
            if self.history.len() >= HISTORY - 1 {
                self.history.pop_front();
            }
            self.history.push_back(self.minute_point());
            self.minute_count = 0;
            self.minute_errors = 0;
            self.minute_sum = 0.;
            self.minute_buckets = [0; 16];
        }
        self.minute = minute;
        self.count += 1;
        self.minute_count += 1;
        self.sum += ms;
        self.minute_sum += ms;
        if failed {
            self.errors += 1;
            self.minute_errors += 1;
        }
        let bucket = BOUNDS.iter().position(|b| ms <= *b).unwrap_or(15);
        self.buckets[bucket] += 1;
        self.minute_buckets[bucket] += 1;
        self.statuses[(status / 100).min(5) as usize] += 1;
    }
    fn json(&self) -> Value {
        let cutoff = chrono::Utc::now().timestamp() / 60 - HISTORY as i64;
        let mut history = self
            .history
            .iter()
            .filter(|v| v["minute"].as_i64().unwrap_or(0) > cutoff)
            .cloned()
            .collect::<VecDeque<_>>();
        if self.minute_count > 0 && self.minute > cutoff {
            history.push_back(self.minute_point());
        }
        json!({
            "count": self.count,
            "errors": self.errors,
            "sum_ms": self.sum,
            "mean_ms": if self.count == 0 { 0. } else { self.sum / self.count as f64 },
            "p50_ms": percentile(&self.buckets, self.count, 0.5),
            "p95_ms": percentile(&self.buckets, self.count, 0.95),
            "p99_ms": percentile(&self.buckets, self.count, 0.99),
            "buckets": self.buckets,
            "statuses": self.statuses,
            "history": history
        })
    }
}
impl Observability {
    pub fn policy(&self, policy: LogPolicy) -> std::io::Result<()> {
        if policy.chunk_bytes == 0
            || policy.chunk_bytes > policy.target_bytes
            || policy.target_bytes / policy.chunk_bytes > 512
        {
            return Err(std::io::Error::other("Invalid log storage limits"));
        }
        self.0.lock().unwrap().policy = policy;
        Ok(())
    }
    pub fn close(&self) {
        let (writer, worker) = {
            let mut data = self.0.lock().unwrap();
            data.directory = None;
            (data.writer.take(), data.writer_thread.take())
        };
        drop(writer);
        if let Some(worker) = worker {
            let _ = worker.join();
        }
    }

    pub fn scoped(
        path: impl Into<PathBuf>,
        sink: Observability,
        project: &str,
    ) -> std::io::Result<Self> {
        sink.0.lock().unwrap().shared_logs = true;
        Self::disk_inner(path.into(), Some((sink, project.to_owned())))
    }
    pub fn shared_logs(&self) -> bool {
        self.0.lock().unwrap().shared_logs
    }
    pub fn server_id(&self) -> Option<String> {
        let data = self.0.lock().unwrap();
        if let Some((sink, _)) = &data.sink {
            let sink = sink.clone();
            drop(data);
            return sink.server_id();
        }
        data.identity.as_ref().map(|(instance, _)| instance.clone())
    }
    pub fn identity(&self, instance: &str, name: &str) {
        self.0.lock().unwrap().identity = Some((instance.to_owned(), name.to_owned()));
    }
    pub fn log_scope(&self) -> Option<String> {
        self.0
            .lock()
            .unwrap()
            .sink
            .as_ref()
            .map(|(_, project)| project.clone())
    }
    pub fn legacy_directory(&self) -> Option<PathBuf> {
        let data = self.0.lock().unwrap();
        data.sink.as_ref().and(data.directory.clone())
    }
    pub fn disk(path: impl Into<PathBuf>) -> std::io::Result<Self> {
        Self::disk_inner(path.into(), None)
    }
    fn disk_inner(path: PathBuf, sink: Option<(Observability, String)>) -> std::io::Result<Self> {
        fs::create_dir_all(&path)?;
        let log = if sink.is_none() {
            Some(
                OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(path.join("application.jsonl"))?,
            )
        } else {
            None
        };
        let this = Self::default();
        // Restore bounded aggregates. Malformed snapshots fail startup rather than silently resetting counters.
        match fs::read(path.join("metrics.json")) {
            Ok(bytes) => {
                let v: Value = serde_json::from_slice(&bytes)?;
                this.0.lock().unwrap().service.restore(&v["service"]);
                this.0.lock().unwrap().log_sequence = v["log_sequence"].as_u64().unwrap_or(0);
                for section in ["endpoints", "work"] {
                    if let Some(metrics) = v[section].as_object() {
                        for (key, v) in metrics {
                            let mut m = Metric {
                                count: v["count"].as_u64().unwrap_or(0),
                                errors: v["errors"].as_u64().unwrap_or(0),
                                sum: v["sum_ms"].as_f64().unwrap_or(0.),
                                ..Default::default()
                            };
                            for (i, n) in v["buckets"]
                                .as_array()
                                .into_iter()
                                .flatten()
                                .take(16)
                                .enumerate()
                            {
                                m.buckets[i] = n.as_u64().unwrap_or(0);
                            }
                            for (i, n) in v["statuses"]
                                .as_array()
                                .into_iter()
                                .flatten()
                                .take(6)
                                .enumerate()
                            {
                                m.statuses[i] = n.as_u64().unwrap_or(0);
                            }
                            m.history = v["history"]
                                .as_array()
                                .into_iter()
                                .flatten()
                                .rev()
                                .take(HISTORY)
                                .cloned()
                                .collect::<Vec<_>>()
                                .into_iter()
                                .rev()
                                .collect();
                            let mut data = this.0.lock().unwrap();
                            if section == "work" {
                                data.work.insert(key.clone(), m);
                            } else {
                                data.metrics.insert(key.clone(), m);
                            }
                        }
                    }
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
            Err(e) => return Err(e),
        }
        this.0.lock().unwrap().directory = Some(path.clone());
        this.0.lock().unwrap().sink = sink;
        let (tx, rx) = mpsc::sync_channel::<Command>(LOG_QUEUE_LIMIT);
        this.0.lock().unwrap().writer = Some(tx);
        let worker = Arc::downgrade(&this.0);
        let thread = std::thread::spawn(move || {
            let mut log = log.map(|file| BufWriter::with_capacity(64 * 1024, file));
            let mut last_log_flush = Instant::now();
            let mut size = log
                .as_ref()
                .and_then(|log| log.get_ref().metadata().ok())
                .map(|m| m.len())
                .unwrap_or(0);
            let mut last = Instant::now();
            loop {
                let event = rx.recv_timeout(Duration::from_secs(1));
                let Some(shared) = worker.upgrade() else {
                    break;
                };
                let mut failed = false;
                let rotation = (|| -> std::io::Result<()> {
                    let Some(log) = log.as_mut() else {
                        return Ok(());
                    };
                    let policy = shared.lock().unwrap().policy;
                    if size >= policy.chunk_bytes {
                        log.flush()?;
                        let segment = format!(
                            "application.segment.{:020}.{}.jsonl",
                            chrono::Utc::now().timestamp_micros(),
                            uuid::Uuid::new_v4()
                        );
                        fs::rename(path.join("application.jsonl"), path.join(segment))?;
                        *log = BufWriter::with_capacity(
                            64 * 1024,
                            OpenOptions::new()
                                .create(true)
                                .append(true)
                                .open(path.join("application.jsonl"))?,
                        );
                        size = 0;
                    }
                    Ok(())
                })();
                failed |= rotation.is_err();
                if let Ok(Command::Log(ref event)) = event {
                    {
                        let mut data = shared.lock().unwrap();
                        data.queued_bytes = data.queued_bytes.saturating_sub(
                            std::mem::size_of::<Value>() + crate::resources::heap_bytes(event),
                        );
                        data.queued_events = data.queued_events.saturating_sub(1);
                    }
                    let result = (|| -> std::io::Result<()> {
                        let Some(log) = log.as_mut() else {
                            return Ok(());
                        };
                        let line = event.to_string();
                        writeln!(log, "{line}")?;
                        size += line.len() as u64 + 1;
                        Ok(())
                    })();
                    failed |= result.is_err();
                }
                if last_log_flush.elapsed() >= Duration::from_secs(1) {
                    failed |= log.as_mut().is_some_and(|log| log.flush().is_err());
                    if log.is_some() {
                        failed |= cleanup(&path, shared.lock().unwrap().policy).is_err();
                    }
                    last_log_flush = Instant::now();
                }
                let disconnected = matches!(event, Err(mpsc::RecvTimeoutError::Disconnected));
                if last.elapsed() >= Duration::from_secs(30)
                    || disconnected
                    || matches!(event, Ok(Command::Flush(_)))
                {
                    let snapshot = Observability(shared.clone()).snapshot();
                    let result = (|| -> std::io::Result<()> {
                        fs::write(path.join("metrics.tmp"), snapshot.to_string())?;
                        fs::rename(path.join("metrics.tmp"), path.join("metrics.json"))
                    })();
                    failed |= result.is_err();
                    if let Ok(Command::Flush(ref reply)) = event {
                        let result = result
                            .and_then(|()| log.as_mut().map(|log| log.flush()).unwrap_or(Ok(())))
                            .and_then(|()| {
                                if log.is_some() {
                                    cleanup(&path, shared.lock().unwrap().policy)
                                } else {
                                    Ok(())
                                }
                            });
                        let _ = reply.send(result);
                    }
                    last = Instant::now();
                }
                if failed {
                    shared.lock().unwrap().storage_errors += 1;
                }
                if disconnected {
                    break;
                }
            }
        });
        this.0.lock().unwrap().writer_thread = Some(thread);
        Ok(this)
    }
    pub fn flush(&self) -> std::io::Result<()> {
        let writer = self.0.lock().unwrap().writer.clone();
        if let Some(writer) = writer {
            let (tx, rx) = mpsc::channel();
            writer
                .send(Command::Flush(tx))
                .map_err(|_| std::io::Error::other("Log writer stopped"))?;
            rx.recv_timeout(Duration::from_secs(10))
                .map_err(|_| std::io::Error::other("Log flush timed out"))??;
        }
        Ok(())
    }
    pub fn log(&self, mut event: Value) {
        let sink = self.0.lock().unwrap().sink.clone();
        if let Some((sink, project)) = sink {
            event["project"] = json!(project);
            if let Some(endpoint) = event["endpoint"].as_str().map(str::to_owned) {
                event["local_endpoint"] = json!(endpoint);
                if let Some((method, path)) = endpoint.split_once(' ') {
                    event["endpoint"] = json!(format!("{method} /{project}{path}"));
                }
            }
            sink.log(event);
            return;
        }
        event["time"] = json!(chrono::Utc::now().to_rfc3339());
        let mut data = self.0.lock().unwrap();
        if let Some((instance, name)) = &data.identity {
            event["server"] = json!(instance);
            event["server_name"] = json!(name);
        }
        if event.get("project").is_none() {
            event["project"] = json!("system");
        }
        static SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let minimum =
            (data.log_sequence + 1).max(chrono::Utc::now().timestamp_micros().max(0) as u64);
        SEQUENCE.fetch_max(minimum, std::sync::atomic::Ordering::Relaxed);
        data.log_sequence = SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        event["sequence"] = json!(data.log_sequence);
        if event.get("level").is_none() {
            event["level"] = json!(if event["status"].as_u64().is_some_and(|s| s >= 500)
                || event["success"] == false
            {
                "error"
            } else if event["status"].as_u64().is_some_and(|s| s >= 400) {
                "warn"
            } else {
                "info"
            });
        }
        let bytes = std::mem::size_of::<Value>() + crate::resources::heap_bytes(&event);
        if event.to_string().len() > 65536
            || data.queued_bytes.saturating_add(bytes) > LOG_QUEUE_BYTES
        {
            data.dropped += 1;
            return;
        }
        if let Some(tx) = &data.writer {
            if tx.try_send(Command::Log(event.clone())).is_err() {
                data.dropped += 1;
            } else {
                data.queued_bytes +=
                    std::mem::size_of::<Value>() + crate::resources::heap_bytes(&event);
                data.queued_events += 1;
            }
        }
        if data.logs.len() == LOG_BUFFER_LIMIT {
            data.logs.pop_front();
        }
        data.logs.push_back(event);
    }
    pub fn request(&self, endpoint: &str, status: u16, elapsed: Duration, id: &str) {
        let ms = elapsed.as_secs_f64() * 1000.;
        {
            let mut data = self.0.lock().unwrap();
            data.metrics
                .entry(endpoint.into())
                .or_default()
                .observe(ms, status >= 400, status);
            data.service.event("http_requests", 1);
            if status >= 400 {
                data.service.event("http_errors", 1);
            }
        }
        self.log(json!({
            "kind": "request",
            "request_id": id,
            "endpoint": endpoint,
            "status": status,
            "duration_ms": ms
        }));
    }
    pub fn snapshot(&self) -> Value {
        let mut d = self.0.lock().unwrap();
        let service = d.service.snapshot();
        json!({
            "service": service,
            "work": d.work.iter().map(|(k,m)|(k.clone(),m.json())).collect::<BTreeMap<_,_>>(),
            "log_sequence": d.log_sequence,
            "endpoints": d.metrics.iter().map(|(k, m)| (k.clone(), m.json())).collect::<BTreeMap<_, _>>(),
            "histogram_bounds_ms": BOUNDS,
            "percentiles": "histogram bucket upper bounds; cumulative since first start",
            "history_minutes": HISTORY,
            "dropped_logs": d.dropped,
            "storage_errors": d.storage_errors
        })
    }
    pub fn resources(&self) -> Value {
        let d = self.0.lock().unwrap();
        let metrics = d
            .metrics
            .iter()
            .chain(d.work.iter())
            .map(|(key, m)| {
                key.capacity()
                    + std::mem::size_of::<Metric>()
                    + 3 * std::mem::size_of::<usize>()
                    + m.history.capacity() * std::mem::size_of::<Value>()
                    + m.history
                        .iter()
                        .map(crate::resources::heap_bytes)
                        .sum::<usize>()
            })
            .sum::<usize>();
        let logs = d.logs.capacity() * std::mem::size_of::<Value>()
            + d.logs
                .iter()
                .map(crate::resources::heap_bytes)
                .sum::<usize>();
        let disk = d.directory.as_ref().and_then(|path| {
            let log_files = log_files(path).ok()?;
            let log_disk = log_files
                .iter()
                .map(crate::resources::file_bytes)
                .collect::<Option<Vec<_>>>()
                .map(|sizes| sizes.iter().sum::<u64>())?;
            let mut files = log_files;
            files.push(path.join("metrics.json"));
            files.push(path.join("metrics.tmp"));
            let total = files
                .iter()
                .map(crate::resources::file_bytes)
                .collect::<Option<Vec<_>>>()
                .map(|sizes| sizes.iter().sum::<u64>())?;
            Some((log_disk, total))
        });
        json!({
            "estimated_metrics_bytes": metrics + d.service.estimated_bytes(),
            "estimated_log_buffer_bytes": logs,
            "estimated_log_queue_bytes": d.queued_bytes,
            "log_writer_buffer_bytes": if d.writer.is_some() { 65536 } else { 0 },
            "queued_log_events": d.queued_events,
            "log_disk_bytes": disk.map(|(logs, _)| logs),
            "disk_bytes": disk.map(|(_, total)| total),
            "log_buffer_limit": LOG_BUFFER_LIMIT,
            "log_queue_limit": LOG_QUEUE_LIMIT,
            "log_disk_limit_bytes": if d.sink.is_none() {d.policy.target_bytes}else{0},
            "log_chunk_bytes": d.policy.chunk_bytes,
            "log_queue_limit_bytes": LOG_QUEUE_BYTES,
            "shared_log_owner":d.sink.is_some(),
            "oldest_log_time": if d.sink.is_none() { d.directory.as_ref().and_then(|path|oldest(path)) } else {None},
            "history_minutes": HISTORY,
            "estimate_note": "Estimates include collection capacity and payloads; exclude allocator overhead, fragmentation, active requests, thread stacks and shared runtime allocations."
        })
    }
    pub fn event(&self, name: &str, count: u64) {
        self.0.lock().unwrap().service.event(name, count);
    }
    pub fn gauge(&self, name: &str, delta: i64) {
        self.0.lock().unwrap().service.gauge(name, delta);
    }
    pub fn gauge_guard(&self, name: &str) -> Gauge {
        Gauge {
            observer: self.clone(),
            name: name.into(),
            value: 0,
        }
    }
    pub fn sample(&self, resources: Value) {
        self.0.lock().unwrap().service.sample(resources);
    }
    pub fn log_source(&self) -> (Vec<Value>, Option<PathBuf>) {
        let data = self.0.lock().unwrap();
        if let Some((sink, _)) = &data.sink {
            let sink = sink.clone();
            drop(data);
            return sink.log_source();
        }
        (
            data.logs.iter().rev().cloned().collect(),
            data.directory.clone(),
        )
    }
    fn work(&self, kind: &str, name: &str, elapsed: Duration, success: bool) {
        let mut data = self.0.lock().unwrap();
        let key = format!("{kind}:{name}");
        if data.work.len() < 128 || data.work.contains_key(&key) {
            data.work.entry(key).or_default().observe(
                elapsed.as_secs_f64() * 1000.,
                !success,
                if success { 200 } else { 500 },
            );
        }
        data.service.event(
            if kind == "plugin" {
                "plugin_calls"
            } else {
                "io_calls"
            },
            1,
        );
        if !success {
            data.service.event("work_errors", 1);
        }
    }
    pub fn logs(&self) -> Value {
        json!(self.0.lock().unwrap().logs)
    }
    pub fn span(&self, kind: &str, name: &str) -> Span {
        Span {
            observer: self.clone(),
            kind: kind.into(),
            name: name.into(),
            start: Instant::now(),
            success: false,
        }
    }
}
pub struct Span {
    observer: Observability,
    kind: String,
    name: String,
    start: Instant,
    success: bool,
}
impl Span {
    pub fn success(&mut self) {
        self.success = true;
    }
}
impl Drop for Span {
    fn drop(&mut self) {
        self.observer
            .work(&self.kind, &self.name, self.start.elapsed(), self.success);
        self.observer.log(json!({
            "kind": self.kind,
            "name": self.name,
            "success": self.success,
            "duration_ms": self.start.elapsed().as_secs_f64() * 1000.
        }));
    }
}

pub struct Gauge {
    observer: Observability,
    name: String,
    value: i64,
}
impl Gauge {
    pub fn set(&mut self, value: usize) {
        let value = value.min(i64::MAX as usize) as i64;
        self.observer.gauge(&self.name, value - self.value);
        self.value = value;
    }
}
impl Drop for Gauge {
    fn drop(&mut self) {
        self.observer.gauge(&self.name, -self.value);
    }
}
