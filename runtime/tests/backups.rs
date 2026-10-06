use flow_runtime::{
    backups, engine::Config, observability::Observability, projects::Projects,
    server_state::ServerState,
};
use serde_json::{Value, json};
use std::{fs, path::PathBuf, sync::Arc};

const APP: &str = r#"
[type ItemID [id Item]]
[entity Item [field id ItemID] [field title String]]
[type ItemOutput [record [field id ItemID] [field title String]]]
[auth [anonymous Anonymous]] [transport HTTP [auth anonymous]]
[permissions Anonymous [Item [READ [when true]] [CREATE [when true]]]]
[mutate Add [input title String] [output ItemOutput] [http POST "/items"] [atomic]
[item [create Item [record [id [new ItemID]] [title $title]]]] [result item]]
[query Items [output [list ItemOutput [max 100]]] [http GET "/items"] [result [first [entities Item] 100]]]
"#;
struct Directory(PathBuf);
impl Directory {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("flow-backup-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn setup(directory: &Directory) -> Arc<Projects> {
    let root = directory.0.join("projects");
    fs::create_dir_all(root.join("one")).unwrap();
    fs::create_dir_all(root.join("two")).unwrap();
    fs::write(root.join("one/application.flow"), APP).unwrap();
    fs::write(root.join("two/application.flow"), APP).unwrap();
    let state = Arc::new(ServerState::open(&directory.0.join("server")).unwrap());
    state
        .enroll("Office backup", "fictional recovery password")
        .unwrap();
    state
        .set_secret("one", "private", "encrypted project credential")
        .unwrap();
    Projects::load_with_server(
        &root,
        &directory.0.join("data"),
        Config::default(),
        Observability::default(),
        Some(state),
    )
    .unwrap()
}

#[test]
fn full_backup_restores_projects_audit_sequences_and_encrypted_keys_on_a_fresh_runtime() {
    let directory = Directory::new();
    let projects = setup(&directory);
    let runtime = projects.get("one").unwrap();
    let row = runtime
        .lock()
        .unwrap()
        .execute("Add", json!({"title":"Saved"}), None)
        .unwrap();
    assert_eq!(row["id"], 1);
    runtime
        .lock()
        .unwrap()
        .execute("Add", json!({"title":"Deleted allocation"}), None)
        .unwrap();
    let deleted = runtime
        .lock()
        .unwrap()
        .admin_rows("Item", 0, 10, "", Some(2))
        .unwrap()["rows"][0]
        .clone();
    runtime
        .lock()
        .unwrap()
        .admin_write(
            "DELETE",
            "Item",
            Some(2),
            Value::Null,
            deleted["etag"].as_str(),
        )
        .unwrap();
    projects
        .get("two")
        .unwrap()
        .lock()
        .unwrap()
        .execute("Add", json!({"title":"Isolated"}), None)
        .unwrap();
    let captured_audit: i64 = rusqlite::Connection::open(directory.0.join("data/one.sqlite"))
        .unwrap()
        .query_row("SELECT count(*) FROM _flow_audit", [], |row| row.get(0))
        .unwrap();
    let saved = backups::create(&projects).unwrap();
    let id = saved["id"].as_str().unwrap();
    assert_eq!(
        backups::list(&projects).unwrap()["backups"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert!(backups::verify(&projects, id, "wrong password").is_err());
    assert_eq!(
        backups::verify(&projects, id, "fictional recovery password").unwrap()["verified"],
        true
    );
    let original = runtime
        .lock()
        .unwrap()
        .admin_rows("Item", 0, 10, "", Some(1))
        .unwrap()["rows"][0]
        .clone();
    runtime
        .lock()
        .unwrap()
        .admin_write(
            "UPDATE",
            "Item",
            Some(1),
            json!({"title":"Changed after backup"}),
            original["etag"].as_str(),
        )
        .unwrap();
    let target = directory.0.join("restored");
    backups::restore_fresh(
        &directory.0.join("backups").join(id),
        "fictional recovery password",
        &target,
    )
    .unwrap();
    let state = Arc::new(ServerState::open(&target.join("server")).unwrap());
    assert_eq!(
        state.secret("one", "private").unwrap().as_deref(),
        Some("encrypted project credential")
    );
    assert!(state.login("fictional recovery password").is_ok());
    let restored = Projects::load_with_server(
        &target.join("projects"),
        &target.join("databases"),
        Config::default(),
        Observability::default(),
        Some(state),
    )
    .unwrap();
    assert_eq!(restored.list().as_array().unwrap().len(), 2);
    assert_eq!(
        restored
            .get("one")
            .unwrap()
            .lock()
            .unwrap()
            .execute("Items", json!({}), None)
            .unwrap()[0]["title"],
        "Saved"
    );
    assert_eq!(
        restored
            .get("two")
            .unwrap()
            .lock()
            .unwrap()
            .execute("Items", json!({}), None)
            .unwrap()[0]["title"],
        "Isolated"
    );
    let restored_audit: i64 = rusqlite::Connection::open(target.join("databases/one.sqlite"))
        .unwrap()
        .query_row("SELECT count(*) FROM _flow_audit", [], |row| row.get(0))
        .unwrap();
    assert_eq!(captured_audit, restored_audit);
    assert_eq!(
        restored
            .get("one")
            .unwrap()
            .lock()
            .unwrap()
            .execute("Add", json!({"title":"Next"}), None)
            .unwrap()["id"],
        3
    );
    assert!(
        !directory
            .0
            .join("backups")
            .join(id)
            .join("server/server.keys")
            .exists()
    );
}

#[test]
fn corrupt_incomplete_or_wrong_password_backup_cannot_replace_working_data() {
    let directory = Directory::new();
    let projects = setup(&directory);
    let backup = backups::create(&projects).unwrap();
    let id = backup["id"].as_str().unwrap();
    let target = directory.0.join("working");
    fs::create_dir(&target).unwrap();
    fs::write(target.join("keep"), "Working data").unwrap();
    assert_eq!(
        backups::restore_fresh(
            &directory.0.join("backups").join(id),
            "fictional recovery password",
            &target
        )
        .unwrap_err()
        .code,
        "conflict"
    );
    assert_eq!(
        fs::read_to_string(target.join("keep")).unwrap(),
        "Working data"
    );
    let empty = directory.0.join("empty");
    assert!(
        backups::restore_fresh(&directory.0.join("backups").join(id), "incorrect", &empty).is_err()
    );
    assert!(!empty.exists());
    fs::write(
        directory
            .0
            .join("backups")
            .join(id)
            .join("databases/one.sqlite"),
        b"corrupted",
    )
    .unwrap();
    assert!(backups::verify(&projects, id, "fictional recovery password").is_err());
    assert!(
        projects
            .get("one")
            .unwrap()
            .lock()
            .unwrap()
            .execute("Items", json!({}), None)
            .is_ok()
    );
}

#[test]
fn online_restore_keeps_current_recovery_data_and_never_reuses_committed_ids() {
    let directory = Directory::new();
    let projects = setup(&directory);
    let runtime = projects.get("one").unwrap();
    runtime
        .lock()
        .unwrap()
        .execute("Add", json!({"title":"Before"}), None)
        .unwrap();
    let captured = backups::create(&projects).unwrap();
    runtime
        .lock()
        .unwrap()
        .execute("Add", json!({"title":"After"}), None)
        .unwrap();
    let restoration = backups::request_restore(
        &projects,
        captured["id"].as_str().unwrap(),
        "fictional recovery password",
    )
    .unwrap();
    assert_eq!(restoration["restoring"], true);
    assert!(
        backups::request_restore(
            &projects,
            captured["id"].as_str().unwrap(),
            "fictional recovery password"
        )
        .is_err()
    );
    runtime
        .lock()
        .unwrap()
        .execute("Add", json!({"title":"Last acknowledged write"}), None)
        .unwrap();
    backups::freeze_for_restore(&projects).unwrap();
    assert_eq!(
        runtime
            .lock()
            .unwrap()
            .execute("Add", json!({"title":"Must not write"}), None)
            .unwrap_err()
            .code,
        "unavailable"
    );
    let state_path = projects.server.as_ref().unwrap().directory().to_path_buf();
    projects.server.as_ref().unwrap().close().unwrap();
    assert!(backups::activate_pending(&state_path).unwrap());
    let state = Arc::new(ServerState::open(&state_path).unwrap());
    let restored = Projects::load_with_server(
        &directory.0.join("projects"),
        &directory.0.join("data"),
        Config::default(),
        Observability::default(),
        Some(state),
    )
    .unwrap();
    assert_eq!(
        restored
            .get("one")
            .unwrap()
            .lock()
            .unwrap()
            .execute("Items", json!({}), None)
            .unwrap(),
        json!([{"id":1,"title":"Before"}])
    );
    assert_eq!(
        restored
            .get("one")
            .unwrap()
            .lock()
            .unwrap()
            .execute("Add", json!({"title":"Fresh allocation"}), None)
            .unwrap()["id"],
        4
    );
    assert_eq!(
        backups::restore_status(&restored).unwrap()["request"],
        restoration["request"]
    );
    let recovery = fs::read_dir(&directory.0)
        .unwrap()
        .filter_map(|entry| entry.ok())
        .find(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with(".data-before-restore-")
        })
        .unwrap();
    let previous = rusqlite::Connection::open(recovery.path().join("one.sqlite")).unwrap();
    let kept: i64 = previous
        .query_row("SELECT count(*) FROM Item", [], |row| row.get(0))
        .unwrap();
    assert_eq!(kept, 3);
}

#[test]
fn interrupted_preparation_retains_working_generation_and_damaged_manifest_is_rejected() {
    let directory = Directory::new();
    let projects = setup(&directory);
    projects
        .get("one")
        .unwrap()
        .lock()
        .unwrap()
        .execute("Add", json!({"title":"Working"}), None)
        .unwrap();
    let captured = backups::create(&projects).unwrap();
    let id = captured["id"].as_str().unwrap();
    backups::request_restore(&projects, id, "fictional recovery password").unwrap();
    assert!(!backups::activate_pending(projects.server.as_ref().unwrap().directory()).unwrap());
    assert_eq!(
        projects
            .get("one")
            .unwrap()
            .lock()
            .unwrap()
            .execute("Items", json!({}), None)
            .unwrap()[0]["title"],
        "Working"
    );
    let path = directory.0.join("backups").join(id).join("manifest.json");
    let mut manifest: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    manifest["created_at"] = json!("tampered");
    fs::write(&path, serde_json::to_vec(&manifest).unwrap()).unwrap();
    assert!(backups::verify(&projects, id, "fictional recovery password").is_err());
}
