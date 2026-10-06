//! Serves the generated application page for a project that grants it.
//! Uses the project's existing user authentication. Does not bypass permission checks.

use crate::{
    Error, Result,
    engine::Runtime,
    program::{Program, Type},
};
use axum::{body::Body, extract::Request, response::Response};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};

pub fn validate(program: &Program, metadata: &Value) -> Result<Value> {
    let invalid = || Error::new("configuration", "Invalid application frontend declaration");
    let fields = metadata["fields"]
        .as_array()
        .filter(|fields| !fields.is_empty() && fields.len() <= 24)
        .ok_or_else(invalid)?;
    let title = metadata["title"]
        .as_str()
        .filter(|title| !title.is_empty() && title.len() <= 120)
        .ok_or_else(invalid)?;
    let id_input = metadata["id_input"].as_str().ok_or_else(invalid)?;
    let mut operations = serde_json::Map::new();
    for (kind, method) in [
        ("list", "GET"),
        ("create", "POST"),
        ("update", "PUT"),
        ("delete", "DELETE"),
    ] {
        let alias = metadata[kind].as_str().ok_or_else(invalid)?;
        let operation = program
            .operations
            .iter()
            .find(|operation| {
                operation.name == alias && operation.method == method && operation.event.is_none()
            })
            .ok_or_else(invalid)?;
        if (kind == "update" || kind == "delete")
            && (!operation.inputs.contains_key(id_input)
                || !operation.inputs.contains_key("version"))
        {
            return Err(invalid());
        }
        operations.insert(
            kind.into(),
            json!({"name":operation.name,"method":operation.method,"path":operation.path}),
        );
    }
    if program.operations.iter().any(|operation| {
        ["/app", "/app/config", "/app.js", "/app.css"].contains(&operation.path.as_str())
    }) {
        return Err(invalid());
    }
    let mut names = std::collections::BTreeSet::new();
    for field in fields {
        let name = field["name"].as_str().ok_or_else(invalid)?;
        if !names.insert(name) || name == id_input || ["id", "owner", "version"].contains(&name) {
            return Err(invalid());
        }
        field["label"]
            .as_str()
            .filter(|label| !label.is_empty() && label.len() <= 120)
            .ok_or_else(invalid)?;
        if !["text", "email", "tel", "textarea"].contains(&field["kind"].as_str().unwrap_or("")) {
            return Err(invalid());
        }
        for kind in ["create", "update"] {
            let alias = metadata[kind].as_str().ok_or_else(invalid)?;
            let input = program
                .operations
                .iter()
                .find(|operation| operation.name == alias)
                .and_then(|operation| operation.inputs.get(name))
                .ok_or_else(invalid)?;
            if !matches!(program.resolve(&input.0)?, Type::String | Type::Text { .. }) {
                return Err(invalid());
            }
        }
    }
    Ok(
        json!({"title":title,"item_label":metadata["item_label"].as_str().filter(|label|!label.is_empty() && label.len()<=40).unwrap_or("record"),"id_input":id_input,"fields":fields,"operations":operations}),
    )
}

pub fn dispatch(runtime: &Arc<Mutex<Runtime>>, request: &Request) -> Option<Response> {
    if request.method() != "GET" {
        return None;
    }
    let path = request.uri().path();
    if !["/app", "/app.js", "/app.css", "/app/config"].contains(&path) {
        return None;
    }
    let runtime = runtime.lock().ok()?;
    let metadata = runtime.frontend.as_ref()?;
    if runtime.suspended {
        return Response::builder()
            .status(503)
            .body(Body::from("Application is restarting"))
            .ok();
    }
    let (status, kind, body) = match path {
        "/app" => (
            200,
            "text/html; charset=utf-8",
            include_bytes!("admin-assets/application.html")
                .as_slice()
                .to_vec(),
        ),
        "/app.js" => (
            200,
            "text/javascript; charset=utf-8",
            include_bytes!("admin-assets/application.js")
                .as_slice()
                .to_vec(),
        ),
        "/app.css" => (
            200,
            "text/css; charset=utf-8",
            include_bytes!("admin-assets/application.css")
                .as_slice()
                .to_vec(),
        ),
        _ => {
            let authorization = request
                .headers()
                .get("authorization")
                .and_then(|value| value.to_str().ok());
            if runtime
                .authenticate_transport(authorization, "HTTP")
                .is_err()
            {
                (
                    401,
                    "application/json",
                    b"{\"error\":\"unauthenticated\"}".to_vec(),
                )
            } else {
                (200, "application/json", serde_json::to_vec(metadata).ok()?)
            }
        }
    };
    Response::builder().status(status).header("content-type",kind).header("cache-control","no-store").header("x-content-type-options","nosniff").header("referrer-policy","no-referrer").header("content-security-policy","default-src 'none'; script-src 'self'; style-src 'self'; connect-src 'self'; base-uri 'none'; form-action 'self'; frame-ancestors 'none'").body(Body::from(body)).ok()
}
