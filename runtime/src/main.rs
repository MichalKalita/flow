use flow_runtime::{
    admin,
    engine::{Config, Runtime},
    http, mqtt,
    observability::Observability,
    program::Program,
};
use std::sync::{Arc, Mutex};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .max_blocking_threads(2)
        .enable_all()
        .build()?
        .block_on(run())
}
async fn run() -> Result<(), Box<dyn std::error::Error>> {
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
    let admin_token = std::env::var("FLOW_ADMIN_TOKEN")
        .map_err(|_| "Set FLOW_ADMIN_TOKEN (at least 32 bytes) for the internal dashboard")?;
    if admin_token.len() < 32 {
        return Err("FLOW_ADMIN_TOKEN must have at least 32 bytes".into());
    }
    let admin_bind = std::env::var("FLOW_ADMIN_BIND")
        .unwrap_or_else(|_| "127.0.0.1:9090".into())
        .parse::<std::net::SocketAddr>()?;
    if !admin_bind.ip().is_loopback() {
        return Err(
            "Admin listener must bind to loopback; use an SSH tunnel for remote access".into(),
        );
    }
    let observer = Observability::disk(
        std::env::var("FLOW_OBSERVABILITY_DIR").unwrap_or_else(|_| "data/observability".into()),
    )?;
    let mut runtime = Runtime::open(&std::fs::read_to_string(source)?, database, config)?;
    runtime.observability = observer.clone();
    let runtime = Arc::new(Mutex::new(runtime));
    let sampler_observer = observer.clone();
    let sampler = tokio::spawn(async move {
        let mut tick = tokio::time::interval(std::time::Duration::from_secs(5));
        loop {
            tick.tick().await;
            if let Ok(process) =
                tokio::task::spawn_blocking(flow_runtime::resources::process_memory).await
            {
                sampler_observer.sample(process);
            }
        }
    });
    let listener = tokio::net::TcpListener::bind(bind).await?;
    let mqtt_bind = args.get(3).map(String::as_str).unwrap_or("127.0.0.1:1883");
    let mqtt_listener = tokio::net::TcpListener::bind(mqtt_bind).await?;
    let admin_listener = tokio::net::TcpListener::bind(admin_bind).await?;
    println!(
        "Flow admin listening on http://{}",
        admin_listener.local_addr()?
    );
    let admin_router = admin::router(runtime.clone(), admin_token);
    let admin_task = tokio::spawn(async move { axum::serve(admin_listener, admin_router).await });
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
    sampler.abort();
    mqtt_task.abort();
    admin_task.abort();
    tokio::task::spawn_blocking(move || observer.flush()).await??;
    Ok(())
}
