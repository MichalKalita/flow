use flow_runtime::{
    admin,
    engine::{Config, Runtime},
    http, mqtt,
    observability::Observability,
    program::Program,
    projects::Projects,
};
use std::sync::{Arc, Mutex};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let workers = flow_runtime::resources::tokio_worker_threads();
    let blocking = flow_runtime::resources::tokio_blocking_threads();
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(workers)
        .max_blocking_threads(blocking)
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
    let source = args.first().map(String::as_str).unwrap_or("projects");
    let database = args.get(1).map(String::as_str).unwrap_or("data/projects");
    let bind = args.get(2).map(String::as_str).unwrap_or("0.0.0.0:80");
    let mut config = Config::default();
    if let Ok(secret) = std::env::var("FLOW_JWT_SECRET") {
        config.jwt_keys.insert("user".into(), secret.into_bytes());
    }
    if let Ok(secret) = std::env::var("FLOW_AUTOMATION_KEY") {
        config
            .event_credentials
            .insert("service:1".into(), format!("ApiKey {secret}"));
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
    let project_mode = std::path::Path::new(source).is_dir();
    let projects = if project_mode {
        Projects::load(
            std::path::Path::new(source),
            std::path::Path::new(database),
            config,
            observer.clone(),
        )?
    } else {
        let mut runtime = Runtime::open(&std::fs::read_to_string(source)?, database, config)?;
        runtime.observability = observer.clone();
        Projects::single(Arc::new(Mutex::new(runtime)))
    };
    let watch_projects = projects.clone();
    let watcher = tokio::spawn(async move {
        let mut tick = tokio::time::interval(std::time::Duration::from_secs(2));
        loop {
            tick.tick().await;
            let registry = watch_projects.clone();
            let logger = watch_projects.system.clone();
            match tokio::task::spawn_blocking(move ||registry.scan()).await {
                Ok(Ok(()))=>{}, Ok(Err(error))=>logger.log(serde_json::json!({"kind":"project","name":"scan","success":false,"error":error.to_string()})),
                Err(error)=>logger.log(serde_json::json!({"kind":"project","name":"scan","success":false,"error":error.to_string()}))
            }
        }
    });
    let sampler_projects = projects.clone();
    let sampler = tokio::spawn(async move {
        let mut tick = tokio::time::interval(std::time::Duration::from_secs(5));
        loop {
            tick.tick().await;
            if let Ok(process) =
                tokio::task::spawn_blocking(flow_runtime::resources::process_memory).await
            {
                sampler_projects.sample(process);
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
    let admin_router = admin::router_projects(projects.clone(), admin_token);
    let admin_task = tokio::spawn(async move { axum::serve(admin_listener, admin_router).await });
    println!("Flow MQTT listening on {}", mqtt_listener.local_addr()?);
    let mqtt_projects = projects.clone();
    let mqtt_task = tokio::spawn(async move {
        if project_mode {
            mqtt::serve_projects(mqtt_listener, mqtt_projects).await
        } else {
            mqtt::serve(mqtt_listener, mqtt_projects.get("application").unwrap()).await
        }
    });
    let workers = flow_runtime::resources::tokio_worker_threads();
    let blocking = flow_runtime::resources::tokio_blocking_threads();
    let admission = flow_runtime::resources::http_admission();
    println!(
        "Flow HTTP listening on http://{} (SQLite: {database}; {workers} Tokio workers, {blocking} blocking, {admission} in-flight)",
        listener.local_addr()?
    );
    let public = if project_mode {
        http::router_projects_limited(projects.clone(), admission)
    } else {
        http::router_shared_limited(projects.get("application").unwrap(), admission)
    };
    axum::serve(listener, public)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    sampler.abort();
    watcher.abort();
    mqtt_task.abort();
    admin_task.abort();
    tokio::task::spawn_blocking(move || projects.flush()).await??;
    Ok(())
}
