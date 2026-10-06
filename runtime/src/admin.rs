use crate::{engine::Runtime, http};
use axum::{
    Json, Router,
    body::Body,
    extract::{Query, Request, State},
    http::{Method, StatusCode},
    middleware::{self, Next},
    response::{Html, IntoResponse, Response},
    routing::{get, post},
};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

#[derive(Clone)]
struct Admin {
    projects: Arc<crate::projects::Projects>,
    token: Arc<String>,
    enrollment_local: bool,
    login_attempts: Arc<Mutex<std::collections::VecDeque<std::time::Instant>>>,
}
pub fn router(runtime: Arc<Mutex<Runtime>>, token: String) -> Router {
    router_projects(crate::projects::Projects::single(runtime), token)
}
pub fn router_projects(projects: Arc<crate::projects::Projects>, token: String) -> Router {
    router_projects_with_setup(projects, token, false)
}
pub fn router_projects_with_setup(
    projects: Arc<crate::projects::Projects>,
    token: String,
    enrollment_local: bool,
) -> Router {
    assert!(token.len() >= 32, "Admin token must have at least 32 bytes");
    let state = Admin {
        projects,
        token: Arc::new(token),
        enrollment_local,
        login_attempts: Arc::new(Mutex::new(std::collections::VecDeque::new())),
    };
    Router::new()
        .route("/api/overview", get(overview))
        .route("/api/settings", get(settings).post(save_settings))
        .route("/api/settings/secret", post(save_secret))
        .route("/api/catalog", get(catalog_index).post(catalog_install))
        .route("/api/catalog/launch", post(catalog_launch))
        .route("/api/backups", get(backup_list).post(backup_create))
        .route("/api/backups/verify", post(backup_verify))
        .route("/api/backups/restore", post(backup_restore))
        .route("/api/backups/status", get(backup_status))
        .route("/api/projects", get(project_list))
        .route("/api/data", get(data_tables).post(data_write))
        .route("/api/data/rows", get(data_rows))
        .route("/api/audit", get(audit))
        .route("/api/call", post(call))
        .route("/api/logs", get(logs))
        .route("/api/jwt", get(jwt_adapters).post(issue_jwt))
        .layer(middleware::from_fn_with_state(state.clone(), authorize))
        .route("/api/setup", get(setup_status).post(enroll))
        .route("/api/login", post(owner_login))
        .route(
            "/",
            get(|| async { Html(include_str!("admin-assets/index.html")) }),
        )
        .route(
            "/assets/app.js",
            get(|| async {
                (
                    [("content-type", "text/javascript; charset=utf-8")],
                    include_str!("admin-assets/app.js"),
                )
            }),
        )
        .route(
            "/assets/app.css",
            get(|| async {
                (
                    [("content-type", "text/css; charset=utf-8")],
                    include_str!("admin-assets/app.css"),
                )
            }),
        )
        .layer(middleware::from_fn_with_state(
            Arc::new(tokio::sync::Semaphore::new(4)),
            admit,
        ))
        .layer(middleware::from_fn(headers))
        .with_state(state)
}
async fn admit(
    State(semaphore): State<Arc<tokio::sync::Semaphore>>,
    request: Request,
    next: Next,
) -> Response {
    let Ok(_permit) = semaphore.try_acquire() else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    next.run(request).await
}
async fn headers(request: Request, next: Next) -> Response {
    let mut response = next.run(request).await;
    for (key, value) in [
        ("cache-control", "no-store"),
        ("x-content-type-options", "nosniff"),
        ("x-frame-options", "DENY"),
        (
            "content-security-policy",
            "default-src 'none'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; connect-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'none'",
        ),
    ] {
        response.headers_mut().insert(key, value.parse().unwrap());
    }
    response
}
async fn authorize(State(state): State<Admin>, request: Request, next: Next) -> Response {
    let supplied = request
        .headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .unwrap_or("");
    let expected = ring::hmac::Key::new(ring::hmac::HMAC_SHA256, state.token.as_bytes());
    let candidate = ring::hmac::sign(
        &ring::hmac::Key::new(ring::hmac::HMAC_SHA256, supplied.as_bytes()),
        b"flow-admin",
    );
    if ring::hmac::verify(&expected, b"flow-admin", candidate.as_ref()).is_err() {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let start = std::time::Instant::now();
    let endpoint = format!("{} {}", request.method(), request.uri().path());
    let response = next.run(request).await;
    state.projects.system.log(json!({"kind":"admin","endpoint":endpoint,"status":response.status().as_u16(),"duration_ms":start.elapsed().as_secs_f64()*1000.}));
    response
}
fn selected(
    state: &Admin,
    query: &BTreeMap<String, String>,
) -> crate::Result<Option<Arc<Mutex<Runtime>>>> {
    let name = query
        .get("project")
        .cloned()
        .or_else(|| state.projects.default_name());
    match name.as_deref() {
        None | Some("all") => Ok(None),
        Some(name) => state
            .projects
            .get(name)
            .map(Some)
            .ok_or_else(|| crate::Error::new("not_found", "Project is unavailable")),
    }
}
fn required(state: &Admin, query: &BTreeMap<String, String>) -> crate::Result<Arc<Mutex<Runtime>>> {
    selected(state, query)?
        .ok_or_else(|| crate::Error::new("invalid_input", "Select an individual project"))
}
pub(crate) fn overview_value(runtime: &Runtime) -> Value {
    let mut endpoints=runtime.program.operations.iter().filter(|op|op.event.is_none()).map(|op| {
            let inputs=op.inputs.iter().map(|(name,(ty,default))| {
                let default=default.as_ref().and_then(|n|crate::program::literal(n).ok()).and_then(|v|v.json().ok());
                json!({"name":name,"type":format!("{:?}", runtime.program.resolve(ty).unwrap_or(ty)),"default":default})
            }).collect::<Vec<_>>();
            json!({"name":op.name,"method":op.method,"path":op.path,"mutation":op.mutation,"status":op.status,"inputs":inputs})
        }).collect::<Vec<_>>();
    if runtime.program.plugins.contains_key("Files.put") {
        endpoints.push(json!({"name":"FileDownload","method":"GET","path":"/api/files/{id}","mutation":false,"status":200,"inputs":[{"name":"id","type":"File ID"}]}));
    }
    let streams=runtime.program.streams.iter().map(|(name,stream)|json!({"name":name,"topic":stream.topic,"retention_seconds":stream.duration,"max_messages":stream.max_messages})).collect::<Vec<_>>();
    let automations = runtime
        .program
        .operations
        .iter()
        .filter_map(|op| {
            op.event
                .as_ref()
                .map(|event| json!({"name":op.name,"source":event.source,"actor":event.actor_id}))
        })
        .collect::<Vec<_>>();
    let mut resources = runtime.storage_resources();

    resources["observability"] = runtime.observability.resources();
    json!({"endpoints":endpoints,"streams":streams,"automations":automations,"metrics":runtime.observability.snapshot(),"resources":resources,"version":env!("CARGO_PKG_VERSION")})
}
async fn project_list(State(state): State<Admin>) -> Json<Value> {
    Json(
        json!({"projects":state.projects.list(),"default_project":state.projects.default_name().unwrap_or("all".into())}),
    )
}
async fn overview(
    State(state): State<Admin>,
    Query(query): Query<BTreeMap<String, String>>,
) -> Response {
    let selected = match selected(&state, &query) {
        Ok(value) => value,
        Err(error) => return failure(error),
    };
    match tokio::task::spawn_blocking(move || {
        let mut value = if let Some(runtime) = selected {
            overview_value(&runtime.lock().unwrap())
        } else {
            crate::dashboard::system_overview(&state.projects)
        };
        value["resources"]["process"] = crate::resources::process_memory();
        value["resources"]["host"] = crate::resources::host_memory();
        value["projects"] = state.projects.list();
        value["scope"] = json!(
            query
                .get("project")
                .cloned()
                .or_else(|| state.projects.default_name())
                .unwrap_or("all".into())
        );
        Json(value)
    })
    .await
    {
        Ok(value) => value.into_response(),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}
async fn audit(
    State(state): State<Admin>,
    Query(query): Query<BTreeMap<String, String>>,
) -> Response {
    let before = query
        .get("before")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let runtime = match selected(&state, &query) {
        Ok(value) => value,
        Err(error) => return failure(error),
    };
    match tokio::task::spawn_blocking(move || {
        if let Some(runtime) = runtime {
            runtime.lock().unwrap().audit_filtered(before, 100, &query)
        } else {
            crate::dashboard::system_audit(&state.projects, &query)
        }
    })
    .await
    {
        Ok(Ok(value)) => Json(value).into_response(),
        Ok(Err(error)) => failure(error),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

async fn call(
    State(state): State<Admin>,
    Query(query): Query<BTreeMap<String, String>>,
    Json(input): Json<Value>,
) -> Response {
    let selected = match required(&state, &query) {
        Ok(value) => value,
        Err(error) => return failure(error),
    };
    let name = input["endpoint"].as_str().unwrap_or("");
    let declared = {
        let runtime = selected.lock().unwrap();
        runtime
            .program
            .operations
            .iter()
            .find(|op| op.name == name && op.event.is_none() && op.method != "WS")
            .map(|op| (op.method.clone(), op.path.clone()))
            .or_else(|| {
                (name == "FileDownload" && runtime.program.plugins.contains_key("Files.put"))
                    .then(|| ("GET".into(), "/api/files/{id}".into()))
            })
    };
    let Some((method, pattern)) = declared else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    let path = input["path"].as_str().unwrap_or(&pattern);
    // Only local declared endpoints. No network proxy, arbitrary host or admin endpoint.
    let uri = match path.parse::<axum::http::Uri>() {
        Ok(uri) if uri.scheme().is_none() && uri.authority().is_none() => uri,
        _ => return StatusCode::BAD_REQUEST.into_response(),
    };
    if !http::route(&pattern, uri.path()).ok().flatten().is_some() {
        return StatusCode::BAD_REQUEST.into_response();
    }
    let method = match method.parse::<Method>() {
        Ok(m) => m,
        Err(_) => return StatusCode::BAD_REQUEST.into_response(),
    };
    let mut request = Request::builder()
        .method(method)
        .uri(uri)
        .header("content-type", "application/json");
    if let Some(auth) = input["authorization"]
        .as_str()
        .filter(|s| !s.trim().is_empty())
    {
        request = request.header("authorization", auth);
    }
    let body = input
        .get("body")
        .filter(|b| !b.is_null())
        .map(Value::to_string)
        .unwrap_or_default();
    let Ok(request) = request.body(Body::from(body)) else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    http::dispatch(State(selected), request).await
}

async fn logs(
    State(state): State<Admin>,
    Query(filters): Query<BTreeMap<String, String>>,
) -> Response {
    let runtime = match selected(&state, &filters) {
        Ok(value) => value,
        Err(error) => return failure(error),
    };
    if runtime.is_none() {
        return match tokio::task::spawn_blocking(move || {
            crate::dashboard::system_logs(&state.projects, &filters)
        })
        .await
        {
            Ok(value) => Json(value).into_response(),
            Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
        };
    }
    let observer = runtime.unwrap().lock().unwrap().observability.clone();
    match tokio::task::spawn_blocking(move || crate::log_store::query(&observer, &filters)).await {
        Ok(Ok(value)) => Json(value).into_response(),
        _ => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}
async fn jwt_adapters(
    State(state): State<Admin>,
    Query(query): Query<BTreeMap<String, String>>,
) -> Response {
    let runtime = match required(&state, &query) {
        Ok(value) => value,
        Err(error) => return failure(error),
    };
    match tokio::task::spawn_blocking(move || runtime.lock().unwrap().jwt_adapters()).await {
        Ok(Ok(value)) => Json(value).into_response(),
        _ => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}
async fn issue_jwt(
    State(state): State<Admin>,
    Query(query): Query<BTreeMap<String, String>>,
    Json(input): Json<Value>,
) -> Response {
    let runtime = match required(&state, &query) {
        Ok(value) => value,
        Err(error) => return failure(error),
    };
    let alias = input["adapter"].as_str().unwrap_or("").to_owned();
    let subject = input["subject"].as_str().unwrap_or("").to_owned();
    let ttl = input["ttl_seconds"].as_u64().unwrap_or(3600);
    match tokio::task::spawn_blocking(move || {
        runtime
            .lock()
            .unwrap()
            .issue_admin_jwt(&alias, &subject, ttl)
    })
    .await
    {
        Ok(Ok(value)) => Json(value).into_response(),
        Ok(Err(error)) => {
            let status = match error.code {
                "not_found" => 404,
                "invalid_input" => 400,
                "configuration" => 409,
                _ => 500,
            };
            (
                axum::http::StatusCode::from_u16(status).unwrap(),
                Json(json!({"error":error.code})),
            )
                .into_response()
        }
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}
fn failure(error: crate::Error) -> Response {
    let status = match error.code {
        "unauthenticated" => 401,
        "forbidden" => 403,
        "unavailable" => 503,
        "not_found" => 404,
        "invalid_input" | "invalid_output" | "limit" => 400,
        "conflict" | "database" => 409,
        _ => 500,
    };
    (
        StatusCode::from_u16(status).unwrap(),
        Json(json!({"error":error.code,"message":error.message})),
    )
        .into_response()
}
async fn data_tables(
    State(state): State<Admin>,
    Query(query): Query<BTreeMap<String, String>>,
) -> Response {
    let runtime = match required(&state, &query) {
        Ok(value) => value,
        Err(error) => return failure(error),
    };
    Json(runtime.lock().unwrap().admin_tables()).into_response()
}
async fn data_rows(
    State(state): State<Admin>,
    Query(query): Query<BTreeMap<String, String>>,
) -> Response {
    let runtime = match required(&state, &query) {
        Ok(value) => value,
        Err(error) => return failure(error),
    };
    match tokio::task::spawn_blocking(move || {
        runtime.lock().unwrap().admin_rows(
            query.get("entity").map(String::as_str).unwrap_or(""),
            query.get("after").and_then(|s| s.parse().ok()).unwrap_or(0),
            50,
            query.get("search").map(String::as_str).unwrap_or(""),
            query.get("id").and_then(|s| s.parse().ok()),
        )
    })
    .await
    {
        Ok(Ok(value)) => Json(value).into_response(),
        Ok(Err(error)) => failure(error),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}
async fn data_write(
    State(state): State<Admin>,
    Query(query): Query<BTreeMap<String, String>>,
    Json(input): Json<Value>,
) -> Response {
    let runtime = match required(&state, &query) {
        Ok(value) => value,
        Err(error) => return failure(error),
    };
    match tokio::task::spawn_blocking(move || {
        runtime.lock().unwrap().admin_write(
            input["action"].as_str().unwrap_or(""),
            input["entity"].as_str().unwrap_or(""),
            input["id"].as_i64(),
            input["record"].clone(),
            input["etag"].as_str(),
        )
    })
    .await
    {
        Ok(Ok(value)) => Json(value).into_response(),
        Ok(Err(error)) => failure(error),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

fn server(state: &Admin) -> crate::Result<Arc<crate::server_state::ServerState>> {
    state.projects.server.clone().ok_or_else(|| {
        crate::Error::new(
            "configuration",
            "Persistent server settings are unavailable",
        )
    })
}

async fn settings(State(state): State<Admin>) -> Response {
    match server(&state).and_then(|server| server.status()) {
        Ok(mut value) => {
            value["logs"] = state.projects.system.resources();
            Json(value).into_response()
        }
        Err(error) => failure(error),
    }
}

async fn save_settings(State(state): State<Admin>, Json(input): Json<Value>) -> Response {
    let server = match server(&state) {
        Ok(server) => server,
        Err(error) => return failure(error),
    };
    let observer = state.projects.system.clone();
    match tokio::task::spawn_blocking(move || {
        let value = server.update_settings(&input)?;
        observer.identity(
            value["instance"].as_str().unwrap_or(""),
            value["name"].as_str().unwrap_or(""),
        );
        observer
            .policy(crate::observability::LogPolicy {
                target_bytes: value["log_target_bytes"].as_u64().unwrap(),
                chunk_bytes: value["log_chunk_bytes"].as_u64().unwrap(),
            })
            .map_err(|_| crate::Error::new("configuration", "Invalid log settings"))?;
        Ok::<_, crate::Error>(value)
    })
    .await
    {
        Ok(Ok(value)) => Json(value).into_response(),
        Ok(Err(error)) => failure(error),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

async fn save_secret(State(state): State<Admin>, Json(input): Json<Value>) -> Response {
    let server = match server(&state) {
        Ok(server) => server,
        Err(error) => return failure(error),
    };
    match tokio::task::spawn_blocking(move || {
        server.set_secret(
            input["project"].as_str().unwrap_or(""),
            input["name"].as_str().unwrap_or(""),
            input["value"].as_str().unwrap_or(""),
        )?;
        server.status()
    })
    .await
    {
        Ok(Ok(value)) => Json(value).into_response(),
        Ok(Err(error)) => failure(error),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

async fn setup_status(State(state): State<Admin>) -> Response {
    match server(&state).and_then(|server| server.status()) {
        Ok(value) => Json(json!({"name":value["name"],"setup_complete":value["setup_complete"],"enrollment_available":state.enrollment_local,"password_login":value["setup_complete"]})).into_response(),
        Err(_) => Json(json!({"setup_complete":true,"enrollment_available":false,"password_login":false})).into_response(),
    }
}

fn login_admission(state: &Admin) -> bool {
    let now = std::time::Instant::now();
    let mut attempts = state.login_attempts.lock().unwrap();
    while attempts
        .front()
        .is_some_and(|time| now.duration_since(*time).as_secs() >= 60)
    {
        attempts.pop_front();
    }
    if attempts.len() >= 10 {
        return false;
    }
    attempts.push_back(now);
    true
}

fn same_origin_local(headers: &axum::http::HeaderMap) -> bool {
    let Some(host) = headers.get("host").and_then(|v| v.to_str().ok()) else {
        return false;
    };
    let hostname = if host.starts_with('[') {
        host.split(']').next().map(|name| format!("{name}]"))
    } else {
        host.split(':').next().map(str::to_owned)
    };
    if !hostname.is_some_and(|name| ["localhost", "127.0.0.1", "[::1]"].contains(&name.as_str())) {
        return false;
    }
    headers
        .get("origin")
        .and_then(|value| value.to_str().ok())
        .is_none_or(|origin| origin == format!("http://{host}"))
}

async fn enroll(
    State(state): State<Admin>,
    headers: axum::http::HeaderMap,
    Json(input): Json<Value>,
) -> Response {
    if !state.enrollment_local || !same_origin_local(&headers) {
        return StatusCode::FORBIDDEN.into_response();
    }
    if !login_admission(&state) {
        return StatusCode::TOO_MANY_REQUESTS.into_response();
    }
    let server = match server(&state) {
        Ok(server) => server,
        Err(error) => return failure(error),
    };
    match tokio::task::spawn_blocking(move || {
        server.enroll(
            input["name"].as_str().unwrap_or(""),
            input["password"].as_str().unwrap_or(""),
        )
    })
    .await
    {
        Ok(Ok(token)) => Json(json!({"token":token})).into_response(),
        Ok(Err(error)) => failure(error),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

async fn owner_login(State(state): State<Admin>, Json(input): Json<Value>) -> Response {
    if !login_admission(&state) {
        return StatusCode::TOO_MANY_REQUESTS.into_response();
    }
    let server = match server(&state) {
        Ok(server) => server,
        Err(error) => return failure(error),
    };
    match tokio::task::spawn_blocking(move || {
        server.login(input["password"].as_str().unwrap_or(""))
    })
    .await
    {
        Ok(Ok(token)) => Json(json!({"token":token})).into_response(),
        Ok(Err(error)) => failure(error),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

async fn catalog_index(State(state): State<Admin>) -> Response {
    match crate::catalog::index(&state.projects) {
        Ok(value) => Json(value).into_response(),
        Err(error) => failure(error),
    }
}
async fn catalog_install(State(state): State<Admin>, Json(input): Json<Value>) -> Response {
    match tokio::task::spawn_blocking(move || {
        crate::catalog::install(
            &state.projects,
            input["template"].as_str().unwrap_or(""),
            input["request_id"].as_str().unwrap_or(""),
        )
    })
    .await
    {
        Ok(Ok(value)) => Json(value).into_response(),
        Ok(Err(error)) => failure(error),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}
async fn catalog_launch(State(state): State<Admin>, Json(input): Json<Value>) -> Response {
    match tokio::task::spawn_blocking(move || {
        crate::catalog::launch(&state.projects, input["project"].as_str().unwrap_or(""))
    })
    .await
    {
        Ok(Ok(value)) => Json(value).into_response(),
        Ok(Err(error)) => failure(error),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}
async fn backup_list(State(state): State<Admin>) -> Response {
    match tokio::task::spawn_blocking(move || crate::backups::list(&state.projects)).await {
        Ok(Ok(value)) => Json(value).into_response(),
        Ok(Err(error)) => failure(error),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}
async fn backup_create(State(state): State<Admin>) -> Response {
    match tokio::task::spawn_blocking(move || crate::backups::create(&state.projects)).await {
        Ok(Ok(value)) => Json(value).into_response(),
        Ok(Err(error)) => failure(error),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}
async fn backup_verify(State(state): State<Admin>, Json(input): Json<Value>) -> Response {
    match tokio::task::spawn_blocking(move || {
        crate::backups::verify(
            &state.projects,
            input["id"].as_str().unwrap_or(""),
            input["password"].as_str().unwrap_or(""),
        )
    })
    .await
    {
        Ok(Ok(value)) => Json(value).into_response(),
        Ok(Err(error)) => failure(error),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

async fn backup_restore(State(state): State<Admin>, Json(input): Json<Value>) -> Response {
    if input["confirm"] != true {
        return failure(crate::Error::new(
            "invalid_input",
            "Confirm the selected backup and replacement of current applications",
        ));
    }
    match tokio::task::spawn_blocking(move || {
        crate::backups::request_restore(
            &state.projects,
            input["id"].as_str().unwrap_or(""),
            input["password"].as_str().unwrap_or(""),
        )
    })
    .await
    {
        Ok(Ok(value)) => Json(value).into_response(),
        Ok(Err(error)) => failure(error),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

async fn backup_status(State(state): State<Admin>) -> Response {
    match crate::backups::restore_status(&state.projects) {
        Ok(value) => Json(value).into_response(),
        Err(error) => failure(error),
    }
}
