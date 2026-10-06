//! Process entry. Loads configuration, starts transports, and serves hosted projects.
//! Keeps the admin listener separate from the public port.

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
        .block_on(async {
            let args = std::env::args().skip(1).collect::<Vec<_>>();
            let _lease = if args
                .first()
                .is_some_and(|argument| argument.starts_with("--"))
            {
                None
            } else {
                Some(flow_runtime::server_state::RuntimeLease::acquire(
                    &state_directory(&args),
                )?)
            };
            while run().await? {}
            Ok(())
        })
}
async fn run() -> Result<bool, Box<dyn std::error::Error>> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if args.first().is_some_and(|s| s == "--restore") {
        if args.len() != 4 {
            return Err("Usage: flow-runtime --restore BACKUP_DIRECTORY RECOVERY_PASSWORD_FILE EMPTY_DESTINATION".into());
        }
        let password_file = std::path::Path::new(&args[2]);
        if std::fs::metadata(password_file)?.len() > 512 {
            return Err("Recovery password file exceeds its limit".into());
        }
        let password = std::fs::read_to_string(password_file)?;
        let result = flow_runtime::backups::restore_fresh(
            std::path::Path::new(&args[1]),
            password.trim_end_matches(['\r', '\n']),
            std::path::Path::new(&args[3]),
        )?;
        println!(
            "Restored {} projects to {}",
            result["projects"].as_array().map_or(0, Vec::len),
            args[3]
        );
        return Ok(false);
    }
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
        return Ok(false);
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
    let state_directory = state_directory(&args);
    flow_runtime::backups::activate_pending(&state_directory)?;
    let server = Arc::new(flow_runtime::server_state::ServerState::open(
        &state_directory,
    )?);
    let admin_token = match std::env::var("FLOW_ADMIN_TOKEN") {
        Ok(value) if value.len() >= 32 => value,
        Ok(_) => return Err("FLOW_ADMIN_TOKEN must have at least 32 bytes".into()),
        Err(std::env::VarError::NotPresent) => server.admin_token()?,
        Err(_) => return Err("FLOW_ADMIN_TOKEN must be valid text".into()),
    };
    let admin_bind = std::env::var("FLOW_ADMIN_BIND")
        .unwrap_or_else(|_| "127.0.0.1:9090".into())
        .parse::<std::net::SocketAddr>()?;
    let admin_allow_remote =
        std::env::var("FLOW_ADMIN_ALLOW_REMOTE").is_ok_and(|value| value == "1");
    if !admin_bind.ip().is_loopback() && !admin_allow_remote {
        return Err(
            "Admin listener must bind to loopback unless FLOW_ADMIN_ALLOW_REMOTE=1 is explicitly set"
                .into(),
        );
    }
    let observer = Observability::disk(
        std::env::var("FLOW_OBSERVABILITY_DIR").unwrap_or_else(|_| "data/observability".into()),
    )?;
    let identity = server.status()?;
    observer.identity(
        identity["instance"].as_str().unwrap_or(""),
        identity["name"].as_str().unwrap_or(""),
    );
    observer.policy(flow_runtime::observability::LogPolicy {
        target_bytes: identity["log_target_bytes"].as_u64().unwrap(),
        chunk_bytes: identity["log_chunk_bytes"].as_u64().unwrap(),
    })?;
    println!(
        "Flow server {} ({})",
        identity["name"].as_str().unwrap_or(""),
        identity["instance"].as_str().unwrap_or("")
    );
    if args.is_empty() {
        std::fs::create_dir_all(source)?;
    }
    let project_mode = std::path::Path::new(source).is_dir();
    let projects = if project_mode {
        Projects::load_with_server(
            std::path::Path::new(source),
            std::path::Path::new(database),
            config,
            observer.clone(),
            Some(server.clone()),
        )?
    } else {
        let mut runtime = Runtime::open(&std::fs::read_to_string(source)?, database, config)?;
        runtime.observability = observer.clone();
        Projects::single_with_server(Arc::new(Mutex::new(runtime)), Some(server.clone()))
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
    *projects.public_address.write().unwrap() = Some(listener.local_addr()?);
    let mqtt_bind = args.get(3).map(String::as_str).unwrap_or("127.0.0.1:1883");
    let mqtt_listener = tokio::net::TcpListener::bind(mqtt_bind).await?;
    let admin_listener = tokio::net::TcpListener::bind(admin_bind).await?;
    println!(
        "Flow admin listening on http://{}",
        admin_listener.local_addr()?
    );
    let admin_router = admin::router_projects_with_setup(
        projects.clone(),
        admin_token,
        admin_bind.ip().is_loopback() && std::env::var_os("FLOW_ADMIN_TOKEN").is_none(),
    );
    let shutdown_projects = projects.clone();
    let admin_task = tokio::spawn(async move {
        axum::serve(admin_listener, admin_router)
            .with_graceful_shutdown(async move {
                shutdown_projects.shutdown.notified().await;
            })
            .await
    });
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
    let admission_label = admission
        .map(|limit| limit.to_string())
        .unwrap_or_else(|| "unlimited".into());
    println!(
        "Flow HTTP listening on http://{} (SQLite: {database}; {workers} Tokio workers, {blocking} blocking, {admission_label} in-flight)",
        listener.local_addr()?
    );
    let public = if project_mode {
        http::router_projects_with_admission(projects.clone(), admission)
    } else {
        http::router_shared_with_admission(projects.get("application").unwrap(), admission)
    };
    let restarting = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let shutdown_started = Arc::new(tokio::sync::Notify::new());
    let shutdown_signal = shutdown_started.clone();
    let restart_signal = restarting.clone();
    let shutdown_projects = projects.clone();
    let mut public_task = tokio::spawn(async move {
        axum::serve(listener,public).with_graceful_shutdown(async move {
            tokio::select! { _=tokio::signal::ctrl_c()=>{}, _=shutdown_projects.restart.notified()=>{ restart_signal.store(true,std::sync::atomic::Ordering::Release); } }
            shutdown_projects.gate.stopped.store(true,std::sync::atomic::Ordering::Release);
            shutdown_signal.notify_one();
        }).await
    });
    tokio::select! {
        result=&mut public_task=>{result??;},
        _=shutdown_started.notified()=>{
            if tokio::time::timeout(std::time::Duration::from_secs(10),&mut public_task).await.is_err() { public_task.abort(); let _=public_task.await; }
        }
    }
    let restarting = restarting.load(std::sync::atomic::Ordering::Acquire);
    sampler.abort();
    watcher.abort();
    mqtt_task.abort();
    let _ = sampler.await;
    let _ = watcher.await;
    let _ = mqtt_task.await;
    projects.shutdown.notify_one();
    let mut admin_task = admin_task;
    if tokio::time::timeout(std::time::Duration::from_secs(10), &mut admin_task)
        .await
        .is_err()
    {
        admin_task.abort();
        let _ = admin_task.await;
    }
    if restarting {
        flow_runtime::backups::freeze_for_restore(&projects)?;
    }
    projects.flush()?;
    for (_, runtime) in projects.active() {
        runtime.lock().unwrap().observability.close();
    }
    projects.system.close();
    if restarting {
        server.close()?;
    }
    Ok(restarting)
}

fn state_directory(args: &[String]) -> std::path::PathBuf {
    let database = args.get(1).map(String::as_str).unwrap_or("data/projects");
    std::env::var_os("FLOW_SERVER_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            let parent = std::path::Path::new(database)
                .parent()
                .filter(|path| !path.as_os_str().is_empty())
                .unwrap_or(std::path::Path::new("data"));
            parent.join("server")
        })
}
