use flow_runtime::{
    engine::{Config, Runtime},
    program::Program,
};
use serde_json::json;
use std::fs;
const BASE: &str = r#"
[schema 1]
[type ItemID [id Item]]
[entity Item [field id ItemID] [field title String [unique]] [field parent [optional Item]]]
[type ItemOutput [record [field id ItemID] [field title String] [field parent [optional ItemID]]]]
[auth [anonymous Anonymous]] [transport HTTP [auth anonymous]]
[permissions Anonymous [Item [READ [when true]] [CREATE [when true]] [DELETE [when true]]]]
[query Items [output [list ItemOutput [max 100]]] [http GET "/items"] [result [entities Item]]]
[mutate Add [input title String] [input parent [optional ItemID]] [output ItemOutput] [http POST "/items"] [atomic]
 [item [create Item [record [id [new ItemID]] [title $title] [parent $parent]]]] [result item]]
[mutate Remove [input id ItemID] [output Bool] [http DELETE "/items/{id}"] [atomic]
 [removed [delete [entity $id]]] [result true]]
"#;
fn candidate(extra: &str) -> String {
    format!(
        "{}\n[migration RenameItem [from 1] [to 2] [rename Item title name] [rename Item parent ancestor] {extra}]",
        BASE.replace("[schema 1]", "[schema 2]")
            .replace("title", "name")
            .replace("parent", "ancestor")
    )
}

#[test]
fn rename_migrations_commit_data_program_history_audit_and_sequences_as_one_generation() {
    let directory = std::env::temp_dir().join(format!("flow-migrations-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&directory).unwrap();
    let path = directory.join("application.sqlite");
    let mut runtime = Runtime::open(BASE, path.to_str().unwrap(), Config::default()).unwrap();
    runtime
        .execute("Add", json!({"title":"Ada"}), None)
        .unwrap();
    runtime
        .execute("Add", json!({"title":"Deleted","parent":1}), None)
        .unwrap();
    runtime.execute("Remove", json!({"id":2}), None).unwrap();
    let database = rusqlite::Connection::open(&path).unwrap();
    assert!(
        runtime
            .reload(&candidate("[rename Item missing name]"), Config::default())
            .is_err()
    );
    assert_eq!(
        runtime.execute("Items", json!({}), None).unwrap(),
        json!([{"id":1,"title":"Ada","parent":null}])
    );
    assert_eq!(
        database
            .query_row(
                "SELECT count(*) FROM pragma_table_info('Item') WHERE name IN ('name','ancestor')",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
    assert_eq!(
        database
            .query_row("SELECT count(*) FROM _flow_migrations", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        database
            .query_row(
                "SELECT version FROM _flow_schema_version WHERE id=1",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        1
    );
    let source = candidate("");
    runtime.reload(&source, Config::default()).unwrap();
    assert_eq!(
        runtime.execute("Items", json!({}), None).unwrap(),
        json!([{"id":1,"name":"Ada","ancestor":null}])
    );
    assert_eq!(runtime.audit(0, 100).unwrap()[2]["after"]["title"], "Ada");
    assert_eq!(
        database
            .query_row("SELECT count(*) FROM _flow_audit", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        3
    );
    assert_eq!(
        database
            .query_row(
                "SELECT version FROM _flow_schema_version WHERE id=1",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        2
    );
    assert_eq!(
        database
            .query_row(
                "SELECT source FROM _flow_active_program WHERE id=1",
                [],
                |row| row.get::<_, String>(0)
            )
            .unwrap(),
        source
    );
    assert_eq!(
        runtime
            .execute("Add", json!({"name":"Next","ancestor":1}), None)
            .unwrap()["id"],
        3
    );
    assert!(runtime.execute("Add", json!({"name":"Ada"}), None).is_err());
    assert!(
        runtime
            .execute("Add", json!({"name":"Orphan","ancestor":999}), None)
            .is_err()
    );
    assert!(runtime.reload(BASE, Config::default()).is_err());
    assert!(
        runtime
            .reload(
                &source.replace("RenameItem", "ChangedHistory"),
                Config::default()
            )
            .is_err()
    );
    drop(runtime);
    let mut runtime = Runtime::open(&source, path.to_str().unwrap(), Config::default()).unwrap();
    assert_eq!(
        runtime
            .execute("Add", json!({"name":"After restart"}), None)
            .unwrap()["id"],
        4
    );
    assert_eq!(
        database
            .query_row("SELECT count(*) FROM _flow_migrations", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        1
    );
    drop(database);
    drop(runtime);
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn fresh_install_registers_migration_history_without_replaying_old_columns() {
    let source = candidate("");
    let mut runtime = Runtime::open(&source, ":memory:", Config::default()).unwrap();
    assert_eq!(
        runtime
            .execute("Add", json!({"name":"Fresh"}), None)
            .unwrap()["id"],
        1
    );
    for declaration in [
        "[schema 0]",
        "[schema 1.5]",
        "[schema 2] [schema 2]",
        "[schema 2] [migration Unsafe [from 1] [to 2] [rename Item id name]]",
        "[schema 3] [migration Gap [from 1] [to 2]]",
    ] {
        assert!(
            Program::compile(&format!("{} {declaration}", BASE.replace("[schema 1]", ""))).is_err()
        );
    }
}

#[test]
fn hosted_restart_recovers_migrated_generation_even_with_stale_cache_and_invalid_candidate() {
    use flow_runtime::{observability::Observability, projects::Projects};
    let directory =
        std::env::temp_dir().join(format!("flow-migration-recovery-{}", uuid::Uuid::new_v4()));
    let root = directory.join("projects");
    let data = directory.join("data");
    fs::create_dir_all(root.join("one")).unwrap();
    fs::write(root.join("one/application.flow"), BASE).unwrap();
    let projects =
        Projects::load(&root, &data, Config::default(), Observability::default()).unwrap();
    let runtime = projects.get("one").unwrap();
    runtime
        .lock()
        .unwrap()
        .execute("Add", json!({"title":"Durable"}), None)
        .unwrap();
    fs::write(root.join("one/application.flow"), candidate("")).unwrap();
    projects.scan().unwrap();
    assert_eq!(
        runtime.lock().unwrap().migration_status().unwrap()["schema_version"],
        2
    );
    fs::write(
        data.join("one-active.json"),
        json!({"source":BASE,"manifest":{}}).to_string(),
    )
    .unwrap();
    fs::write(root.join("one/application.flow"), "[broken").unwrap();
    runtime.lock().unwrap().observability.close();
    drop(runtime);
    drop(projects);
    let restarted =
        Projects::load(&root, &data, Config::default(), Observability::default()).unwrap();
    let runtime = restarted.get("one").unwrap();
    assert_eq!(restarted.list()[0]["status"], "stale");
    assert_eq!(
        runtime
            .lock()
            .unwrap()
            .execute("Items", json!({}), None)
            .unwrap(),
        json!([{"id":1,"name":"Durable","ancestor":null}])
    );
    assert_eq!(
        runtime.lock().unwrap().migration_status().unwrap()["migrations"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        runtime
            .lock()
            .unwrap()
            .execute("Add", json!({"name":"After recovery","ancestor":1}), None)
            .unwrap()["id"],
        2
    );
    runtime.lock().unwrap().observability.close();
    drop(runtime);
    drop(restarted);
    fs::remove_dir_all(directory).unwrap();
}
