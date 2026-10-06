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
    assert_eq!(
        estimates["log_queue_limit"].as_u64().unwrap(),
        flow_runtime::observability::LOG_QUEUE_LIMIT as u64
    );
    assert_eq!(
        estimates["log_buffer_limit"].as_u64().unwrap(),
        flow_runtime::observability::LOG_BUFFER_LIMIT as u64
    );
    assert_eq!(
        estimates["log_disk_limit_bytes"].as_u64().unwrap(),
        flow_runtime::observability::LOG_DISK_LIMIT_BYTES
    );
    assert_eq!(
        flow_runtime::observability::LOG_DISK_LIMIT_BYTES,
        1024 * 1024 * 1024
    );
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    assert!(
        flow_runtime::resources::process_memory()["rss_bytes"]
            .as_u64()
            .unwrap()
            > 0
    );
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        let host = flow_runtime::resources::host_memory();
        let limit = host["limit_bytes"].as_u64().unwrap();
        assert!(limit >= 512 * 1024 * 1024);
        if let Some(physical) = host["physical_bytes"].as_u64() {
            assert!(limit <= physical);
        }
    }
    drop(runtime);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn live_gauges_release_and_work_counters_restore() {
    let path = std::env::temp_dir().join(format!("flow-service-{}", uuid::Uuid::new_v4()));
    let observer = Observability::disk(&path).unwrap();
    {
        let mut first = observer.gauge_guard("ws_connections");
        first.set(2);
        let mut second = observer.gauge_guard("ws_connections");
        second.set(1);
        assert_eq!(
            observer.snapshot()["service"]["gauges"]["ws_connections"],
            3
        );
        first.set(1);
    }
    assert_eq!(
        observer.snapshot()["service"]["gauges"]["ws_connections"],
        0
    );
    observer.event("stream_events", 7);
    let mut span = observer.span("io", "sqlite.test");
    span.success();
    drop(span);
    observer.flush().unwrap();
    drop(observer);
    let restored = Observability::disk(&path).unwrap();
    let metrics = restored.snapshot();
    assert_eq!(metrics["service"]["counters"]["stream_events"], 7);
    assert_eq!(metrics["service"]["counters"]["io_calls"], 1);
    assert!(metrics["service"]["gauges"].as_object().unwrap().is_empty());
    assert_eq!(metrics["work"]["io:sqlite.test"]["count"], 1);
    drop(restored);
    std::fs::remove_dir_all(path).unwrap();
}

#[test]
fn archive_log_filters_and_pagination_extend_beyond_memory() {
    use std::collections::BTreeMap;
    let path = std::env::temp_dir().join(format!("flow-log-pages-{}", uuid::Uuid::new_v4()));
    let observer = Observability::disk(&path).unwrap();
    for i in 0..350 {
        observer.request(
            "GET /health",
            200,
            Duration::from_millis(1),
            &format!("request-{i}"),
        );
    }
    observer.flush().unwrap();
    let mut filters = BTreeMap::from([
        ("limit".into(), "100".into()),
        ("kind".into(), "request".into()),
    ]);
    let mut seen = std::collections::BTreeSet::new();
    loop {
        let page = flow_runtime::log_store::query(&observer, &filters).unwrap();
        for row in page["entries"].as_array().unwrap() {
            assert!(seen.insert(row["sequence"].as_u64().unwrap()));
        }
        let Some(cursor) = page["next_cursor"].as_str() else {
            break;
        };
        filters.insert("before".into(), cursor.into());
    }
    assert_eq!(seen.len(), 350);
    filters.remove("before");
    filters.insert("search".into(), "request-0".into());
    let page = flow_runtime::log_store::query(&observer, &filters).unwrap();
    assert_eq!(page["entries"][0]["request_id"], "request-0");
    drop(observer);
    std::fs::remove_dir_all(path).unwrap();
}

#[test]
fn bounded_archive_scan_resumes_to_find_older_matches() {
    use std::{collections::BTreeMap, io::Write};
    let path = std::env::temp_dir().join(format!("flow-log-scan-{}", uuid::Uuid::new_v4()));
    let observer = Observability::disk(&path).unwrap();
    let archive = path.join("application.1.jsonl");
    let mut file = std::fs::File::create(&archive).unwrap();
    writeln!(
        file,
        "{}",
        json!({"sequence":1,"kind":"request","name":"needle"})
    )
    .unwrap();
    for sequence in 2..202 {
        writeln!(
            file,
            "{}",
            json!({"sequence":sequence,"kind":"request","padding":"x".repeat(32768)})
        )
        .unwrap();
    }
    drop(file);
    let mut filters = BTreeMap::from([("search".into(), "needle".into())]);
    let first = flow_runtime::log_store::query(&observer, &filters).unwrap();
    assert_eq!(first["scan_limited"], true);
    assert_eq!(first["entries"], json!([]));
    let cursor = first["next_cursor"].as_str().unwrap();
    std::fs::rename(&archive, path.join("application.2.jsonl")).unwrap();
    filters.insert("before".into(), cursor.into());
    let resumed = flow_runtime::log_store::query(&observer, &filters).unwrap();
    assert_eq!(resumed["entries"][0]["name"], "needle");
    assert_eq!(resumed["next_cursor"], Value::Null);
    std::fs::remove_file(path.join("application.2.jsonl")).unwrap();
    let expired = flow_runtime::log_store::query(&observer, &filters).unwrap();
    assert_eq!(expired["cursor_expired"], true);
    drop(observer);
    std::fs::remove_dir_all(path).unwrap();
}

#[test]
fn server_owned_logs_preserve_project_metrics_identity_and_read_only_legacy_history() {
    use std::collections::BTreeMap;
    let path = std::env::temp_dir().join(format!("flow-shared-logs-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(path.join("project-a")).unwrap();
    std::fs::write(
        path.join("project-a/application.1.jsonl"),
        "{\"sequence\":1,\"kind\":\"legacy\",\"time\":\"2020-01-01T00:00:00Z\"}\n",
    )
    .unwrap();
    let root = Observability::disk(path.join("server")).unwrap();
    root.identity("instance-a", "Original name");
    let first = Observability::scoped(path.join("project-a"), root.clone(), "a").unwrap();
    let second = Observability::scoped(path.join("project-b"), root.clone(), "b").unwrap();
    first.request("GET /health", 200, Duration::from_millis(1), "a-request");
    root.identity("instance-a", "New name");
    second.request("GET /health", 200, Duration::from_millis(1), "b-request");
    first.flush().unwrap();
    second.flush().unwrap();
    root.flush().unwrap();
    assert!(!path.join("project-a/application.jsonl").exists());
    assert!(!path.join("project-b/application.jsonl").exists());
    assert_eq!(first.snapshot()["endpoints"]["GET /health"]["count"], 1);
    assert_eq!(second.snapshot()["endpoints"]["GET /health"]["count"], 1);
    let system = flow_runtime::log_store::query(&root, &BTreeMap::new()).unwrap();
    assert_eq!(system["entries"].as_array().unwrap().len(), 2);
    assert_eq!(system["entries"][0]["server_name"], "New name");
    assert_eq!(system["entries"][1]["server_name"], "Original name");
    assert_eq!(system["entries"][1]["endpoint"], "GET /a/health");
    let project = flow_runtime::log_store::query(&first, &BTreeMap::new()).unwrap();
    assert_eq!(project["entries"].as_array().unwrap().len(), 2);
    assert_eq!(project["entries"][0]["project"], "a");
    assert_eq!(project["entries"][0]["endpoint"], "GET /health");
    assert_eq!(project["entries"][1]["kind"], "legacy");
    first.close();
    second.close();
    root.close();
    std::fs::remove_dir_all(path).unwrap();
}

#[test]
fn retention_converges_after_lowering_target_and_cursor_survives_immutable_rotation() {
    use flow_runtime::observability::LogPolicy;
    use std::collections::BTreeMap;
    let path = std::env::temp_dir().join(format!("flow-retention-{}", uuid::Uuid::new_v4()));
    let observer = Observability::disk(&path).unwrap();
    observer
        .policy(LogPolicy {
            target_bytes: 8192,
            chunk_bytes: 1024,
        })
        .unwrap();
    for n in 0..30 {
        observer.log(json!({"kind":"test","number":n,"padding":"x".repeat(256)}));
    }
    observer.flush().unwrap();
    let resources = observer.resources();
    assert!(resources["log_disk_bytes"].as_u64().unwrap() <= 8192);
    let oldest = resources["oldest_log_time"].as_str().unwrap();
    assert!(chrono::DateTime::parse_from_rfc3339(oldest).is_ok());
    let files = flow_runtime::observability::log_files(&path).unwrap();
    assert!(files.iter().any(|path| {
        path.file_name()
            .unwrap()
            .to_string_lossy()
            .contains("segment")
    }));
    let page =
        flow_runtime::log_store::query(&observer, &BTreeMap::from([("limit".into(), "5".into())]))
            .unwrap();
    let cursor = page["next_cursor"].as_str().unwrap().to_owned();
    observer
        .policy(LogPolicy {
            target_bytes: 2048,
            chunk_bytes: 512,
        })
        .unwrap();
    observer.flush().unwrap();
    assert!(observer.resources()["log_disk_bytes"].as_u64().unwrap() <= 2048);
    let next =
        flow_runtime::log_store::query(&observer, &BTreeMap::from([("before".into(), cursor)]))
            .unwrap();
    assert!(
        next["entries"]
            .as_array()
            .unwrap()
            .iter()
            .all(|event| event["number"].as_u64().unwrap() < 25)
    );
    observer.close();
    let restarted = Observability::disk(&path).unwrap();
    assert!(restarted.resources()["oldest_log_time"].is_string());
    restarted.close();
    std::fs::remove_dir_all(path).unwrap();
}

#[test]
fn histogram_counts_filtered_archived_logs_beyond_a_page_without_duplicate_memory_rows() {
    use std::collections::BTreeMap;
    let path = std::env::temp_dir().join(format!("flow-log-histogram-{}", uuid::Uuid::new_v4()));
    let root = Observability::disk(&path).unwrap();
    for n in 0..350 {
        root.log(json!({"kind":if n%2==0 {"request"} else {"io"}}));
    }
    root.flush().unwrap();
    let histogram = flow_runtime::log_store::query(
        &root,
        &BTreeMap::from([
            ("histogram".into(), "true".into()),
            ("kind".into(), "request".into()),
        ]),
    )
    .unwrap();
    assert_eq!(
        histogram["histogram"]
            .as_array()
            .unwrap()
            .iter()
            .map(|point| point["count"].as_u64().unwrap())
            .sum::<u64>(),
        175
    );
    assert_eq!(histogram["partial"], false);
    root.close();
    std::fs::remove_dir_all(path).unwrap();
}
