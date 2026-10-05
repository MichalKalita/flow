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
    runtime: Arc<Mutex<Runtime>>,
    token: Arc<String>,
}
pub fn router(runtime: Arc<Mutex<Runtime>>, token: String) -> Router {
    assert!(token.len() >= 32, "Admin token must have at least 32 bytes");
    let state = Admin {
        runtime,
        token: Arc::new(token),
    };
    Router::new()
        .route("/api/overview", get(overview))
        .route("/api/audit", get(audit))
        .route("/api/call", post(call))
        .route("/api/logs", get(logs))
        .route("/api/jwt", get(jwt_adapters).post(issue_jwt))
        .layer(middleware::from_fn_with_state(
            Arc::new(tokio::sync::Semaphore::new(4)),
            admit,
        ))
        .layer(middleware::from_fn_with_state(state.clone(), authorize))
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
    next.run(request).await
}
async fn overview(State(state): State<Admin>) -> Response {
    match tokio::task::spawn_blocking(move || {
        let process=crate::resources::process_memory();
        let runtime=state.runtime.lock().unwrap();
        let mut endpoints=runtime.program.operations.iter().filter(|op|op.event.is_none()).map(|op| {
            let inputs=op.inputs.iter().map(|(name,(ty,default))| {
                let default=default.as_ref().and_then(|n|crate::program::literal(n).ok()).and_then(|v|v.json().ok());
                json!({"name":name,"type":format!("{ty:?}"),"default":default})
            }).collect::<Vec<_>>();
            json!({"name":op.name,"method":op.method,"path":op.path,"mutation":op.mutation,"status":op.status,"inputs":inputs})
        }).collect::<Vec<_>>();
        if runtime.program.plugins.contains_key("Files.put") {
            endpoints.push(json!({"name":"FileDownload","method":"GET","path":"/api/files/{id}","mutation":false,"status":200,"inputs":[{"name":"id","type":"File ID"}]}));
        }
        let streams=runtime.program.streams.iter().map(|(name,stream)|json!({"name":name,"topic":stream.topic,"retention_seconds":stream.duration,"max_messages":stream.max_messages})).collect::<Vec<_>>();
        let automations=runtime.program.operations.iter().filter_map(|op|op.event.as_ref().map(|event|json!({"name":op.name,"source":event.source,"actor":event.actor_id}))).collect::<Vec<_>>();
        let mut resources=runtime.storage_resources();
        resources["process"]=process;
        resources["host"]=crate::resources::host_memory();
        resources["observability"]=runtime.observability.resources();
        Json(json!({"endpoints":endpoints,"streams":streams,"automations":automations,"metrics":runtime.observability.snapshot(),"resources":resources,"version":env!("CARGO_PKG_VERSION")}))
    }).await {
        Ok(response)=>response.into_response(),
        Err(_)=>StatusCode::INTERNAL_SERVER_ERROR.into_response(),
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
    let runtime = state.runtime.clone();
    match tokio::task::spawn_blocking(move || {
        runtime.lock().unwrap().audit_filtered(before, 100, &query)
    })
    .await
    {
        Ok(Ok(value)) => Json(value).into_response(),
        _ => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}
async fn call(State(state): State<Admin>, Json(input): Json<Value>) -> Response {
    let name = input["endpoint"].as_str().unwrap_or("");
    let declared = {
        let runtime = state.runtime.lock().unwrap();
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
    if let Some(auth) = input["authorization"].as_str() {
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
    http::dispatch(State(state.runtime), request).await
}

async fn logs(
    State(state): State<Admin>,
    Query(filters): Query<BTreeMap<String, String>>,
) -> Response {
    let observer = state.runtime.lock().unwrap().observability.clone();
    match tokio::task::spawn_blocking(move || crate::log_store::query(&observer, &filters)).await {
        Ok(Ok(value)) => Json(value).into_response(),
        _ => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}
async fn jwt_adapters(State(state): State<Admin>) -> Response {
    match tokio::task::spawn_blocking(move || state.runtime.lock().unwrap().jwt_adapters()).await {
        Ok(Ok(value)) => Json(value).into_response(),
        _ => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}
async fn issue_jwt(State(state): State<Admin>, Json(input): Json<Value>) -> Response {
    let alias = input["adapter"].as_str().unwrap_or("").to_owned();
    let subject = input["subject"].as_str().unwrap_or("").to_owned();
    let ttl = input["ttl_seconds"].as_u64().unwrap_or(3600);
    match tokio::task::spawn_blocking(move || {
        state
            .runtime
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
