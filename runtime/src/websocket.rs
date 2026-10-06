//! WebSocket transport for declared stream and operation access.
//! Uses the same project selection and grants as HTTP.

use crate::{Error, engine::Runtime};
use axum::extract::ws::{Message, WebSocket};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, Mutex},
    time::Duration,
};
struct Subscription {
    operation: String,
    input: Value,
    seen: BTreeSet<String>,
}
pub async fn session(
    mut socket: WebSocket,
    runtime: Arc<Mutex<Runtime>>,
    credential: Option<String>,
    path: String,
) {
    let observer = runtime.lock().unwrap().observability.clone();
    let mut connection = observer.gauge_guard("ws_connections");
    connection.set(1);
    let mut subscription_gauge = observer.gauge_guard("ws_subscriptions");
    observer.event("ws_sessions_opened", 1);
    let mut subscriptions: BTreeMap<String, Subscription> = BTreeMap::new();
    let mut tick = tokio::time::interval(Duration::from_millis(100));
    loop {
        let event = tokio::select! {
            value = socket.recv() => Some(value),
            _ = tick.tick() => None,
        };
        match event {
            Some(message) => {
                let Some(Ok(message)) = message else { return };
                observer.event("ws_received", 1);
                match message {
                    Message::Text(text) => {
                        let request: Value = match serde_json::from_str(&text) {
                            Ok(v) => v,
                            Err(_) => {
                                let _ = socket
                                    .send(Message::Text(
                                        json!({"error":"invalid_input"}).to_string().into(),
                                    ))
                                    .await;
                                continue;
                            }
                        };
                        let Some(id) = request["id"]
                            .as_str()
                            .filter(|s| !s.is_empty() && s.len() <= 128)
                            .map(str::to_owned)
                        else {
                            continue;
                        };
                        if request["unsubscribe"] == true {
                            subscriptions.remove(&id);
                            subscription_gauge.set(subscriptions.len());
                            continue;
                        };
                        let Some(operation) = request["query"].as_str().map(str::to_owned) else {
                            continue;
                        };
                        let allowed = runtime.lock().ok().is_some_and(|r| {
                            r.program
                                .operations
                                .iter()
                                .any(|o| o.name == operation && o.method == "WS" && o.path == path)
                        });
                        if !allowed || subscriptions.len() >= 32 || subscriptions.contains_key(&id)
                        {
                            let _ = socket
                                .send(Message::Text(
                                    json!({"id":id,"error":"invalid_input"}).to_string().into(),
                                ))
                                .await;
                            continue;
                        };
                        subscriptions.insert(
                            id,
                            Subscription {
                                operation,
                                input: request.get("input").cloned().unwrap_or(json!({})),
                                seen: BTreeSet::new(),
                            },
                        );
                        subscription_gauge.set(subscriptions.len());
                    }
                    Message::Close(_) => return,
                    Message::Ping(data) => {
                        if socket.send(Message::Pong(data)).await.is_err() {
                            return;
                        }
                    }
                    Message::Pong(_) => {}
                    _ => return,
                }
            }
            None => {
                // Even idle connections lose authentication promptly when credentials expire.
                if !runtime.lock().ok().is_some_and(|r| {
                    r.authenticate_transport(credential.as_deref(), "WebSocket")
                        .is_ok()
                }) {
                    let _ = socket.send(Message::Close(None)).await;
                    return;
                };
                let mut remove = vec![];
                for (id, subscription) in &mut subscriptions {
                    let runtime = runtime.clone();
                    let credential = credential.clone();
                    let operation = subscription.operation.clone();
                    let input = subscription.input.clone();
                    let result = tokio::task::spawn_blocking(move || {
                        runtime
                            .lock()
                            .map_err(|_| Error::new("internal", "Runtime lock failed"))?
                            .execute_transport(
                                &operation,
                                input,
                                credential.as_deref(),
                                "WebSocket",
                            )
                    })
                    .await;
                    match result {
                        Ok(Ok((output, _, keys))) => {
                            let Some(rows) = output.as_array() else {
                                return;
                            };
                            for (key, row) in keys.iter().zip(rows) {
                                if subscription.seen.insert(key.clone()) {
                                    if socket
                                        .send(Message::Text(
                                            json!({"id":id,"data":row}).to_string().into(),
                                        ))
                                        .await
                                        .is_err()
                                    {
                                        return;
                                    }
                                    observer.event("ws_delivered", 1);
                                };
                            }
                            if subscription.seen.len() > 100000 {
                                remove.push(id.clone());
                            }
                        }
                        Ok(Err(error)) => {
                            observer.event("ws_errors", 1);
                            let _ = socket
                                .send(Message::Text(
                                    json!({"id":id,"error":error.code}).to_string().into(),
                                ))
                                .await;
                            remove.push(id.clone());
                        }
                        Err(_) => return,
                    }
                }
                for id in remove {
                    subscriptions.remove(&id);
                }
                subscription_gauge.set(subscriptions.len());
            }
        }
    }
}
