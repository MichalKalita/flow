use crate::projects::Projects;
use serde_json::{Value, json};
use std::collections::BTreeMap;
fn sum_object(target: &mut Value, value: &Value) {
    if !target.is_object() {
        *target = json!({})
    }
    if let Some(fields) = value.as_object() {
        for (key, value) in fields {
            if let Some(number) = value.as_f64() {
                target[key] = json!(target[key].as_f64().unwrap_or(0.) + number);
            }
        }
    }
}
fn percentile(buckets: &Value, rank: f64, bounds: &Value) -> Value {
    let values = buckets.as_array().cloned().unwrap_or_default();
    let total = values.iter().filter_map(Value::as_f64).sum::<f64>();
    if total == 0. {
        return Value::Null;
    }
    let mut sum = 0.;
    for (i, value) in values.iter().enumerate() {
        sum += value.as_f64().unwrap_or(0.);
        if sum >= (total * rank).ceil() {
            return bounds.get(i).cloned().unwrap_or(json!(">60000"));
        }
    }
    Value::Null
}
fn finish(metric: &mut Value, bounds: &Value) {
    let count = metric["count"].as_f64().unwrap_or(0.);
    metric["mean_ms"] = json!(if count > 0. {
        metric["sum_ms"].as_f64().unwrap_or(0.) / count
    } else {
        0.
    });
    for (key, rank) in [("p50_ms", 0.5), ("p95_ms", 0.95), ("p99_ms", 0.99)] {
        metric[key] = percentile(&metric["buckets"], rank, bounds);
    }
}
fn merge_metric(target: &mut Value, value: &Value) {
    for key in ["count", "errors", "sum_ms"] {
        target[key] = json!(target[key].as_f64().unwrap_or(0.) + value[key].as_f64().unwrap_or(0.));
    }
    for (key, n) in [("buckets", 16), ("statuses", 6)] {
        let mut values = vec![0.; n];
        for (i, v) in values.iter_mut().enumerate() {
            *v = target[key].get(i).and_then(Value::as_f64).unwrap_or(0.)
                + value[key].get(i).and_then(Value::as_f64).unwrap_or(0.)
        }
        target[key] = json!(values);
    }
}
pub fn system_overview(projects: &Projects) -> Value {
    let mut result = json!({"endpoints":[],"streams":[],"automations":[],"metrics":{"endpoints":{},"work":{},"dropped_logs":0,"storage_errors":0,"service":{"counters":{},"gauges":{},"history":[],"seconds":[],"uptime_seconds":0,"resources":{}}},"resources":{"sqlite":{},"observability":{},"process":crate::resources::process_memory(),"host":crate::resources::host_memory()},"version":env!("CARGO_PKG_VERSION")});
    let mut total = json!({"count":0,"errors":0,"sum_ms":0,"buckets":[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0],"statuses":[0,0,0,0,0,0]});
    let mut history = BTreeMap::<i64, Value>::new();
    let mut services = BTreeMap::<i64, Value>::new();
    let mut seconds = BTreeMap::<i64, Value>::new();
    let system = projects.system.snapshot();
    result["metrics"]["histogram_bounds_ms"] = system["histogram_bounds_ms"].clone();
    let bounds = result["metrics"]["histogram_bounds_ms"].clone();
    result["metrics"]["service"]["uptime_seconds"] = system["service"]["uptime_seconds"].clone();
    result["metrics"]["service"]["resources"] = system["service"]["resources"].clone();
    let views = projects.active().into_iter().map(|(name,runtime)| (name,crate::admin::overview_value(&runtime.lock().unwrap()))).chain(projects.default_name().is_none().then(||("system".to_string(),json!({"metrics":system,"resources":{"sqlite":{},"observability":projects.system.resources()}}))));
    for (name, view) in views {
        for section in ["endpoints", "streams", "automations"] {
            for mut item in view[section].as_array().cloned().unwrap_or_default() {
                item["project"] = json!(name);
                item["name"] = json!(format!("{name}/{}", item["name"].as_str().unwrap_or("")));
                if section == "endpoints" {
                    item["path"] = json!(format!("/{name}{}", item["path"].as_str().unwrap_or("")));
                }
                result[section].as_array_mut().unwrap().push(item);
            }
        }
        for section in ["endpoints", "work"] {
            if let Some(metrics) = view["metrics"][section].as_object() {
                for (label, value) in metrics {
                    let key = if section == "endpoints" {
                        if let Some((method, path)) = label.split_once(' ') {
                            format!("{method} /{name}{path}")
                        } else {
                            format!("{name}/{label}")
                        }
                    } else {
                        format!("{name}/{label}")
                    };
                    let mut compact = value.clone();
                    compact["history"] = json!([]);
                    result["metrics"][section][key] = compact;
                    if section == "endpoints" {
                        merge_metric(&mut total, value);
                        for point in value["history"].as_array().into_iter().flatten() {
                            let minute = point["minute"].as_i64().unwrap_or(0);
                            let target = history.entry(minute).or_insert(json!({"minute":minute}));
                            let mut point = point.clone();
                            point["sum_ms"] = json!(
                                point["mean_ms"].as_f64().unwrap_or(0.)
                                    * point["count"].as_f64().unwrap_or(0.)
                            );
                            merge_metric(target, &point);
                        }
                    }
                }
            }
        }
        for key in ["dropped_logs", "storage_errors"] {
            result["metrics"][key] = json!(
                result["metrics"][key].as_u64().unwrap_or(0)
                    + view["metrics"][key].as_u64().unwrap_or(0)
            );
        }
        for category in ["counters", "gauges"] {
            sum_object(
                &mut result["metrics"]["service"][category],
                &view["metrics"]["service"][category],
            );
        }
        for point in view["metrics"]["service"]["history"]
            .as_array()
            .into_iter()
            .flatten()
        {
            let minute = point["minute"].as_i64().unwrap_or(0);
            let target = services
                .entry(minute)
                .or_insert(json!({"minute":minute,"resources":point["resources"]}));
            for category in ["counts", "gauges"] {
                sum_object(&mut target[category], &point[category]);
            }
        }
        for point in view["metrics"]["service"]["seconds"]
            .as_array()
            .into_iter()
            .flatten()
        {
            let second = point["second"].as_i64().unwrap_or(0);
            let target = seconds.entry(second).or_insert(json!({"second":second}));
            sum_object(&mut target["counts"], &point["counts"]);
        }
        for category in ["sqlite", "observability"] {
            sum_object(
                &mut result["resources"][category],
                &view["resources"][category],
            );
        }
    }
    for point in history.values_mut() {
        finish(point, &bounds)
    }
    finish(&mut total, &bounds);
    total["history"] = json!(history.into_values().collect::<Vec<_>>());
    // One rollup supplies system charts; endpoint summaries retain their own totals.
    result["metrics"]["system"] = total;
    result["metrics"]["service"]["history"] = json!(services.into_values().collect::<Vec<_>>());
    result["metrics"]["service"]["seconds"] = json!(seconds.into_values().collect::<Vec<_>>());
    result
}
pub fn system_logs(projects: &Projects, query: &BTreeMap<String, String>) -> Value {
    let before = query
        .get("before")
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(u64::MAX);
    let mut rows = Vec::new();
    let mut sources = projects
        .active()
        .into_iter()
        .map(|(name, runtime)| (name, runtime.lock().unwrap().observability.clone()))
        .collect::<Vec<_>>();
    sources.push(("system".into(), projects.system.clone()));
    for (name, observer) in sources {
        for mut event in observer.log_source().0 {
            if event["sequence"].as_u64().unwrap_or(0) >= before {
                continue;
            }
            event["project"] = json!(name);
            if let Some(endpoint) = event["endpoint"].as_str() {
                event["endpoint"] = json!(if let Some((method, path)) = endpoint.split_once(' ') {
                    format!("{method} /{name}{path}")
                } else {
                    format!("{name}/{endpoint}")
                });
            }
            if query
                .get("since")
                .filter(|s| !s.is_empty())
                .is_some_and(|s| event["time"].as_str().is_none_or(|t| t < s.as_str()))
            {
                continue;
            }
            if query
                .get("status")
                .filter(|s| !s.is_empty())
                .is_some_and(|s| s.parse::<u64>().ok() != event["status"].as_u64())
            {
                continue;
            }
            let text = event.to_string().to_lowercase();
            if query
                .get("search")
                .is_some_and(|s| !text.contains(&s.to_lowercase()))
            {
                continue;
            }
            if ["kind", "level", "endpoint"].iter().any(|key| {
                query
                    .get(*key)
                    .is_some_and(|v| !v.is_empty() && event[*key].as_str() != Some(v))
            }) {
                continue;
            }
            event["project"] = json!(name);
            rows.push(event);
        }
    }
    rows.sort_by_key(|v| std::cmp::Reverse(v["sequence"].as_u64().unwrap_or(0)));
    let more = rows.len() > 100;
    rows.truncate(100);
    let cursor = if more {
        rows.last().map(|v| v["sequence"].to_string())
    } else {
        None
    };
    json!({"entries":rows,"next_cursor":cursor,"scan_limited":false,"cursor_expired":false,"scanned_bytes":0,"source":"System recent buffers; select a project for archive history"})
}

pub fn system_audit(projects: &Projects, query: &BTreeMap<String, String>) -> crate::Result<Value> {
    let raw = query.get("before").map(String::as_str).unwrap_or("0");
    if raw.len() > 8192 {
        return Err(crate::Error::new("invalid_input", "Invalid audit cursor"));
    }
    let mut cursors: BTreeMap<String, i64> = if raw == "0" {
        BTreeMap::new()
    } else {
        serde_json::from_str(raw)?
    };
    let mut rows = Vec::new();
    for (name, runtime) in projects.active() {
        let page = runtime.lock().unwrap().audit_filtered(
            *cursors.get(&name).unwrap_or(&0),
            100,
            query,
        )?;
        for mut row in page.as_array().cloned().unwrap_or_default() {
            row["project"] = json!(name);
            rows.push(row);
        }
        rows.sort_by(|a, b| {
            b["time"]
                .as_str()
                .cmp(&a["time"].as_str())
                .then_with(|| b["id"].as_i64().cmp(&a["id"].as_i64()))
                .then_with(|| a["project"].as_str().cmp(&b["project"].as_str()))
        });
        rows.truncate(100);
    }
    for row in &mut rows {
        cursors.insert(
            row["project"].as_str().unwrap().to_owned(),
            row["id"].as_i64().unwrap(),
        );
        row["cursor"] = json!(serde_json::to_string(&cursors)?);
    }
    Ok(json!(rows))
}
