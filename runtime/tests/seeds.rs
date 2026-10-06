use flow_runtime::{
    engine::{Config, Runtime},
    program::Program,
};
use serde_json::json;
use std::fs;
const BASE: &str = r#"
[type ItemID [id Item]]
[entity Item [field id ItemID] [field name String]]
[type ItemOutput [record [field id ItemID] [field name String]]]
[auth [anonymous Anonymous]] [transport HTTP [auth anonymous]]
[permissions Anonymous [Item [READ [when true]] [CREATE [when true]] [DELETE [when true]]]]
[query Items [output [list ItemOutput [max 100]]] [http GET "/items"] [result [entities Item]]]
[mutate Create [input name String] [output ItemOutput] [http POST "/items"] [atomic]
 [item [create Item [record [id [new ItemID]] [name $name]]]] [result item]]
[mutate Remove [input id ItemID] [output Bool] [http DELETE "/items/{id}"] [atomic]
 [removed [delete [entity $id]]] [result true]]
"#;

#[test]
fn test_seeds_are_opt_in_once_only_and_never_reset_committed_allocations() {
    let directory = std::env::temp_dir().join(format!("flow-seeds-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&directory).unwrap();
    let path = directory.join("application.sqlite");
    let source = format!(
        "{BASE}\n[seed Item [group production] [rows [record [id 1] [name \"Bootstrap\"]]]]\n[seed Item [group test] [rows [record [id 1000] [name \"Demo\"]]]]"
    );
    let mut runtime = Runtime::open(&source, path.to_str().unwrap(), Config::default()).unwrap();
    let rows = runtime.execute("Items", json!({}), None).unwrap();
    assert_eq!(rows, json!([{"id":1,"name":"Bootstrap"}]));
    assert_eq!(
        runtime
            .execute("Create", json!({"name":"Before demo"}), None)
            .unwrap()["id"],
        2
    );
    runtime
        .reload(
            &source,
            Config {
                test_seeds: true,
                ..Config::default()
            },
        )
        .unwrap();
    assert_eq!(
        runtime
            .execute("Items", json!({}), None)
            .unwrap()
            .as_array()
            .unwrap()
            .len(),
        3
    );
    assert_eq!(
        runtime
            .execute("Create", json!({"name":"After demo"}), None)
            .unwrap()["id"],
        1001
    );
    runtime.execute("Remove", json!({"id":1000}), None).unwrap();
    drop(runtime);
    let mut runtime = Runtime::open(
        &source,
        path.to_str().unwrap(),
        Config {
            test_seeds: true,
            ..Config::default()
        },
    )
    .unwrap();
    assert!(
        runtime
            .execute("Items", json!({}), None)
            .unwrap()
            .as_array()
            .unwrap()
            .iter()
            .all(|row| row["id"] != 1000)
    );
    runtime.reload(&source, Config::default()).unwrap();
    assert_eq!(
        runtime
            .execute("Create", json!({"name":"Production again"}), None)
            .unwrap()["id"],
        1002
    );
    let database = rusqlite::Connection::open(&path).unwrap();
    let insertions: i64 = database
        .query_row(
            "SELECT count(*) FROM _flow_audit WHERE action='INSERT' AND entity_id=1000",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(insertions, 1);
    let ledger: i64 = database
        .query_row(
            "SELECT count(*) FROM _flow_seed_rows WHERE entity='Item'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(ledger, 2);
    drop(database);
    drop(runtime);
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn duplicate_groups_and_conflicting_new_seeds_cannot_overwrite_data_or_leave_audit() {
    assert!(Program::compile(&format!("{BASE}[seed Item [rows [record [id 1] [name \"First\"]]]][seed Item [group test] [rows [record [id 1] [name \"Second\"]]]]")).is_err());
    for group in ["unknown", "production test"] {
        assert!(Program::compile(&format!("{BASE}[seed Item [group {group}] [rows]]")).is_err());
    }
    let directory =
        std::env::temp_dir().join(format!("flow-seed-conflict-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&directory).unwrap();
    let path = directory.join("application.sqlite");
    let mut runtime = Runtime::open(BASE, path.to_str().unwrap(), Config::default()).unwrap();
    runtime
        .execute("Create", json!({"name":"User data"}), None)
        .unwrap();
    let candidate =
        format!("{BASE}[seed Item [rows [record [id 1] [name \"Unexpected replacement\"]]]]");
    assert_eq!(
        runtime
            .reload(&candidate, Config::default())
            .unwrap_err()
            .code,
        "configuration"
    );
    assert_eq!(
        runtime.execute("Items", json!({}), None).unwrap(),
        json!([{"id":1,"name":"User data"}])
    );
    let database = rusqlite::Connection::open(&path).unwrap();
    assert_eq!(
        database
            .query_row("SELECT count(*) FROM _flow_seed_rows", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        database
            .query_row("SELECT count(*) FROM _flow_audit", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        1
    );
    drop(database);
    drop(runtime);
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn hosted_seed_profiles_are_project_local_and_invalid_reload_retains_the_working_profile() {
    use flow_runtime::{observability::Observability, projects::Projects};
    let directory =
        std::env::temp_dir().join(format!("flow-seed-profiles-{}", uuid::Uuid::new_v4()));
    let source = format!("{BASE}[seed Item [group test] [rows [record [id 10] [name \"Demo\"]]]]");
    for project in ["production", "sandbox"] {
        fs::create_dir_all(directory.join("projects").join(project)).unwrap();
        fs::write(
            directory
                .join("projects")
                .join(project)
                .join("application.flow"),
            &source,
        )
        .unwrap();
    }
    fs::write(
        directory.join("projects/sandbox/project.json"),
        "{\"test_seeds\":true}",
    )
    .unwrap();
    let projects = Projects::load(
        &directory.join("projects"),
        &directory.join("data"),
        Config::default(),
        Observability::default(),
    )
    .unwrap();
    let production = projects.get("production").unwrap();
    let sandbox = projects.get("sandbox").unwrap();
    assert_eq!(
        production
            .lock()
            .unwrap()
            .execute("Items", json!({}), None)
            .unwrap(),
        json!([])
    );
    assert_eq!(
        sandbox
            .lock()
            .unwrap()
            .execute("Items", json!({}), None)
            .unwrap(),
        json!([{"id":10,"name":"Demo"}])
    );
    fs::write(
        directory.join("projects/sandbox/project.json"),
        "{\"test_seeds\":\"yes\"}",
    )
    .unwrap();
    projects.scan().unwrap();
    assert!(directory.join("projects/sandbox-error.txt").exists());
    assert_eq!(
        sandbox
            .lock()
            .unwrap()
            .execute("Items", json!({}), None)
            .unwrap(),
        json!([{"id":10,"name":"Demo"}])
    );
    production.lock().unwrap().observability.close();
    sandbox.lock().unwrap().observability.close();
    drop(production);
    drop(sandbox);
    drop(projects);
    fs::remove_dir_all(directory).unwrap();
}
