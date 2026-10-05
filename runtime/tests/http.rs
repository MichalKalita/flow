use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use flow_runtime::{
    engine::{Config, Runtime},
    http,
};
use serde_json::{Value, json};
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::Duration;
const APP: &str = include_str!("../application.flow");
fn token() -> String {
    let key = b"development-key-32-bytes-minimum-123456";
    let head = URL_SAFE_NO_PAD.encode(r#"{"alg":"HS256"}"#);
    let payload=URL_SAFE_NO_PAD.encode(json!({"iss":"https://identity.example.com","aud":"application","sub":"idp:u1","exp":chrono::Utc::now().timestamp()+300}).to_string());
    let data = format!("{head}.{payload}");
    let signature = ring::hmac::sign(
        &ring::hmac::Key::new(ring::hmac::HMAC_SHA256, key),
        data.as_bytes(),
    );
    format!(
        "Bearer {data}.{}",
        URL_SAFE_NO_PAD.encode(signature.as_ref())
    )
}
async fn request(
    address: SocketAddr,
    method: &str,
    path: &str,
    body: &str,
    auth: Option<&str>,
) -> (u16, Value) {
    let method = method.to_owned();
    let path = path.to_owned();
    let body = body.to_owned();
    let auth = auth.map(str::to_owned);
    tokio::task::spawn_blocking(move||{let mut socket=TcpStream::connect(address).unwrap();socket.set_read_timeout(Some(Duration::from_secs(5))).unwrap();let authorization=auth.map(|s|format!("Authorization: {s}\r\n")).unwrap_or_default();write!(socket,"{method} {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\nContent-Type: application/json\r\n{authorization}Content-Length: {}\r\n\r\n{body}",body.len()).unwrap();let mut response=String::new();socket.read_to_string(&mut response).unwrap();let (headers,body)=response.split_once("\r\n\r\n").unwrap();let status=headers.split_whitespace().nth(1).unwrap().parse().unwrap();(status,serde_json::from_str(body).unwrap())}).await.unwrap()
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn real_http_routes_and_permissions() {
    let mut config = Config::default();
    config.jwt_keys.insert(
        "user".into(),
        b"development-key-32-bytes-minimum-123456".to_vec(),
    );
    let runtime = Runtime::open(APP, ":memory:", config).unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server =
        tokio::spawn(async move { axum::serve(listener, http::router(runtime)).await.unwrap() });
    let (status, products) = request(address, "GET", "/api/products", "", None).await;
    assert_eq!(status, 200);
    assert_eq!(products.as_array().unwrap().len(), 4);
    assert_eq!(
        request(address, "GET", "/api/users", "", None).await,
        (200, json!([]))
    );
    assert_eq!(
        request(address, "GET", "/api/users", "", Some("Bearer invalid"))
            .await
            .0,
        401
    );
    let auth = token();
    let (status, receipt) = request(
        address,
        "POST",
        "/api/orders",
        r#"{"userId":"u1","items":[{"productId":"p1","quantity":2}],"paymentMethod":"CARD"}"#,
        Some(&auth),
    )
    .await;
    assert_eq!(status, 201, "{receipt}");
    let path = format!("/api/orders/{}", receipt["order"]["id"].as_str().unwrap());
    assert_eq!(request(address, "GET", &path, "", Some(&auth)).await.0, 200);
    assert_eq!(request(address, "GET", &path, "", None).await.0, 404);
    assert_eq!(
        request(
            address,
            "POST",
            "/api/devices/mower1/commands",
            r#"{"action":"START"}"#,
            Some(&auth)
        )
        .await
        .0,
        202
    );
    assert_eq!(
        request(
            address,
            "POST",
            "/api/devices/mower2/commands",
            r#"{"action":"STOP"}"#,
            Some(&auth)
        )
        .await
        .0,
        403
    );
    assert_eq!(
        request(
            address,
            "POST",
            "/api/devices/mower1/commands",
            r#"{"deviceId":"mower2","action":"STOP"}"#,
            Some(&auth)
        )
        .await
        .0,
        400
    );
    assert_eq!(
        request(address, "GET", "/api/products?actor=u1", "", None)
            .await
            .0,
        400
    );
    assert_eq!(request(address, "GET", "/missing", "", None).await.0, 404);
    server.abort();
}
