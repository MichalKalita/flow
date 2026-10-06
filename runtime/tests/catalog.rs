use flow_runtime::{
    catalog, engine::Config, observability::Observability, projects::Projects,
    server_state::ServerState,
};
use serde_json::json;
use std::{fs, sync::Arc};

#[test]
fn catalog_installs_isolated_contacts_once_with_normal_permissions_and_conflict_checks() {
    let directory = std::env::temp_dir().join(format!("flow-catalog-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(directory.join("projects")).unwrap();
    let server = Arc::new(ServerState::open(&directory.join("server")).unwrap());
    server
        .enroll("Contacts test", "fictional owner password")
        .unwrap();
    let projects = Projects::load_with_server(
        &directory.join("projects"),
        &directory.join("data"),
        Config::default(),
        Observability::default(),
        Some(server),
    )
    .unwrap();
    let request = uuid::Uuid::new_v4().to_string();
    let installed = catalog::install(&projects, "contacts", &request).unwrap();
    assert_eq!(installed["name"], "contacts");
    assert_eq!(
        catalog::install(&projects, "contacts", &request).unwrap()["name"],
        "contacts"
    );
    assert_eq!(projects.list().as_array().unwrap().len(), 1);
    let token = installed["authorization"].as_str().unwrap();
    let runtime = projects.get("contacts").unwrap();
    assert_eq!(
        runtime
            .lock()
            .unwrap()
            .execute("CreateContact", json!({"name":"Unauthorized"}), None)
            .unwrap_err()
            .code,
        "unauthenticated"
    );
    let first = runtime
        .lock()
        .unwrap()
        .execute(
            "CreateContact",
            json!({"name":"Ada","email":"ada@example.test"}),
            Some(token),
        )
        .unwrap();
    assert_eq!(first["id"], 1);
    assert_eq!(first["version"], 1);
    assert!(
        runtime
            .lock()
            .unwrap()
            .execute(
                "CreateContact",
                json!({"name":"x".repeat(201)}),
                Some(token)
            )
            .is_err()
    );
    let updated=runtime.lock().unwrap().execute("UpdateContact",json!({"contactId":1,"version":1,"name":"Ada Updated","email":"ada@example.test","phone":"","notes":""}),Some(token)).unwrap();
    assert_eq!(updated["version"], 2);
    assert_eq!(runtime.lock().unwrap().execute("UpdateContact",json!({"contactId":1,"version":1,"name":"Stale overwrite","email":"","phone":"","notes":""}),Some(token)).unwrap_err().code,"conflict");
    assert_eq!(
        runtime
            .lock()
            .unwrap()
            .execute(
                "DeleteContact",
                json!({"contactId":1,"version":1}),
                Some(token)
            )
            .unwrap_err()
            .code,
        "conflict"
    );
    for n in 0..55 {
        runtime
            .lock()
            .unwrap()
            .execute(
                "CreateContact",
                json!({"name":format!("Contact {n}")}),
                Some(token),
            )
            .unwrap();
    }
    let page = runtime
        .lock()
        .unwrap()
        .execute("Contacts", json!({}), Some(token))
        .unwrap();
    assert_eq!(page.as_array().unwrap().len(), 50);
    let after = page.as_array().unwrap().last().unwrap()["id"].clone();
    let next = runtime
        .lock()
        .unwrap()
        .execute("Contacts", json!({"after":after}), Some(token))
        .unwrap();
    assert_eq!(next.as_array().unwrap().len(), 6);
    let second =
        catalog::install(&projects, "contacts", &uuid::Uuid::new_v4().to_string()).unwrap();
    assert_eq!(second["name"], "contacts-2");
    let other = projects.get("contacts-2").unwrap();
    assert_eq!(
        other
            .lock()
            .unwrap()
            .execute("Contacts", json!({}), Some(token))
            .unwrap_err()
            .code,
        "unauthenticated"
    );
    assert_eq!(
        other
            .lock()
            .unwrap()
            .execute("Contacts", json!({}), second["authorization"].as_str())
            .unwrap(),
        json!([])
    );
    runtime
        .lock()
        .unwrap()
        .execute(
            "DeleteContact",
            json!({"contactId":1,"version":2}),
            Some(token),
        )
        .unwrap();
    let audit = rusqlite::Connection::open(directory.join("data/contacts.sqlite")).unwrap();
    let stale: i64 = audit
        .query_row(
            "SELECT count(*) FROM _flow_audit WHERE after_json LIKE '%Stale overwrite%'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(stale, 0);
    assert!(
        !fs::read_to_string(directory.join("projects/contacts/project.json"))
            .unwrap()
            .contains(token)
    );
    drop(audit);
    drop(runtime);
    drop(other);
    drop(projects);
    fs::remove_dir_all(directory).unwrap();
}
