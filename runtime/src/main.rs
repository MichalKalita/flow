use flow_runtime::{
    engine::{Config, Runtime},
    http,
    program::Program,
};
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if args.first().is_some_and(|s| s == "--check") {
        let path = args
            .get(1)
            .ok_or("Usage: flow-runtime --check application.flow")?;
        let program = Program::compile(&std::fs::read_to_string(path)?)?;
        println!(
            "Valid: {} entities, {} HTTP operations",
            program.entities.len(),
            program.operations.len()
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
    let runtime = Runtime::open(&std::fs::read_to_string(source)?, database, config)?;
    let listener = tokio::net::TcpListener::bind(bind).await?;
    println!(
        "Flow HTTP listening on http://{} (SQLite: {database})",
        listener.local_addr()?
    );
    axum::serve(listener, http::router(runtime))
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    Ok(())
}
