use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use flow_runtime::{
    engine::{Config, Runtime},
    program::Program,
    syntax::parse,
    value::{decimal, number},
};
use serde_json::{Value, json};
use std::collections::BTreeMap;
const APP: &str = include_str!("../application.flow");
const KEY: &[u8] = b"development-key-32-bytes-minimum-123456";
fn config() -> Config {
    Config {
        jwt_keys: BTreeMap::from([("user".into(), KEY.to_vec())]),
        event_credentials: BTreeMap::from([(
            "service:1".into(),
            "ApiKey automation-key-long-enough-123456789".into(),
        )]),
    }
}
fn token(subject: &str) -> String {
    token_claims(
        json!({"iss":"https://identity.example.com","aud":"application","sub":subject,"exp":chrono::Utc::now().timestamp()+3600}),
        "HS256",
        KEY,
    )
}
fn token_claims(claims: Value, algorithm: &str, key: &[u8]) -> String {
    let header = URL_SAFE_NO_PAD.encode(json!({"alg":algorithm}).to_string());
    let payload = URL_SAFE_NO_PAD.encode(claims.to_string());
    let unsigned = format!("{header}.{payload}");
    let signature = ring::hmac::sign(
        &ring::hmac::Key::new(ring::hmac::HMAC_SHA256, key),
        unsigned.as_bytes(),
    );
    format!(
        "Bearer {unsigned}.{}",
        URL_SAFE_NO_PAD.encode(signature.as_ref())
    )
}
fn runtime() -> Runtime {
    Runtime::open(APP, ":memory:", config()).unwrap()
}
fn order(user: i64, qty: u32) -> Value {
    json!({"userId":user,"items":[{"productId":1,"quantity":qty}],"paymentMethod":"CARD"})
}
#[test]
fn parser_and_exact_numbers() {
    assert!(parse("[a [b] \"c\\uD83D\\uDE00\" 0.01] # comment").is_ok());
    for source in [
        "[a",
        "]",
        "[a\"b\"]",
        "[01]",
        "[+1]",
        "[\"a\"b]",
        "[\"\\uD800\"]",
    ] {
        assert!(parse(source).is_err(), "{source}");
    }
    let n = number("999999999999999999999999999999.01").unwrap() + number("0.02").unwrap();
    assert_eq!(decimal(&n).unwrap(), "999999999999999999999999999999.03");
}
#[test]
fn source_compiles_and_public_projection_is_narrow() {
    assert_eq!(
        Program::compile(APP)
            .unwrap()
            .operations
            .iter()
            .filter(|o| o.method != "WS" && o.event.is_none())
            .count(),
        10
    );
    let mut r = runtime();
    let (products, trace) = r.execute_traced("Products", json!({}), None).unwrap();
    assert_eq!(products.as_array().unwrap().len(), 4);
    assert_eq!(products[0], json!({"id":1,"name":"Studio sluchátka"}));
    assert!(
        trace
            .iter()
            .all(|sql| !sql.contains('*') && !sql.contains("price") && !sql.contains("stock"))
    );
    assert_eq!(r.execute("Users", json!({}), None).unwrap(), json!([]));
}
#[test]
fn credentials_verified_and_roles_not_client_controlled() {
    let mut r = runtime();
    let auth = token("idp:u1");
    assert_eq!(
        r.execute("Users", json!({}), Some(&auth)).unwrap(),
        json!([{"id":1,"name":"Petra Nováková"}])
    );
    let invalid = token_claims(
        json!({"iss":"https://identity.example.com","aud":"application","sub":"idp:u1","exp":chrono::Utc::now().timestamp()+3600}),
        "HS256",
        b"different-key",
    );
    assert_eq!(
        r.execute("Users", json!({}), Some(&invalid))
            .unwrap_err()
            .code,
        "unauthenticated"
    );
    for claims in [
        json!({"iss":"bad","aud":"application","sub":"idp:u1","exp":9999999999i64}),
        json!({"iss":"https://identity.example.com","aud":"bad","sub":"idp:u1","exp":9999999999i64}),
        json!({"iss":"https://identity.example.com","aud":"application","sub":"idp:u1","exp":1}),
        json!({"iss":"https://identity.example.com","aud":"application","sub":"missing","exp":9999999999i64}),
    ] {
        let auth = token_claims(claims, "HS256", KEY);
        assert_eq!(
            r.execute("Users", json!({}), Some(&auth)).unwrap_err().code,
            "unauthenticated"
        );
    }
    assert_eq!(
        r.execute("Users", json!({"actor":1}), Some(&auth))
            .unwrap_err()
            .code,
        "invalid_input"
    );
}
#[test]
fn order_transaction_and_owned_reads() {
    let mut r = runtime();
    let auth = token("idp:u1");
    let receipt = r.execute("CreateOrder", order(1, 2), Some(&auth)).unwrap();
    assert_eq!(receipt["order"]["total"], 4980);
    assert_eq!(receipt["order"]["items"][0]["quantity"], 2);
    let id = receipt["order"]["id"].as_i64().unwrap();
    assert_eq!(
        r.execute("Order", json!({"orderId":id}), Some(&auth))
            .unwrap(),
        receipt["order"]
    );
    for credential in [None, Some(token("idp:u2"))] {
        assert_eq!(
            r.execute("Order", json!({"orderId":id}), credential.as_deref())
                .unwrap_err()
                .code,
            "not_found"
        );
    }
    assert_eq!(
        r.execute("UserOrders", json!({"userId":1}), Some(&auth))
            .unwrap()
            .as_array()
            .unwrap()
            .len(),
        1
    );
}
#[test]
fn failed_permissions_and_stock_validation_roll_back() {
    let mut r = runtime();
    let auth = token("idp:u1");
    assert_eq!(
        r.execute("CreateOrder", order(2, 2), Some(&auth))
            .unwrap_err()
            .code,
        "not_found"
    );
    let receipt = r.execute("CreateOrder", order(1, 12), Some(&auth)).unwrap();
    assert_eq!(receipt["order"]["items"][0]["quantity"], 12);
    assert!(r.execute("CreateOrder", order(1, 1), Some(&auth)).is_err());
    assert_eq!(
        r.execute("UserOrders", json!({"userId":1}), Some(&auth))
            .unwrap()
            .as_array()
            .unwrap()
            .len(),
        1
    );
}
#[test]
fn duplicate_cart_rows_group_before_stock_update() {
    let mut r = runtime();
    let auth = token("idp:u1");
    let receipt = r
        .execute(
            "CreateOrder",
            json!({
                "userId": 1,
                "items": [
                    {"productId": 1, "quantity": 1},
                    {"productId": 1, "quantity": 2}
                ],
                "paymentMethod": "BANK"
            }),
            Some(&auth),
        )
        .unwrap();
    assert_eq!(receipt["order"]["total"], 7470);
    assert_eq!(receipt["order"]["items"].as_array().unwrap().len(), 1);
    assert_eq!(receipt["order"]["items"][0]["quantity"], 3);
}
#[test]
fn device_permissions_apply_to_writes() {
    let mut r = runtime();
    let auth = token("idp:u1");
    assert!(
        r.execute(
            "SendCommand",
            json!({"deviceId":1,"action":"START"}),
            Some(&auth)
        )
        .is_ok()
    );
    assert_eq!(
        r.execute(
            "SendCommand",
            json!({"deviceId":2,"action":"STOP"}),
            Some(&auth)
        )
        .unwrap_err()
        .code,
        "forbidden"
    );
    assert_eq!(
        r.execute("Device", json!({"deviceId":2}), Some(&auth))
            .unwrap_err()
            .code,
        "not_found"
    );
    assert_eq!(
        r.execute("Device", json!({"deviceId":1}), Some(&auth))
            .unwrap()["positions"],
        json!([])
    );
}
#[test]
fn strict_inputs_and_prices() {
    let mut r = runtime();
    let auth = token("idp:u1");
    for input in [
        json!({"userId":1,"items":[],"paymentMethod":"CARD"}),
        json!({"userId":1,"items":[{"productId":1,"quantity":1,"hidden":true}],"paymentMethod":"CARD"}),
        json!({"userId":1,"items":[{"productId":1,"quantity":0}],"paymentMethod":"CARD"}),
        json!({"userId":1,"items":[{"productId":1,"quantity":1.01}],"paymentMethod":"CARD"}),
    ] {
        assert_eq!(
            r.execute("CreateOrder", input, Some(&auth))
                .unwrap_err()
                .code,
            "invalid_input"
        );
    }
    assert!(Program::compile(&APP.replace("[price 2490.00]", "[price 0]")).is_err());
    assert!(Program::compile(&APP.replace("[price 2490.00]", "[price 1.001]")).is_err());
}
#[test]
fn sqlite_survives_restart_and_rejects_implicit_schema_change() {
    let path = std::env::temp_dir().join(format!("flow-{}.sqlite", uuid::Uuid::new_v4()));
    let path_str = path.to_str().unwrap();
    let auth = token("idp:u1");
    let id;
    {
        let mut r = Runtime::open(APP, path_str, config()).unwrap();
        id = r.execute("CreateOrder", order(1, 1), Some(&auth)).unwrap()["order"]["id"].clone();
    }
    {
        let mut r = Runtime::open(APP, path_str, config()).unwrap();
        assert_eq!(
            r.execute("Order", json!({"orderId":id}), Some(&auth))
                .unwrap()["total"],
            2490
        );
    }
    assert!(Runtime::open(&format!("{APP}\n# changed"), path_str, config()).is_err());
    let _ = std::fs::remove_file(path);
}

#[test]
fn extreme_exponents_and_constants_return_errors_without_panicking() {
    assert!(number("1e-2147483648").is_err());
    assert!(
        Program::compile("[type Huge [integer [range 0 [pow [pow [pow 10 100] 100] 100]]]]")
            .is_err()
    );
}

fn png() -> String {
    use base64::Engine as _;
    let image = image::DynamicImage::new_rgb8(16, 12);
    let mut bytes = std::io::Cursor::new(vec![]);
    image.write_to(&mut bytes, image::ImageFormat::Png).unwrap();
    base64::engine::general_purpose::STANDARD.encode(bytes.into_inner())
}
#[test]
fn photo_plugins_store_real_png_atomically_and_enforce_permissions() {
    let mut runtime = runtime();
    let admin = token("idp:catalog-admin");
    let user = token("idp:u1");
    let input = json!({"productId":1,"photo":png(),"width":8,"height":6});
    assert_eq!(
        runtime
            .execute("UploadPhoto", input.clone(), Some(&user))
            .unwrap_err()
            .code,
        "forbidden"
    );
    assert_eq!(
        runtime
            .execute("ProductPhotos", json!({"productId":1}), Some(&user))
            .unwrap(),
        json!([])
    );
    let receipt = runtime.execute("UploadPhoto", input, Some(&admin)).unwrap();
    assert_eq!(receipt["photo"]["width"], 8);
    assert_eq!(receipt["photo"]["height"], 6);
    let id = receipt["file"]["id"].as_i64().unwrap();
    let bytes = runtime.file_bytes(&id, Some(&user)).unwrap();
    let audit = runtime.audit(0, 200).unwrap();
    let blob = audit
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["entity"] == "_flow_blobs" && v["entity_id"] == id)
        .unwrap();
    assert_eq!(blob["after"]["bytes"], bytes.len());
    assert_eq!(blob["actor"]["id"], 4);
    let image = image::load_from_memory(&bytes).unwrap();
    assert_eq!((image.width(), image.height()), (8, 6));
    assert_eq!(runtime.file_bytes(&id, None).unwrap_err().code, "not_found");
    assert_eq!(
        runtime
            .execute("ProductPhotos", json!({"productId":1}), Some(&user))
            .unwrap()
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        runtime
            .execute(
                "UploadPhoto",
                json!({"productId":1,"photo":"invalid-base64","width":8,"height":6}),
                Some(&admin)
            )
            .unwrap_err()
            .code,
        "invalid_input"
    );
}

#[test]
fn numeric_brands_require_explicit_conversion() {
    let source = r#"[type Price [decimal [range 0.01 1000] [scale 2]]] [type Weight [decimal [range 0.01 1000] [scale 2]]]
    [auth [anonymous Anonymous]] [transport HTTP [auth anonymous]]
    [query Wrong [input value Weight] [output Price] [http GET "/wrong"] [result $value]]
    [query Convert [input value Weight] [output Price] [http GET "/convert"] [result [as Price $value]]]"#;
    let mut runtime = Runtime::open(source, ":memory:", Config::default()).unwrap();
    assert_eq!(
        runtime
            .execute("Wrong", json!({"value":1.25}), None)
            .unwrap_err()
            .code,
        "invalid_output"
    );
    assert_eq!(
        runtime
            .execute("Convert", json!({"value":1.25}), None)
            .unwrap(),
        json!(1.25)
    );
}

#[test]
fn audit_is_atomic_attributes_actors_and_survives_restart() {
    let path = std::env::temp_dir().join(format!("flow-audit-{}.sqlite", uuid::Uuid::new_v4()));
    let mut r = Runtime::open(APP, path.to_str().unwrap(), config()).unwrap();
    let seeds = r.audit(0, 200).unwrap();
    assert!(
        seeds
            .as_array()
            .unwrap()
            .iter()
            .all(|v| v["operation"] == "seed")
    );
    let user_seed = seeds
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["entity"] == "User" && v["entity_id"] == 1)
        .unwrap();
    assert_eq!(user_seed["after"]["country"], "CZ");
    assert_eq!(user_seed["after"]["name"], "Petra Nováková");
    assert_eq!(user_seed["after"]["roles"], json!([]));
    let seed_max = seeds[0]["id"].as_i64().unwrap();
    let auth = token("idp:u1");
    let wal = std::fs::metadata(format!("{}-wal", path.display()))
        .unwrap()
        .len();
    r.execute("Products", json!({}), None).unwrap();
    assert_eq!(
        std::fs::metadata(format!("{}-wal", path.display()))
            .unwrap()
            .len(),
        wal
    );
    assert_eq!(r.audit(0, 200).unwrap(), seeds);
    assert!(r.execute("CreateOrder", order(2, 1), Some(&auth)).is_err());
    assert_eq!(r.audit(0, 200).unwrap(), seeds);
    r.execute("CreateOrder", order(1, 2), Some(&auth)).unwrap();
    let audit = r.audit(0, 200).unwrap();
    let changes = audit
        .as_array()
        .unwrap()
        .iter()
        .filter(|v| v["id"].as_i64().unwrap() > seed_max)
        .collect::<Vec<_>>();
    assert!(
        changes
            .iter()
            .any(|v| v["entity"] == "Order" && v["action"] == "INSERT")
    );
    let stock = changes
        .iter()
        .find(|v| v["entity"] == "Product" && v["action"] == "UPDATE")
        .unwrap();
    assert_ne!(stock["before"]["stock"], stock["after"]["stock"]);
    for v in &changes {
        assert_eq!(v["actor"]["id"], 1);
        assert_eq!(v["operation"], "CreateOrder");
        assert_eq!(v["transaction_id"], changes[0]["transaction_id"]);
    }
    let page = r.audit(audit[1]["id"].as_i64().unwrap(), 1).unwrap();
    assert_eq!(page[0], audit[2]);
    drop(r);
    let restarted = Runtime::open(APP, path.to_str().unwrap(), config()).unwrap();
    assert_eq!(restarted.audit(0, 200).unwrap(), audit);
    drop(restarted);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn audit_write_failure_rolls_back_application_changes() {
    let path = std::env::temp_dir().join(format!(
        "flow-audit-failure-{}.sqlite",
        uuid::Uuid::new_v4()
    ));
    let mut r = Runtime::open(APP, path.to_str().unwrap(), config()).unwrap();
    let baseline = r.audit(0, 200).unwrap();
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute_batch("CREATE TRIGGER fail_audit BEFORE INSERT ON _flow_audit BEGIN SELECT RAISE(ABORT,'audit unavailable'); END;").unwrap();
    let auth = token("idp:u1");
    assert!(r.execute("CreateOrder", order(1, 1), Some(&auth)).is_err());
    assert_eq!(r.audit(0, 200).unwrap(), baseline);
    assert_eq!(
        r.execute("UserOrders", json!({"userId":1}), Some(&auth))
            .unwrap(),
        json!([])
    );
    db.execute_batch("DROP TRIGGER fail_audit;").unwrap();
    r.execute("CreateOrder", order(1, 1), Some(&auth)).unwrap();
    drop(db);
    drop(r);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn admin_issued_jwt_uses_existing_identity_and_never_logs_credentials() {
    let mut runtime = Runtime::open(APP, ":memory:", config()).unwrap();
    let adapters = runtime.jwt_adapters().unwrap();
    assert!(adapters.to_string().contains("idp:u1"));
    assert!(!adapters.to_string().contains("development-key"));
    assert!(runtime.issue_admin_jwt("user", "idp:unknown", 300).is_err());
    assert!(runtime.issue_admin_jwt("user", "idp:u1", 0).is_err());
    assert!(runtime.issue_admin_jwt("user", "idp:u1", 86401).is_err());
    assert!(runtime.issue_admin_jwt("service", "idp:u1", 300).is_err());
    let issued = runtime.issue_admin_jwt("user", "idp:u1", 300).unwrap();
    let authorization = issued["authorization"].as_str().unwrap();
    let users = runtime
        .execute("Users", json!({}), Some(authorization))
        .unwrap();
    assert_eq!(users.as_array().unwrap().len(), 1);
    assert_eq!(users[0]["id"], 1);
    assert_eq!(
        issued["claims"]["exp"].as_i64().unwrap() - issued["claims"]["iat"].as_i64().unwrap(),
        300
    );
    assert!(
        !runtime
            .observability
            .logs()
            .to_string()
            .contains(issued["token"].as_str().unwrap())
    );
}

#[test]
fn numeric_ids_are_branded_and_reject_text_fractional_and_unsafe_values() {
    use flow_runtime::{program::Type, value::Value as RuntimeValue};
    let program = Program::compile(APP).unwrap();
    let ty = Type::Id("User".into());
    assert_eq!(
        program
            .validate(&ty, RuntimeValue::Id("User".into(), 1), None)
            .unwrap()
            .json()
            .unwrap(),
        json!(1)
    );
    assert!(
        program
            .validate(&ty, RuntimeValue::Id("Product".into(), 1), None)
            .is_err()
    );
    for input in [
        json!("1"),
        json!("u1"),
        json!(0),
        json!(-1),
        json!(1.5),
        json!(9007199254740992i64),
    ] {
        assert!(
            program
                .validate(&ty, RuntimeValue::from_json(&input).unwrap(), None)
                .is_err(),
            "{input}"
        );
    }
    let mut runtime = runtime();
    let auth = token("idp:u1");
    assert!(
        runtime
            .execute("UserOrders", json!({"userId":"1"}), Some(&auth))
            .is_err()
    );
    let users = runtime.execute("Users", json!({}), Some(&auth)).unwrap();
    assert_eq!(users[0]["id"], 1);
}

#[test]
fn denied_mutation_rolls_back_id_allocation() {
    let mut runtime = runtime();
    let auth = token("idp:u1");
    assert!(
        runtime
            .execute(
                "SendCommand",
                json!({"deviceId":2,"action":"START"}),
                Some(&auth)
            )
            .is_err()
    );
    let command = runtime
        .execute(
            "SendCommand",
            json!({"deviceId":1,"action":"START"}),
            Some(&auth),
        )
        .unwrap();
    assert_eq!(command["id"], 1);
    let command = runtime
        .execute(
            "SendCommand",
            json!({"deviceId":1,"action":"STOP"}),
            Some(&auth),
        )
        .unwrap();
    assert_eq!(command["id"], 2);
}

#[test]
fn numeric_sequence_survives_delete_restart_and_allocates_before_insert() {
    let source = r#"
[type NoteID [id Note]]
[entity Note [field id NoteID]]
[seed Note [rows [record [id 10]]]]
[auth [anonymous Anonymous]] [transport HTTP [auth anonymous]]
[permissions * [Note [CREATE [when true]] [DELETE [when true]] [READ [when true]]]]
[type Output [record [field id NoteID]]]
[mutate Create [output Output] [http POST "/notes"] [atomic]
 [note [create Note [record [id [new NoteID]]]]] [result note]]
[mutate Pair [output [list Output [max 2]]] [http POST "/pair"] [atomic]
 [firstId [new NoteID]] [secondId [new NoteID]]
 [first [create Note [record [id firstId]]]] [second [create Note [record [id secondId]]]]
 [result [list first second]]]
[mutate Delete [input id NoteID] [output Bool] [http DELETE "/notes/{id}"] [atomic]
 [removed [delete [entity $id]]] [result true]]
[mutate Boost [output [list Output [max 2]]] [http POST "/boost"] [atomic]
 [first [create Note [record [id 100]]]]
 [second [create Note [record [id [new NoteID]]]]] [result [list first second]]]
"#;
    let path = std::env::temp_dir().join(format!("flow-numeric-{}.sqlite", uuid::Uuid::new_v4()));
    let mut runtime = Runtime::open(source, path.to_str().unwrap(), Config::default()).unwrap();
    assert_eq!(
        runtime.execute("Create", json!({}), None).unwrap()["id"],
        11
    );
    runtime.execute("Delete", json!({"id":11}), None).unwrap();
    drop(runtime);
    let mut runtime = Runtime::open(source, path.to_str().unwrap(), Config::default()).unwrap();
    assert_eq!(
        runtime.execute("Create", json!({}), None).unwrap()["id"],
        12
    );
    let pair = runtime.execute("Pair", json!({}), None).unwrap();
    assert_eq!(pair, json!([{"id":13},{"id":14}]));
    assert_eq!(
        runtime.execute("Boost", json!({}), None).unwrap(),
        json!([{"id":100},{"id":101}])
    );
    let db = rusqlite::Connection::open(&path).unwrap();
    assert_eq!(
        db.query_row("SELECT typeof(id) FROM Note WHERE id=12", [], |row| row
            .get::<_, String>(
            0
        ))
        .unwrap(),
        "integer"
    );
    assert!(
        runtime
            .audit(0, 200)
            .unwrap()
            .as_array()
            .unwrap()
            .iter()
            .all(|row| row["entity_id"].is_i64())
    );
    drop(db);
    drop(runtime);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn legacy_text_id_database_is_rejected_without_changing_stored_data() {
    let path = std::env::temp_dir().join(format!("flow-legacy-{}.sqlite", uuid::Uuid::new_v4()));
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute_batch("CREATE TABLE _flow_schema(id INTEGER PRIMARY KEY, hash TEXT); INSERT INTO _flow_schema VALUES(1,'legacy'); CREATE TABLE Legacy(id TEXT PRIMARY KEY); INSERT INTO Legacy VALUES('u1');").unwrap();
    let error = Runtime::open(APP, path.to_str().unwrap(), config())
        .err()
        .unwrap();
    assert!(error.message.contains("legacy text IDs"));
    assert_eq!(
        db.query_row("SELECT id FROM Legacy", [], |row| row.get::<_, String>(0))
            .unwrap(),
        "u1"
    );
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM sqlite_master WHERE name='_flow_id_sequences'",
            [],
            |row| row.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    drop(db);
    std::fs::remove_file(path).unwrap();
}
