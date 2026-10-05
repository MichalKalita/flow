use flow_runtime::{
    engine::{Config, Runtime},
    program::Program,
};
use serde_json::json;
use sha2::{Digest, Sha256};
fn source(rules: &str, operations: &str) -> String {
    let hash = format!(
        "{:x}",
        Sha256::digest(b"secret-api-key-32-bytes-minimum-123456")
    );
    format!(
        r#"
[type UserID [id User]]
[type DocumentID [id Document]]
[type FolderID [id Folder]]
[type Amount [decimal [range 0.01 1000000] [scale 2]]]
[entity User [field id UserID] [field hash String [unique]] [field roles [list String [max 2]]]]
[entity Document [field id DocumentID] [field owner User] [field title String] [field secret String] [field salary Amount] [field contact UserID]]
[entity Folder [field id FolderID] [field owner [optional User]] [field parent [optional Folder]]]
[type DocOutput [record [field id DocumentID] [field title String]]]
[type SecretOutput [record [field secret String]]]
[type RoleOutput [record [field roles [list String [max 2]]]]]
[seed User [rows [record [id "u1"] [hash "{hash}"] [roles [list]]]]]
[seed Document [rows [record [id "d1"] [owner "u1"] [title "Original"] [secret "private"] [salary 2500] [contact "u1"]]]]
[seed Folder [rows [record [id "owned"] [owner "u1"]] [record [id "child"] [parent "owned"]] [record [id "orphan"]]]]
[auth [key User [apiKey] [entity [eq User.hash credential.hash]]] [anonymous Anonymous]]
[transport HTTP [auth key anonymous]]
{rules}
{operations}
"#
    )
}
const AUTH: &str = "ApiKey secret-api-key-32-bytes-minimum-123456";
const READ: &str = r#"[query Docs [output [list DocOutput [max 10]]] [http GET "/docs"] [result [first [entities Document] 10]]]"#;
#[test]
fn api_key_identity_and_action_inheritance() {
    let rules =
        r#"[permissions User [Document [UPDATE [includes READ] [when [eq target.owner actor]]]]]"#;
    let mut r = Runtime::open(&source(rules, READ), ":memory:", Config::default()).unwrap();
    assert_eq!(r.execute("Docs", json!({}), None).unwrap(), json!([]));
    assert_eq!(
        r.execute("Docs", json!({}), Some(AUTH))
            .unwrap()
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        r.execute(
            "Docs",
            json!({}),
            Some("ApiKey wrong-key-long-enough-12345678901234567890")
        )
        .unwrap_err()
        .code,
        "unauthenticated"
    );
}
#[test]
fn field_permissions_are_additional_and_permission_dependencies_private() {
    let rules = r#"[permissions User [Document [READ [when [eq target.secret "private"]]]] [Document.secret [READ [when false]]]]"#;
    let op = r#"[query Secrets [input id DocumentID] [output SecretOutput] [http GET "/docs/{id}"] [result [entity $id]]]"#;
    let mut r = Runtime::open(
        &source(rules, &format!("{READ}\n{op}")),
        ":memory:",
        Config::default(),
    )
    .unwrap();
    assert_eq!(
        r.execute("Docs", json!({}), Some(AUTH)).unwrap()[0]["title"],
        "Original"
    );
    assert_eq!(
        r.execute("Secrets", json!({"id":"d1"}), Some(AUTH))
            .unwrap_err()
            .code,
        "not_found"
    );
}
#[test]
fn can_cannot_evaluate_business_bindings_with_permission_privileges() {
    let rules = r#"[permissions User [User [READ [when true]]] [Document [READ [when true]]] [Document.contact [READ [when false]]]]"#;
    let op = r#"[query Leak [input id DocumentID] [output UserID] [http GET "/leak/{id}"] [document [entity $id]] [aCheck [can READ [entity protected]]] [protected document.contact] [result protected]]"#;
    let mut r = Runtime::open(&source(rules, op), ":memory:", Config::default()).unwrap();
    assert_eq!(
        r.execute("Leak", json!({"id":"d1"}), Some(AUTH))
            .unwrap_err()
            .code,
        "not_found"
    );
}
#[test]
fn actor_cannot_use_new_role_to_authorize_another_change() {
    let rules = r#"[permissions User [User [READ [when [eq actor target]]] [UPDATE [when [eq actor target]]]] [Document [UPDATE [when [contains actor.roles ADMIN]]]]]"#;
    let op = r#"[mutate Elevate [input id UserID] [input docId DocumentID] [output Bool] [http POST "/elevate"] [atomic] [user [entity $id]] [doc [entity $docId]] [aRole [set user.roles [list ADMIN]]] [bSecret [set doc.secret "leaked"]] [result true]]
[query Roles [input id UserID] [output RoleOutput] [http GET "/roles/{id}"] [result [entity $id]]]"#;
    let mut r = Runtime::open(&source(rules, op), ":memory:", Config::default()).unwrap();
    assert_eq!(
        r.execute("Elevate", json!({"id":"u1","docId":"d1"}), Some(AUTH))
            .unwrap_err()
            .code,
        "forbidden"
    );
    assert_eq!(
        r.execute("Roles", json!({"id":"u1"}), Some(AUTH)).unwrap(),
        json!({"roles":[]})
    );
}
#[test]
fn recursive_relation_permissions_fail_closed_at_null_parent() {
    let rules = r#"[permissions User [Folder [READ [when [or [eq target.owner actor] [can READ target.parent]]]]]]"#;
    let op = r#"[type FolderOutput [record [field id FolderID]]]
[query Folders [output [list FolderOutput [max 10]]] [http GET "/folders"] [result [first [entities Folder] 10]]]"#;
    let mut r = Runtime::open(&source(rules, op), ":memory:", Config::default()).unwrap();
    let ids = r.execute("Folders", json!({}), Some(AUTH)).unwrap();
    assert_eq!(ids, json!([{"id":"child"},{"id":"owned"}]));
}
#[test]
fn write_fields_require_their_own_grants() {
    let rules = r#"[permissions User [Document [UPDATE [when [eq actor target.owner]]]] [Document.secret [UPDATE [when false]]]]"#;
    let op = r#"[mutate Change [input id DocumentID] [output Bool] [http POST "/change/{id}"] [atomic] [document [entity $id]] [changed [set document.secret "changed"]] [result true]]"#;
    let mut r = Runtime::open(&source(rules, op), ":memory:", Config::default()).unwrap();
    assert_eq!(
        r.execute("Change", json!({"id":"d1"}), Some(AUTH))
            .unwrap_err()
            .code,
        "forbidden"
    );
}
#[test]
fn deny_unknown_or_unsafe_programs() {
    for rules in [
        r#"[permissions User [Document [READ [includes UPDATE] [when true]] [UPDATE [includes READ] [when true]]]]"#,
        r#"[permissions User [Document [READ [when [not [can READ target]]]]]]"#,
        r#"[permissions User [Document [READ [when [eq before.owner actor]]]]]"#,
        r#"[permissions User [Document [DENY [when true]]]]"#,
        r#"[permissions User [Document.missing [READ [when true]]]]"#,
    ] {
        assert!(Program::compile(&source(rules, READ)).is_err());
    }
    let op = r#"[query Write [input id DocumentID] [output Bool] [http GET "/write"] [document [entity $id]] [hidden [set document.secret "x"]] [result true]]"#;
    assert!(Program::compile(&source("", op)).is_err());
}

#[test]
fn delete_honors_entity_and_field_rights() {
    let rules =
        r#"[permissions User [Document [DELETE [includes READ] [when [eq actor target.owner]]]]]"#;
    let op = r#"[mutate Remove [input id DocumentID] [output Bool] [http DELETE "/docs/{id}"] [atomic] [removed [delete [entity $id]]] [result true]]"#;
    let mut runtime = Runtime::open(
        &source(rules, &format!("{READ}\n{op}")),
        ":memory:",
        Config::default(),
    )
    .unwrap();
    assert_eq!(
        runtime
            .execute("Remove", json!({"id":"d1"}), Some(AUTH))
            .unwrap(),
        true
    );
    assert_eq!(
        runtime.execute("Docs", json!({}), Some(AUTH)).unwrap(),
        json!([])
    );
    let restricted = format!("{rules}\n[permissions User [Document.secret [DELETE [when false]]]]");
    let mut runtime = Runtime::open(
        &source(&restricted, &format!("{READ}\n{op}")),
        ":memory:",
        Config::default(),
    )
    .unwrap();
    assert_eq!(
        runtime
            .execute("Remove", json!({"id":"d1"}), Some(AUTH))
            .unwrap_err()
            .code,
        "forbidden"
    );
    assert_eq!(
        runtime
            .execute("Docs", json!({}), Some(AUTH))
            .unwrap()
            .as_array()
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn newly_created_records_can_be_updated_before_commit() {
    let rules = r#"[permissions User [Document [CREATE [when [and [eq actor after.owner] [eq after.title "Updated"]]]] [READ [when [eq actor target.owner]]]]]"#;
    let op = r#"[mutate Add [input id DocumentID] [input userId UserID] [output Bool] [http POST "/docs"] [atomic]
    [added [create Document [record [id $id] [owner [entity $userId]] [title "Initial"] [secret "private"] [salary 2500] [contact $userId]]]]
    [updated [set added.title "Updated"]] [result true]]"#;
    let mut runtime = Runtime::open(
        &source(rules, &format!("{READ}\n{op}")),
        ":memory:",
        Config::default(),
    )
    .unwrap();
    assert_eq!(
        runtime
            .execute("Add", json!({"id":"d2","userId":"u1"}), Some(AUTH))
            .unwrap(),
        true
    );
    assert_eq!(
        runtime.execute("Docs", json!({}), Some(AUTH)).unwrap()[1]["title"],
        "Updated"
    );
}
