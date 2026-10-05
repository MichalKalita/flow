use flow_runtime::observability::Observability;
use serde_json::{Value, json};
use std::time::Duration;

#[test]
fn bounded_logs_and_metrics_restore_without_database() {
    let path = std::env::temp_dir().join(format!("flow-metrics-{}", uuid::Uuid::new_v4()));
    let observer = Observability::disk(&path).unwrap();
    for i in 0..250 {
        observer.request(
            "GET /devices/{id}",
            if i == 0 { 500 } else { 200 },
            Duration::from_millis(12),
            "request-id",
        );
    }
    let metrics = observer.snapshot();
    assert_eq!(metrics["endpoints"]["GET /devices/{id}"]["count"], 250);
    assert_eq!(metrics["endpoints"]["GET /devices/{id}"]["errors"], 1);
    assert_eq!(metrics["endpoints"]["GET /devices/{id}"]["p95_ms"], 20.);
    assert_eq!(observer.logs().as_array().unwrap().len(), 200);
    let mut span = observer.span("plugin", "Payment.createUrl");
    span.success();
    drop(span);
    observer.flush().unwrap();
    let disk: Value =
        serde_json::from_slice(&std::fs::read(path.join("metrics.json")).unwrap()).unwrap();
    assert_eq!(disk["endpoints"], metrics["endpoints"]);
    let logs = std::fs::read_to_string(path.join("application.jsonl")).unwrap();
    assert!(
        logs.lines()
            .any(|line| serde_json::from_str::<Value>(line).unwrap()["kind"] == "plugin")
    );
    drop(observer);
    let restarted = Observability::disk(&path).unwrap();
    restarted.request(
        "GET /devices/{id}",
        200,
        Duration::from_millis(5),
        "after-restart",
    );
    assert_eq!(
        restarted.snapshot()["endpoints"]["GET /devices/{id}"]["count"],
        251
    );
    restarted.flush().unwrap();
    drop(restarted);
    // Worker owns no strong observer reference between commands; temporary files
    // can be removed while its file handle closes on the next timeout.
    std::fs::remove_dir_all(path).unwrap();
}

#[test]
fn failed_span_and_request_logs_never_contain_inputs() {
    let observer = Observability::default();
    drop(observer.span("io", "sqlite.apply"));
    assert_eq!(observer.logs()[0]["success"], false);
    observer.log(json!({"kind":"startup"}));
    assert!(observer.logs()[1]["time"].is_string());
}

#[test]
fn resource_report_measures_sqlite_files_and_separates_estimates() {
    use flow_runtime::engine::{Config, Runtime};
    let path = std::env::temp_dir().join(format!("flow-resources-{}.sqlite", uuid::Uuid::new_v4()));
    let source = r#"[auth [anonymous Anonymous]] [transport HTTP [auth anonymous]] [query Health [output Bool] [http GET "/health"] [result true]]"#;
    let runtime = Runtime::open(source, path.to_str().unwrap(), Config::default()).unwrap();
    let report = runtime.storage_resources();
    let db = &report["sqlite"];
    assert_eq!(
        db["main_bytes"].as_u64().unwrap(),
        std::fs::metadata(&path).unwrap().len()
    );
    let sum = ["main_bytes", "wal_bytes", "shm_bytes"]
        .iter()
        .map(|key| db[*key].as_u64().unwrap())
        .sum::<u64>();
    assert_eq!(db["total_disk_bytes"].as_u64().unwrap(), sum);
    assert!(db["page_cache_bytes"].as_u64().unwrap() > 0);
    runtime
        .observability
        .request("GET /health", 200, Duration::from_millis(1), "id");
    let estimates = runtime.observability.resources();
    assert!(estimates["estimated_metrics_bytes"].as_u64().unwrap() > 0);
    assert!(estimates["estimated_log_buffer_bytes"].as_u64().unwrap() > 0);
    assert_eq!(estimates["log_queue_limit"], 512);
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    assert!(
        flow_runtime::resources::process_memory()["rss_bytes"]
            .as_u64()
            .unwrap()
            > 0
    );
    drop(runtime);
    std::fs::remove_file(path).unwrap();
}
