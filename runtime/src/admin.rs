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
        .layer(middleware::from_fn_with_state(state.clone(), authorize))
        .route("/", get(|| async { Html(include_str!("admin.html")) }))
        .layer(middleware::from_fn(headers))
        .with_state(state)
}
async fn headers(request: Request, next: Next) -> Response {
    let mut response = next.run(request).await;
    for (key, value) in [
        ("cache-control", "no-store"),
        ("x-content-type-options", "nosniff"),
        ("x-frame-options", "DENY"),
        (
            "content-security-policy",
            "default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; connect-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'none'",
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
async fn overview(State(state): State<Admin>) -> Json<Value> {
    let runtime = state.runtime.lock().unwrap();
    let endpoints=runtime.program.operations.iter().filter(|op|op.event.is_none()).map(|op|json!({"name":op.name,"method":op.method,"path":op.path,"inputs":op.inputs.iter().map(|(name,(ty,_))|json!({"name":name,"type":format!("{ty:?}")})).collect::<Vec<_>>()})).collect::<Vec<_>>();
    Json(
        json!({"endpoints":endpoints,"metrics":runtime.observability.snapshot(),"logs":runtime.observability.logs()}),
    )
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
    match tokio::task::spawn_blocking(move || runtime.lock().unwrap().audit(before, 100)).await {
        Ok(Ok(value)) => Json(value).into_response(),
        _ => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}
async fn call(State(state): State<Admin>, Json(input): Json<Value>) -> Response {
    let name = input["endpoint"].as_str().unwrap_or("");
    let op = state
        .runtime
        .lock()
        .unwrap()
        .program
        .operations
        .iter()
        .find(|op| op.name == name && op.event.is_none() && op.method != "WS")
        .cloned();
    let Some(op) = op else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    let path = input["path"].as_str().unwrap_or(&op.path);
    // Only local declared endpoints. No network proxy, arbitrary host or admin endpoint.
    let uri = match path.parse::<axum::http::Uri>() {
        Ok(uri) if uri.scheme().is_none() && uri.authority().is_none() => uri,
        _ => return StatusCode::BAD_REQUEST.into_response(),
    };
    if !http::route(&op.path, uri.path()).ok().flatten().is_some() {
        return StatusCode::BAD_REQUEST.into_response();
    }
    let method = match op.method.parse::<Method>() {
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
