use flow_runtime::{
    engine::{Config, Runtime},
    http, mqtt,
    program::Program,
};
use std::sync::{Arc, Mutex};
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if args.first().is_some_and(|s| s == "--check") {
        let path = args
            .get(1)
            .ok_or("Usage: flow-runtime --check application.flow")?;
        let program = Program::compile(&std::fs::read_to_string(path)?)?;
        println!(
            "Valid: {} entities, {} HTTP operations, {} WebSocket subscriptions",
            program.entities.len(),
            program
                .operations
                .iter()
                .filter(|o| o.method != "WS" && o.event.is_none())
                .count(),
            program
                .operations
                .iter()
                .filter(|o| o.method == "WS")
                .count()
        );
        return Ok(());
    }
    let source = args
        .first()
        .map(String::as_str)
        .unwrap_or("application.flow");
    let database = args.get(1).map(String::as_str).unwrap_or("flow.sqlite");
    let bind = args.get(2).map(String::as_str).unwrap_or("127.0.0.1:8080");
    let mut config = Config::default();
    if let Ok(secret) = std::env::var("FLOW_JWT_SECRET") {
        config.jwt_keys.insert("user".into(), secret.into_bytes());
    }
    if let Ok(secret) = std::env::var("FLOW_AUTOMATION_KEY") {
        config.event_credentials.insert(
            "service:device-automation".into(),
            format!("ApiKey {secret}"),
        );
    }
    let runtime = Runtime::open(&std::fs::read_to_string(source)?, database, config)?;
    let runtime = Arc::new(Mutex::new(runtime));
    let listener = tokio::net::TcpListener::bind(bind).await?;
    let mqtt_bind = args.get(3).map(String::as_str).unwrap_or("127.0.0.1:1883");
    let mqtt_listener = tokio::net::TcpListener::bind(mqtt_bind).await?;
    println!("Flow MQTT listening on {}", mqtt_listener.local_addr()?);
    let mqtt_runtime = runtime.clone();
    let mqtt_task = tokio::spawn(async move { mqtt::serve(mqtt_listener, mqtt_runtime).await });
    println!(
        "Flow HTTP listening on http://{} (SQLite: {database})",
        listener.local_addr()?
    );
    axum::serve(listener, http::router_shared(runtime))
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    mqtt_task.abort();
    Ok(())
}
