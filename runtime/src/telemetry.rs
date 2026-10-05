use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, VecDeque},
    time::Instant,
};

pub(crate) struct Service {
    pub counters: BTreeMap<String, u64>,
    pub gauges: BTreeMap<String, i64>,
    history: VecDeque<Value>,
    seconds: VecDeque<Value>,
    minute: i64,
    second: i64,
    minute_counts: BTreeMap<String, u64>,
    second_counts: BTreeMap<String, u64>,
    resources: Value,
    previous_cpu: Option<(Instant, f64)>,
    started: Instant,
}
impl Default for Service {
    fn default() -> Self {
        Self {
            counters: BTreeMap::new(),
            gauges: BTreeMap::new(),
            history: VecDeque::new(),
            seconds: VecDeque::new(),
            minute: 0,
            second: 0,
            minute_counts: BTreeMap::new(),
            second_counts: BTreeMap::new(),
            resources: Value::Null,
            previous_cpu: None,
            started: Instant::now(),
        }
    }
}
impl Service {
    fn point(&self) -> Value {
        json!({"minute": self.minute, "counts": self.minute_counts, "gauges": self.gauges, "resources": self.resources})
    }
    fn roll(&mut self) {
        let now = chrono::Utc::now().timestamp();
        if self.minute != now / 60 {
            if self.minute > 0 {
                if self.history.len() >= 359 {
                    self.history.pop_front();
                }
                self.history.push_back(self.point());
            }
            self.minute = now / 60;
            self.minute_counts.clear();
        }
        if self.second != now {
            if self.second > 0 {
                if self.seconds.len() >= 59 {
                    self.seconds.pop_front();
                }
                self.seconds
                    .push_back(json!({"second": self.second, "counts": self.second_counts}));
            }
            self.second = now;
            self.second_counts.clear();
        }
    }
    pub fn event(&mut self, name: &str, count: u64) {
        self.roll();
        *self.counters.entry(name.into()).or_default() += count;
        *self.minute_counts.entry(name.into()).or_default() += count;
        *self.second_counts.entry(name.into()).or_default() += count;
    }
    pub fn gauge(&mut self, name: &str, delta: i64) {
        self.roll();
        let value = self.gauges.entry(name.into()).or_default();
        *value = (*value + delta).max(0);
    }
    pub fn sample(&mut self, mut resources: Value) {
        self.roll();
        if let Some(cpu) = resources["cpu_seconds"].as_f64() {
            let now = Instant::now();
            if let Some((previous_time, previous_cpu)) = self.previous_cpu {
                resources["cpu_percent"] = json!(
                    ((cpu - previous_cpu) / now.duration_since(previous_time).as_secs_f64() * 100.)
                        .max(0.)
                );
            }
            self.previous_cpu = Some((now, cpu));
        }
        self.resources = resources;
    }
    pub fn restore(&mut self, value: &Value) {
        if let Some(counters) = value["counters"].as_object() {
            self.counters = counters
                .iter()
                .filter_map(|(k, v)| v.as_u64().map(|n| (k.clone(), n)))
                .collect();
        }
        self.history = value["history"]
            .as_array()
            .into_iter()
            .flatten()
            .rev()
            .take(359)
            .cloned()
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        // Active connections and sub-second rates always start at zero after a restart.
    }
    pub fn snapshot(&mut self) -> Value {
        self.roll();
        let cutoff = chrono::Utc::now().timestamp() / 60 - 360;
        let mut history = self
            .history
            .iter()
            .filter(|v| v["minute"].as_i64().unwrap_or(0) > cutoff)
            .cloned()
            .collect::<Vec<_>>();
        history.push(self.point());
        let cutoff_seconds = chrono::Utc::now().timestamp() - 60;
        let mut seconds = self
            .seconds
            .iter()
            .filter(|v| v["second"].as_i64().unwrap_or(0) > cutoff_seconds)
            .cloned()
            .collect::<Vec<_>>();
        seconds.push(json!({"second": self.second, "counts": self.second_counts}));
        json!({"counters": self.counters, "gauges": self.gauges, "history": history, "seconds": seconds, "resources": self.resources, "uptime_seconds": self.started.elapsed().as_secs()})
    }
    pub fn estimated_bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            + self.history.capacity() * std::mem::size_of::<Value>()
            + self.seconds.capacity() * std::mem::size_of::<Value>()
            + self
                .history
                .iter()
                .chain(self.seconds.iter())
                .map(crate::resources::heap_bytes)
                .sum::<usize>()
            + crate::resources::heap_bytes(&self.resources)
    }
}
