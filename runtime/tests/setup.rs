use flow_runtime::{
    admin,
    engine::{Config, Runtime},
    http,
    projects::Projects,
    server_state::ServerState,
};
use serde_json::{Value, json};
use std::{
    io::{Read, Write},
    net::{SocketAddr, TcpStream},
    sync::{Arc, Mutex},
};

async fn request(
    address: SocketAddr,
    method: &str,
    path: &str,
    body: Value,
    token: Option<&str>,
    origin: Option<&str>,
) -> (u16, Value) {
    let method = method.to_owned();
    let path = path.to_owned();
    let body = body.to_string();
    let token = token.map(str::to_owned);
    let origin = origin.map(str::to_owned);
    tokio::task::spawn_blocking(move || {
        let mut stream = TcpStream::connect(address).unwrap();
        stream.set_read_timeout(Some(std::time::Duration::from_secs(10))).unwrap();
        let auth = token.map(|token| format!("Authorization: Bearer {token}\r\n")).unwrap_or_default();
        let origin = origin.map(|origin| format!("Origin: {origin}\r\n")).unwrap_or_default();
        write!(stream, "{method} {path} HTTP/1.1\r\nHost: {address}\r\nConnection: close\r\nContent-Type: application/json\r\n{auth}{origin}Content-Length: {}\r\n\r\n{body}", body.len()).unwrap();
        let mut output = String::new();
        stream.read_to_string(&mut output).unwrap();
        let (headers, body) = output.split_once("\r\n\r\n").unwrap();
        let status = headers.split_whitespace().nth(1).unwrap().parse().unwrap();
        (status, serde_json::from_str(body).unwrap_or(Value::Null))
    }).await.unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn protected_first_setup_password_login_and_settings_do_not_reach_public_routes() {
    let directory = std::env::temp_dir().join(format!("flow-setup-{}", uuid::Uuid::new_v4()));
    let server = Arc::new(ServerState::open(&directory).unwrap());
    let runtime = Arc::new(Mutex::new(Runtime::open("[auth [anonymous Anonymous]] [transport HTTP [auth anonymous]] [query Health [output Bool] [http GET \"/health\"] [result true]]", ":memory:", Config::default()).unwrap()));
    let projects = Projects::single_with_server(runtime.clone(), Some(server.clone()));
    let admin_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = admin_listener.local_addr().unwrap();
    let admin_router =
        admin::router_projects_with_setup(projects, server.admin_token().unwrap(), true);
    let task = tokio::spawn(async move {
        axum::serve(admin_listener, admin_router).await.unwrap();
    });
    assert_eq!(
        request(address, "GET", "/api/settings", json!({}), None, None)
            .await
            .0,
        401
    );
    let setup = request(address, "GET", "/api/setup", json!({}), None, None).await;
    assert_eq!(setup.1["enrollment_available"], true);
    let input = json!({"name":"Office","password":"correct owner password"});
    assert_eq!(
        request(
            address,
            "POST",
            "/api/setup",
            input.clone(),
            None,
            Some("https://attacker.invalid")
        )
        .await
        .0,
        403
    );
    let enrolled = request(address, "POST", "/api/setup", input, None, None).await;
    assert_eq!(enrolled.0, 200);
    let token = enrolled.1["token"].as_str().unwrap();
    assert_eq!(
        request(
            address,
            "POST",
            "/api/setup",
            json!({"name":"Other","password":"other owner password"}),
            None,
            None
        )
        .await
        .0,
        409
    );
    assert_eq!(
        request(
            address,
            "POST",
            "/api/login",
            json!({"password":"incorrect"}),
            None,
            None
        )
        .await
        .0,
        401
    );
    let login = request(
        address,
        "POST",
        "/api/login",
        json!({"password":"correct owner password"}),
        None,
        None,
    )
    .await;
    assert_eq!(login.0, 200);
    assert_eq!(login.1["token"], token);
    let settings = request(
        address,
        "GET",
        "/api/settings",
        json!({}),
        Some(token),
        None,
    )
    .await;
    assert_eq!(settings.0, 200);
    assert_eq!(settings.1["name"], "Office");
    assert!(settings.1.get("admin_token").is_none());
    assert!(settings.1.get("owner_hash").is_none());
    let changed = request(
        address,
        "POST",
        "/api/settings",
        json!({"revision":settings.1["revision"],"name":"Renamed"}),
        Some(token),
        None,
    )
    .await;
    assert_eq!(changed.0, 200);
    assert_eq!(changed.1["instance"], settings.1["instance"]);
    assert_eq!(
        request(
            address,
            "POST",
            "/api/settings",
            json!({"revision":settings.1["revision"],"name":"Old"}),
            Some(token),
            None
        )
        .await
        .0,
        409
    );
    let public_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let public_address = public_listener.local_addr().unwrap();
    let public = http::router_shared(runtime);
    let public_task = tokio::spawn(async move {
        axum::serve(public_listener, public).await.unwrap();
    });
    assert_eq!(
        request(
            public_address,
            "GET",
            "/api/settings",
            json!({}),
            Some(token),
            None
        )
        .await
        .0,
        404
    );
    assert_eq!(
        request(public_address, "POST", "/api/setup", json!({}), None, None)
            .await
            .0,
        404
    );
    task.abort();
    public_task.abort();
    drop(server);
    let _ = std::fs::remove_dir_all(directory);
}
