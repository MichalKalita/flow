use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, VecDeque},
    fs::{self, OpenOptions},
    io::{BufWriter, Write},
    path::PathBuf,
    sync::{Arc, Mutex, mpsc},
    time::{Duration, Instant},
};

const BOUNDS: [f64; 15] = [
    1., 2., 5., 10., 20., 50., 100., 200., 500., 1000., 2000., 5000., 10000., 30000., 60000.,
];
const HISTORY: usize = 360;
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
    dropped: u64,
    storage_errors: u64,
    queued_bytes: usize,
    queued_events: usize,
    directory: Option<PathBuf>,
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
}
impl Metric {
    fn json(&self) -> Value {
        let percentile = |p: f64| {
            let rank = (self.count as f64 * p).ceil() as u64;
            let mut n = 0;
            if rank == 0 {
                return Value::Null;
            }
            for (i, count) in self.buckets.iter().enumerate() {
                n += count;
                if n >= rank {
                    return BOUNDS.get(i).map(|v| json!(v)).unwrap_or(json!(">60000"));
                }
            }
            Value::Null
        };
        let cutoff = chrono::Utc::now().timestamp() / 60 - HISTORY as i64;
        let mut history = self
            .history
            .iter()
            .filter(|v| v["minute"].as_i64().unwrap_or(0) > cutoff)
            .cloned()
            .collect::<VecDeque<_>>();
        if self.minute_count > 0 && self.minute > cutoff {
            history.push_back(json!({
                "minute": self.minute,
                "count": self.minute_count,
                "errors": self.minute_errors,
                "mean_ms": self.minute_sum/self.minute_count as f64
            }));
        }
        json!({
            "count": self.count,
            "errors": self.errors,
            "sum_ms": self.sum,
            "mean_ms": if self.count==0{0.}else{self.sum/self.count as f64},
            "p50_ms": percentile(0.5),
            "p95_ms": percentile(0.95),
            "p99_ms": percentile(0.99),
            "buckets": self.buckets,
            "history": history
        })
    }
}
impl Observability {
    pub fn disk(path: impl Into<PathBuf>) -> std::io::Result<Self> {
        let path = path.into();
        fs::create_dir_all(&path)?;
        let log = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path.join("application.jsonl"))?;
        let this = Self::default();
        // Restore bounded aggregates. Malformed snapshots fail startup rather than silently resetting counters.
        match fs::read(path.join("metrics.json")) {
            Ok(bytes) => {
                let v: Value = serde_json::from_slice(&bytes)?;
                if let Some(metrics) = v["endpoints"].as_object() {
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
                        this.0.lock().unwrap().metrics.insert(key.clone(), m);
                    }
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
            Err(e) => return Err(e),
        }
        this.0.lock().unwrap().directory = Some(path.clone());
        let (tx, rx) = mpsc::sync_channel::<Command>(512);
        this.0.lock().unwrap().writer = Some(tx);
        let worker = Arc::downgrade(&this.0);
        std::thread::spawn(move || {
            let mut log = BufWriter::with_capacity(64 * 1024, log);
            let mut last_log_flush = Instant::now();
            let mut size = log.get_ref().metadata().map(|m| m.len()).unwrap_or(0);
            let mut last = Instant::now();
            loop {
                let event = rx.recv_timeout(Duration::from_secs(1));
                let Some(shared) = worker.upgrade() else {
                    break;
                };
                let mut failed = false;
                if let Ok(Command::Log(ref event)) = event {
                    {
                        let mut data = shared.lock().unwrap();
                        data.queued_bytes = data.queued_bytes.saturating_sub(
                            std::mem::size_of::<Value>() + crate::resources::heap_bytes(event),
                        );
                        data.queued_events = data.queued_events.saturating_sub(1);
                    }
                    let result = (|| -> std::io::Result<()> {
                        if size >= 8 * 1024 * 1024 {
                            log.flush()?;
                            for i in (1..=3).rev() {
                                let from = if i == 1 {
                                    path.join("application.jsonl")
                                } else {
                                    path.join(format!("application.{}.jsonl", i - 1))
                                };
                                let to = path.join(format!("application.{i}.jsonl"));
                                match fs::rename(from, to) {
                                    Ok(()) => (),
                                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
                                    Err(e) => return Err(e),
                                }
                            }
                            log = BufWriter::with_capacity(
                                64 * 1024,
                                OpenOptions::new()
                                    .create(true)
                                    .append(true)
                                    .open(path.join("application.jsonl"))?,
                            );
                            size = 0;
                        }
                        let line = event.to_string();
                        writeln!(log, "{line}")?;
                        size += line.len() as u64 + 1;
                        Ok(())
                    })();
                    failed = result.is_err();
                }
                if last_log_flush.elapsed() >= Duration::from_secs(1) {
                    failed |= log.flush().is_err();
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
                        let result = result.and_then(|()| log.flush());
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
        event["time"] = json!(chrono::Utc::now().to_rfc3339());
        let mut data = self.0.lock().unwrap();
        if let Some(tx) = &data.writer {
            if tx.try_send(Command::Log(event.clone())).is_err() {
                data.dropped += 1;
            } else {
                data.queued_bytes +=
                    std::mem::size_of::<Value>() + crate::resources::heap_bytes(&event);
                data.queued_events += 1;
            }
        }
        if data.logs.len() == 200 {
            data.logs.pop_front();
        }
        data.logs.push_back(event);
    }
    pub fn request(&self, endpoint: &str, status: u16, elapsed: Duration, id: &str) {
        let ms = elapsed.as_secs_f64() * 1000.;
        {
            let mut data = self.0.lock().unwrap();
            let m = data.metrics.entry(endpoint.into()).or_default();
            let minute = chrono::Utc::now().timestamp() / 60;
            if m.minute != minute && m.minute_count > 0 {
                if m.history.len() >= HISTORY - 1 {
                    m.history.pop_front();
                }
                m.history.push_back(json!({
                    "minute": m.minute,
                    "count": m.minute_count,
                    "errors": m.minute_errors,
                    "mean_ms": m.minute_sum/m.minute_count as f64
                }));
                m.minute_count = 0;
                m.minute_errors = 0;
                m.minute_sum = 0.;
            }
            m.minute = minute;
            m.count += 1;
            m.minute_count += 1;
            m.sum += ms;
            m.minute_sum += ms;
            if status >= 400 {
                m.errors += 1;
                m.minute_errors += 1;
            }
            let bucket = BOUNDS.iter().position(|b| ms <= *b).unwrap_or(15);
            m.buckets[bucket] += 1;
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
        let d = self.0.lock().unwrap();
        json!({
            "endpoints": d.metrics.iter().map(|(k,m)|(k.clone(),m.json())).collect::<BTreeMap<_,_>>(),
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
            let mut files = vec![
                path.join("application.jsonl"),
                path.join("metrics.json"),
                path.join("metrics.tmp"),
            ];
            files.extend((1..=3).map(|i| path.join(format!("application.{i}.jsonl"))));
            files
                .iter()
                .map(crate::resources::file_bytes)
                .collect::<Option<Vec<_>>>()
                .map(|sizes| sizes.iter().sum::<u64>())
        });
        json!({
            "estimated_metrics_bytes": metrics,
            "estimated_log_buffer_bytes": logs,
            "estimated_log_queue_bytes": d.queued_bytes,
            "log_writer_buffer_bytes": if d.writer.is_some(){65536}else{0},
            "queued_log_events": d.queued_events,
            "disk_bytes": disk,
            "log_buffer_limit": 200,
            "log_queue_limit": 512,
            "history_minutes": HISTORY,
            "estimate_note": "Estimates include collection capacity and payloads; exclude allocator overhead, fragmentation, active requests, thread stacks and shared runtime allocations."
        })
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
        self.observer.log(json!({
            "kind": self.kind,
            "name": self.name,
            "success": self.success,
            "duration_ms": self.start.elapsed().as_secs_f64()*1000.
        }));
    }
}
