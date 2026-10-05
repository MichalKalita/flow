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
fn order(user: &str, qty: u32) -> Value {
    json!({"userId":user,"items":[{"productId":"p1","quantity":qty}],"paymentMethod":"CARD"})
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
            .filter(|o| o.method != "WS")
            .count(),
        9
    );
    let mut r = runtime();
    let (products, trace) = r.execute_traced("Products", json!({}), None).unwrap();
    assert_eq!(products.as_array().unwrap().len(), 4);
    assert_eq!(products[0], json!({"id":"p1","name":"Studio sluchátka"}));
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
        json!([{"id":"u1","name":"Petra Nováková"}])
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
        r.execute("Users", json!({"actor":"u1"}), Some(&auth))
            .unwrap_err()
            .code,
        "invalid_input"
    );
}
#[test]
fn order_transaction_and_owned_reads() {
    let mut r = runtime();
    let auth = token("idp:u1");
    let receipt = r
        .execute("CreateOrder", order("u1", 2), Some(&auth))
        .unwrap();
    assert_eq!(receipt["order"]["total"], 4980);
    assert_eq!(receipt["order"]["items"][0]["quantity"], 2);
    let id = receipt["order"]["id"].as_str().unwrap();
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
        r.execute("UserOrders", json!({"userId":"u1"}), Some(&auth))
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
        r.execute("CreateOrder", order("u2", 2), Some(&auth))
            .unwrap_err()
            .code,
        "forbidden"
    );
    let receipt = r
        .execute("CreateOrder", order("u1", 12), Some(&auth))
        .unwrap();
    assert_eq!(receipt["order"]["items"][0]["quantity"], 12);
    assert!(
        r.execute("CreateOrder", order("u1", 1), Some(&auth))
            .is_err()
    );
    assert_eq!(
        r.execute("UserOrders", json!({"userId":"u1"}), Some(&auth))
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
    let receipt=r.execute("CreateOrder",json!({"userId":"u1","items":[{"productId":"p1","quantity":1},{"productId":"p1","quantity":2}],"paymentMethod":"BANK"}),Some(&auth)).unwrap();
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
            json!({"deviceId":"mower1","action":"START"}),
            Some(&auth)
        )
        .is_ok()
    );
    assert_eq!(
        r.execute(
            "SendCommand",
            json!({"deviceId":"mower2","action":"STOP"}),
            Some(&auth)
        )
        .unwrap_err()
        .code,
        "forbidden"
    );
    assert_eq!(
        r.execute("Device", json!({"deviceId":"mower2"}), Some(&auth))
            .unwrap_err()
            .code,
        "not_found"
    );
    assert_eq!(
        r.execute("Device", json!({"deviceId":"mower1"}), Some(&auth))
            .unwrap()["positions"],
        json!([])
    );
}
#[test]
fn strict_inputs_and_prices() {
    let mut r = runtime();
    let auth = token("idp:u1");
    for input in [
        json!({"userId":"u1","items":[],"paymentMethod":"CARD"}),
        json!({"userId":"u1","items":[{"productId":"p1","quantity":1,"hidden":true}],"paymentMethod":"CARD"}),
        json!({"userId":"u1","items":[{"productId":"p1","quantity":0}],"paymentMethod":"CARD"}),
        json!({"userId":"u1","items":[{"productId":"p1","quantity":1.01}],"paymentMethod":"CARD"}),
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
        id = r
            .execute("CreateOrder", order("u1", 1), Some(&auth))
            .unwrap()["order"]["id"]
            .clone();
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
