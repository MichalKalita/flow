use crate::{Error, Result, engine::Runtime};
use axum::{
    Router,
    body::{Body, to_bytes},
    extract::{FromRequestParts, Request, State, WebSocketUpgrade},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use percent_encoding::percent_decode_str;
use serde_json::{Map, Value, json};
use std::sync::{Arc, Mutex};

pub fn router(runtime: Runtime) -> Router {
    router_shared(Arc::new(Mutex::new(runtime)))
}
pub fn router_shared(runtime: Arc<Mutex<Runtime>>) -> Router {
    Router::new().fallback(dispatch).with_state(runtime)
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
    let mut response = dispatch_inner(State(runtime), request).await;
    observer.request(&endpoint, response.status().as_u16(), start.elapsed(), &id);
    response
        .headers_mut()
        .insert("x-request-id", id.parse().unwrap());
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
                .file_bytes(&id, authorization.as_deref())?;
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
        if let Some(query) = parts.uri.query() {
            for pair in query.split('&').filter(|p| !p.is_empty()) {
                let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
                merge(
                    &mut inputs,
                    decode(&key.replace('+', " "))?,
                    Value::String(decode(&value.replace('+', " "))?),
                )?;
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
