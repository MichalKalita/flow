//! Dispatches public HTTP to declared operations for one project.
//! Applies the same grants as every other transport. Does not expose the admin database editor.

use crate::{Error, Result, engine::Runtime};
use axum::{
    Router,
    body::{Body, to_bytes},
    extract::{FromRequestParts, Request, State, WebSocketUpgrade},
    http::StatusCode,
    middleware::{self, Next},
    response::{IntoResponse, Response},
};
use percent_encoding::percent_decode_str;
use serde_json::{Map, Value, json};
use std::sync::{Arc, Mutex};

pub fn router(runtime: Runtime) -> Router {
    router_shared(Arc::new(Mutex::new(runtime)))
}
pub fn router_shared(runtime: Arc<Mutex<Runtime>>) -> Router {
    router_shared_limited(runtime, 16)
}
pub fn router_shared_limited(runtime: Arc<Mutex<Runtime>>, admission: usize) -> Router {
    router_shared_with_admission(runtime, Some(admission))
}
pub fn router_shared_with_admission(
    runtime: Arc<Mutex<Runtime>>,
    admission: Option<usize>,
) -> Router {
    let observer = runtime.lock().unwrap().observability.clone();
    with_admission(
        Router::new().fallback(dispatch).with_state(runtime),
        admission,
        observer,
    )
}
fn with_admission(
    router: Router,
    admission: Option<usize>,
    observer: crate::observability::Observability,
) -> Router {
    match admission {
        Some(limit) => router.layer(middleware::from_fn_with_state(
            (
                Arc::new(tokio::sync::Semaphore::new(limit.max(1))),
                observer,
            ),
            admit,
        )),
        None => router,
    }
}
async fn admit(
    State((semaphore, observer)): State<(
        Arc<tokio::sync::Semaphore>,
        crate::observability::Observability,
    )>,
    request: Request,
    next: Next,
) -> Response {
    let Ok(_permit) = semaphore.try_acquire() else {
        observer.request(
            "overloaded",
            503,
            std::time::Duration::ZERO,
            &uuid::Uuid::new_v4().to_string(),
        );
        return json_response(503, json!({"error":"overloaded"}));
    };
    next.run(request).await
}
fn decode(s: &str) -> Result<String> {
    percent_decode_str(s)
        .decode_utf8()
        .map(|s| s.into_owned())
        .map_err(|_| Error::new("invalid_input", "Invalid URL encoding"))
}
pub(crate) fn route(pattern: &str, path: &str) -> Result<Option<Map<String, Value>>> {
    let pattern = pattern.split('/').collect::<Vec<_>>();
    let path = path.split('/').collect::<Vec<_>>();
    if pattern.len() != path.len() {
        return Ok(None);
    };
    let mut inputs = Map::new();
    for (a, b) in pattern.iter().zip(path) {
        let b = decode(b)?;
        if let Some(key) = a.strip_prefix('{').and_then(|s| s.strip_suffix('}')) {
            if b.is_empty() || b.contains('/') {
                return Ok(None);
            };
            if inputs.insert(key.into(), Value::String(b)).is_some() {
                return Err(Error::new("invalid_input", "Duplicate path input"));
            }
        } else if *a != b {
            return Ok(None);
        }
    }
    Ok(Some(inputs))
}
fn parameter(
    runtime: &Arc<Mutex<Runtime>>,
    operation: &str,
    key: &str,
    value: Value,
) -> Result<Value> {
    let guard = runtime
        .lock()
        .map_err(|_| Error::new("internal", "Runtime lock failed"))?;
    let ty = guard
        .program
        .operations
        .iter()
        .find(|op| op.name == operation)
        .and_then(|op| op.inputs.get(key))
        .map(|(ty, _)| ty);
    let id_type = if let Some(mut ty) = ty {
        loop {
            match guard.program.resolve(ty)? {
                crate::program::Type::Optional(inner) => ty = inner,
                crate::program::Type::Id(_) => break true,
                _ => break false,
            }
        }
    } else {
        false
    };
    if id_type {
        let text = value
            .as_str()
            .ok_or_else(|| Error::new("invalid_input", "Invalid numeric ID"))?;
        if !text.bytes().all(|b| b.is_ascii_digit()) {
            return Err(Error::new("invalid_input", "Invalid numeric ID"));
        }
        let number = crate::value::number(text)?;
        return Ok(json!(crate::program::valid_id(crate::value::Value::Num(
            number
        ))?));
    }
    Ok(value)
}
fn merge(inputs: &mut Map<String, Value>, key: String, value: Value) -> Result<()> {
    if inputs.insert(key, value).is_some() {
        return Err(Error::new("invalid_input", "Input supplied more than once"));
    };
    Ok(())
}
pub(crate) async fn dispatch(
    State(runtime): State<Arc<Mutex<Runtime>>>,
    request: Request,
) -> Response {
    let start = std::time::Instant::now();
    let id = uuid::Uuid::new_v4().to_string();
    let (observer, endpoint) = {
        let guard = runtime.lock().unwrap();
        let path = request.uri().path();
        let endpoint = if request.method() == "GET" && path.starts_with("/api/files/") {
            "GET /api/files/{id}".to_owned()
        } else {
            guard
                .program
                .operations
                .iter()
                .find(|op| {
                    (op.method == request.method().as_str() || op.method == "WS")
                        && route(&op.path, path).ok().flatten().is_some()
                })
                .map(|op| format!("{} {}", op.method, op.path))
                .unwrap_or_else(|| "unmatched".into())
        };
        (guard.observability.clone(), endpoint)
    };
    let mut active = observer.gauge_guard("http_inflight");
    active.set(1);
    let mut response = if let Some(response) = crate::frontend::dispatch(&runtime, &request) {
        response
    } else {
        dispatch_inner(State(runtime), request).await
    };
    observer.request(&endpoint, response.status().as_u16(), start.elapsed(), &id);
    response
        .headers_mut()
        .insert("x-request-id", id.parse().unwrap());
    if let Some(instance) = observer.server_id()
        && let Ok(value) = instance.parse()
    {
        response.headers_mut().insert("x-flow-server", value);
    }
    response
}
async fn dispatch_inner(State(runtime): State<Arc<Mutex<Runtime>>>, request: Request) -> Response {
    let result = async {
        let (mut parts, body) = request.into_parts();
        let authorization = match parts
            .headers
            .get_all("authorization")
            .iter()
            .collect::<Vec<_>>()
            .as_slice()
        {
            [] => None,
            [v] => Some(
                v.to_str()
                    .map_err(|_| Error::new("unauthenticated", "Invalid authorization"))?
                    .to_owned(),
            ),
            _ => return Err(Error::new("unauthenticated", "Duplicate authorization")),
        };
        let method = parts.method.as_str();
        let path = parts.uri.path();
        let is_websocket = runtime.lock().ok().is_some_and(|r| {
            r.program
                .operations
                .iter()
                .any(|o| o.method == "WS" && o.path == path)
        });
        if is_websocket {
            runtime
                .lock()
                .map_err(|_| Error::new("internal", "Runtime lock failed"))?
                .authenticate_transport(authorization.as_deref(), "WebSocket")?;
            let path = path.to_owned();
            let upgrade = WebSocketUpgrade::from_request_parts(&mut parts, &())
                .await
                .map_err(|_| Error::new("invalid_input", "Expected WebSocket upgrade"))?;
            return Ok::<_, Error>(
                upgrade
                    .max_message_size(1024 * 1024)
                    .on_upgrade(move |socket| {
                        crate::websocket::session(socket, runtime, authorization, path)
                    })
                    .into_response(),
            );
        }
        if method == "GET" && path.starts_with("/api/files/") {
            let id = decode(path.trim_start_matches("/api/files/"))?;
            if id.is_empty() || id.contains('/') {
                return Err(Error::new("not_found", "File missing"));
            };
            let bytes = runtime
                .lock()
                .map_err(|_| Error::new("internal", "Runtime lock failed"))?
                .file_bytes(
                    &id.parse::<i64>()
                        .map_err(|_| Error::new("invalid_input", "Invalid numeric ID"))?,
                    authorization.as_deref(),
                )?;
            return Ok(Response::builder()
                .status(200)
                .header("content-type", "image/png")
                .header("cache-control", "no-store")
                .body(Body::from(bytes))
                .unwrap());
        }
        let (name, status, mut inputs) = {
            let guard = runtime
                .lock()
                .map_err(|_| Error::new("internal", "Runtime lock failed"))?;
            let mut matches = vec![];
            for op in &guard.program.operations {
                if op.method == method
                    && let Some(inputs) = route(&op.path, path)?
                {
                    matches.push((op.name.clone(), op.status, inputs))
                }
            }
            if matches.len() != 1 {
                return Err(Error::new("not_found", "Route missing or ambiguous"));
            };
            matches.pop().unwrap()
        };
        for (key, value) in &mut inputs {
            *value = parameter(&runtime, &name, key, value.clone())?;
        }
        if let Some(query) = parts.uri.query() {
            for pair in query.split('&').filter(|p| !p.is_empty()) {
                let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
                let key = decode(&key.replace('+', " "))?;
                let value = parameter(
                    &runtime,
                    &name,
                    &key,
                    Value::String(decode(&value.replace('+', " "))?),
                )?;
                merge(&mut inputs, key, value)?;
            }
        }
        let bytes = to_bytes(body, 16 * 1024 * 1024)
            .await
            .map_err(|_| Error::new("invalid_input", "Body exceeds 16 MiB"))?;
        if !bytes.is_empty() {
            let content_type = parts
                .headers
                .get("content-type")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("");
            if content_type.split(';').next() != Some("application/json") {
                return Err(Error::new("invalid_input", "Expected application/json"));
            };
            let json: Value = serde_json::from_slice(&bytes)?;
            let object = json
                .as_object()
                .ok_or_else(|| Error::new("invalid_input", "Expected object body"))?;
            for (key, value) in object {
                merge(&mut inputs, key.clone(), value.clone())?;
            }
        }
        let output = tokio::task::spawn_blocking(move || {
            runtime
                .lock()
                .map_err(|_| Error::new("internal", "Runtime lock failed"))?
                .execute(&name, Value::Object(inputs), authorization.as_deref())
        })
        .await
        .map_err(|_| Error::new("internal", "Runtime task failed"))??;
        Ok::<_, Error>(json_response(status, output))
    }
    .await;
    let (status, body) = match result {
        Ok(response) => return response,
        Err(error) => {
            let status = match error.code {
                "unauthenticated" => 401,
                "forbidden" => 403,
                "not_found" => 404,
                "invalid_input" | "invalid_output" | "limit" => 400,
                "conflict" => 409,
                "unavailable" => 503,
                _ => 500,
            };
            if status == 500 {
                eprintln!("{error}")
            };
            (
                status,
                json!({"error":if status==500{"internal"}else{error.code}}),
            )
        }
    };
    json_response(status, body)
}
fn json_response(status: u16, body: Value) -> Response {
    Response::builder()
        .status(StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR))
        .header("content-type", "application/json; charset=utf-8")
        .header("cache-control", "no-store")
        .body(Body::from(body.to_string()))
        .unwrap()
}

pub fn router_projects(projects: Arc<crate::projects::Projects>) -> Router {
    router_projects_limited(projects, 16)
}
pub fn router_projects_limited(
    projects: Arc<crate::projects::Projects>,
    admission: usize,
) -> Router {
    router_projects_with_admission(projects, Some(admission))
}
pub fn router_projects_with_admission(
    projects: Arc<crate::projects::Projects>,
    admission: Option<usize>,
) -> Router {
    let observer = projects.system.clone();
    with_admission(
        Router::new()
            .fallback(dispatch_project)
            .with_state(projects),
        admission,
        observer,
    )
}
async fn dispatch_project(
    State(projects): State<Arc<crate::projects::Projects>>,
    mut request: Request,
) -> Response {
    let path = request.uri().path();
    let mut segments = path.trim_start_matches('/').splitn(2, '/');
    let name = segments.next().unwrap_or("");
    let Some(runtime) = projects.get(name) else {
        projects.system.request(
            "unmatched",
            404,
            std::time::Duration::ZERO,
            &uuid::Uuid::new_v4().to_string(),
        );
        return json_response(404, json!({"error":"project_not_found"}));
    };
    let mut uri = format!("/{}", segments.next().unwrap_or(""));
    if let Some(query) = request.uri().query() {
        uri.push('?');
        uri.push_str(query)
    }
    let name = name.to_string();
    *request.uri_mut() = uri.parse().unwrap();
    let mut response = dispatch(State(runtime), request).await;
    response
        .headers_mut()
        .insert("x-flow-project", name.parse().unwrap());
    response
}
