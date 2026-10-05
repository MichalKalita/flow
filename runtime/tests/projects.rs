use flow_runtime::{
    engine::{Config, Runtime},
    observability::Observability,
    program::Program,
    projects::Projects,
};
use serde_json::json;
use std::path::PathBuf;
const APP: &str = include_str!("../application.flow");
struct Directory(PathBuf);
impl Directory {
    fn new() -> Self {
        Self(std::env::temp_dir().join(format!("flow-project-test-{}", uuid::Uuid::new_v4())))
    }
    fn source(&self, name: &str, source: &str) {
        std::fs::create_dir_all(self.0.join("projects").join(name)).unwrap();
        std::fs::write(
            self.0.join("projects").join(name).join("application.flow"),
            source,
        )
        .unwrap();
    }
    fn load(&self) -> std::sync::Arc<Projects> {
        let mut config = Config::default();
        config.jwt_keys.insert(
            "user".into(),
            b"development-key-32-bytes-minimum-123456".to_vec(),
        );
        config.event_credentials.insert(
            "service:1".into(),
            "ApiKey automation-key-long-enough-123456789".into(),
        );
        Projects::load(
            &self.0.join("projects"),
            &self.0.join("data"),
            config,
            Observability::default(),
        )
        .unwrap()
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
#[test]
fn projects_are_isolated_and_failed_initial_load_does_not_stop_others() {
    let dir = Directory::new();
    dir.source("alpha", APP);
    dir.source("beta", APP);
    dir.source("broken", "[invalid");
    let projects = dir.load();
    assert_eq!(projects.active().len(), 2);
    assert!(dir.0.join("projects/broken-error.txt").exists());
    let alpha = projects.get("alpha").unwrap();
    alpha
        .lock()
        .unwrap()
        .admin_write(
            "CREATE",
            "Product",
            None,
            json!({"name":"Only alpha","price":1,"stock":1}),
            None,
        )
        .unwrap();
    assert_eq!(
        alpha
            .lock()
            .unwrap()
            .admin_rows("Product", 0, 50, "Only alpha", None)
            .unwrap()["rows"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        projects
            .get("beta")
            .unwrap()
            .lock()
            .unwrap()
            .admin_rows("Product", 0, 50, "Only alpha", None)
            .unwrap()["rows"],
        json!([])
    );
}
#[test]
fn reload_validates_schema_keeps_last_good_and_recovers_after_restart() {
    let dir = Directory::new();
    dir.source("demo", APP);
    let projects = dir.load();
    let runtime = projects.get("demo").unwrap();
    let initial = projects.list()[0]["generation"].as_u64().unwrap();
    let deleted = runtime
        .lock()
        .unwrap()
        .admin_rows("Product", 0, 1, "", Some(4))
        .unwrap()["rows"][0]
        .clone();
    runtime
        .lock()
        .unwrap()
        .admin_write(
            "DELETE",
            "Product",
            Some(4),
            serde_json::Value::Null,
            deleted["etag"].as_str(),
        )
        .unwrap();
    dir.source("demo", "[bad");
    projects.scan().unwrap();
    assert_eq!(projects.list()[0]["status"], "stale");
    assert_eq!(projects.list()[0]["generation"], initial);
    assert!(
        runtime
            .lock()
            .unwrap()
            .execute("Products", json!({}), None)
            .is_ok()
    );
    projects.flush().unwrap();
    drop(runtime);
    drop(projects);
    let projects = dir.load();
    assert_eq!(projects.list()[0]["status"], "stale");
    assert!(projects.get("demo").is_some());
    let additive = APP.replacen(
        "[entity Product\n",
        "[entity Product\n  [field note [optional String]]\n",
        1,
    );
    let failed = format!(
        "{additive}\n[type OrphanID [id Orphan]] [entity Orphan [field id OrphanID] [field product Product]] [seed Orphan [rows [record [id 1] [product 999]]]]\n"
    );
    dir.source("demo", &failed);
    projects.scan().unwrap();
    assert_eq!(projects.list()[0]["status"], "stale");
    let db = rusqlite::Connection::open(dir.0.join("data/demo.sqlite")).unwrap();
    let tables: i64 = db
        .query_row(
            "SELECT count(*) FROM sqlite_master WHERE type='table' AND name='Orphan'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(tables, 0);
    let columns: i64 = db
        .query_row(
            "SELECT count(*) FROM pragma_table_info('Product') WHERE name='note'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(columns, 0);
    drop(db);
    assert!(
        !projects
            .get("demo")
            .unwrap()
            .lock()
            .unwrap()
            .admin_tables()
            .as_array()
            .unwrap()
            .iter()
            .any(|t| t["name"] == "Orphan")
    );
    dir.source("demo", &additive);
    projects.scan().unwrap();
    assert_eq!(projects.list()[0]["status"], "running");
    assert_eq!(
        projects
            .get("demo")
            .unwrap()
            .lock()
            .unwrap()
            .admin_rows("Product", 0, 1, "", Some(4))
            .unwrap()["rows"],
        json!([])
    );
    assert!(!dir.0.join("projects/demo-error.txt").exists());
    let runtime = projects.get("demo").unwrap();
    let rows = runtime
        .lock()
        .unwrap()
        .admin_rows("Product", 0, 1, "", None)
        .unwrap();
    assert_eq!(rows["rows"][0]["record"]["note"], serde_json::Value::Null);
    let destructive = additive.replacen("[field note [optional String]]", "[field note String]", 1);
    dir.source("demo", &destructive);
    projects.scan().unwrap();
    assert_eq!(projects.list()[0]["status"], "stale");
    assert!(
        runtime
            .lock()
            .unwrap()
            .admin_tables()
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["name"] == "Product")
            .unwrap()["fields"]
            .as_array()
            .unwrap()
            .iter()
            .any(|f| f["name"] == "note" && f["type"].as_str().unwrap().starts_with("Optional"))
    );
    dir.source("demo", &additive);
    projects.scan().unwrap();
    projects.flush().unwrap();
    drop(runtime);
    drop(projects);
    // A compatible source changed while offline also reloads against the cached schema.
    dir.source("demo", &(additive + "\n# Updated while offline\n"));
    let projects = dir.load();
    assert_eq!(projects.list()[0]["status"], "running");
    projects.flush().unwrap();
    drop(projects);
    // Simulate an older file cache left behind after a committed schema reload.
    std::fs::write(
        dir.0.join("data/demo-active.json"),
        json!({"source":APP,"manifest":{}}).to_string(),
    )
    .unwrap();
    let projects = dir.load();
    assert_eq!(projects.list()[0]["status"], "running");
}
#[test]
fn admin_writes_validate_references_and_audit_atomically_with_conflict_detection() {
    let mut runtime = Runtime::open(APP, ":memory:", {
        let mut c = Config::default();
        c.jwt_keys.insert(
            "user".into(),
            b"development-key-32-bytes-minimum-123456".to_vec(),
        );
        c.event_credentials.insert(
            "service:1".into(),
            "ApiKey automation-key-long-enough-123456789".into(),
        );
        c
    })
    .unwrap();
    let row = runtime.admin_rows("Product", 0, 1, "", None).unwrap()["rows"][0].clone();
    let mut record = row["record"].clone();
    record["name"] = json!("Admin renamed");
    runtime
        .admin_write(
            "UPDATE",
            "Product",
            Some(1),
            record.clone(),
            row["etag"].as_str(),
        )
        .unwrap();
    assert_eq!(
        runtime
            .admin_write("UPDATE", "Product", Some(1), record, row["etag"].as_str())
            .unwrap_err()
            .code,
        "conflict"
    );
    let baseline = runtime.audit(0, 200).unwrap();
    assert!(
        baseline
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["operation"] == "admin.UPDATE"
                && r["transport"] == "admin"
                && r["after"]["name"] == "Admin renamed")
    );
    assert!(
        runtime
            .admin_write(
                "CREATE",
                "Product",
                None,
                json!({"name":"Bad","price":-1,"stock":1}),
                None
            )
            .is_err()
    );
    assert!(
        runtime
            .admin_write(
                "CREATE",
                "DeviceAccess",
                None,
                json!({"user":999,"device":1,"rights":["READ"]}),
                None
            )
            .is_err()
    );
    assert_eq!(runtime.audit(0, 200).unwrap(), baseline);
    let current = runtime.admin_rows("User", 0, 1, "", Some(1)).unwrap()["rows"][0].clone();
    assert!(
        runtime
            .admin_write(
                "DELETE",
                "User",
                Some(1),
                serde_json::Value::Null,
                current["etag"].as_str()
            )
            .is_err()
    );
    assert_eq!(runtime.audit(0, 200).unwrap(), baseline);
    assert!(runtime.admin_rows("_flow_audit", 0, 50, "", None).is_err());
    let created = runtime
        .admin_write(
            "CREATE",
            "Product",
            None,
            json!({"name":"Disposable","price":1,"stock":1}),
            None,
        )
        .unwrap();
    let id = created["id"].as_i64().unwrap();
    assert!(id > 4);
    let current = runtime.admin_rows("Product", 0, 1, "", Some(id)).unwrap()["rows"][0].clone();
    runtime
        .admin_write(
            "DELETE",
            "Product",
            Some(id),
            serde_json::Value::Null,
            current["etag"].as_str(),
        )
        .unwrap();
    assert_eq!(
        runtime
            .admin_rows("Product", 0, 50, "Disposable", None)
            .unwrap()["rows"],
        json!([])
    );
}
#[test]
fn repository_hosted_projects_compile_and_load() {
    let source = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../projects");
    let dir = Directory::new();
    let mut names = Vec::new();
    for entry in std::fs::read_dir(&source).unwrap() {
        let entry = entry.unwrap();
        if !entry.file_type().unwrap().is_dir() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        let program = entry.path().join("application.flow");
        if !program.exists() {
            continue;
        }
        Program::compile(&std::fs::read_to_string(&program).unwrap()).unwrap();
        dir.source(&name, &std::fs::read_to_string(&program).unwrap());
        names.push(name);
    }
    names.sort();
    assert!(names.contains(&"demo".into()));
    assert!(names.contains(&"bookstore".into()));
    let projects = dir.load();
    assert_eq!(projects.active().len(), names.len());
    for name in &names {
        let item = projects
            .list()
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["name"] == *name)
            .cloned()
            .unwrap();
        assert_eq!(item["status"], "running", "{name}");
        assert!(projects.get(name).is_some());
    }
}
