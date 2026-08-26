use garmin_mcp::auth;
use rmcp::transport::streamable_http_server::{
    session::local::LocalSessionManager, StreamableHttpService,
};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Load .env from cwd if present.  Existing process-env vars win, so
    // explicit `GARMIN_EMAIL=… cargo run` still overrides .env.  No-op
    // (.ok()) if the file is absent — production deployments use real env.
    let _ = dotenvy::dotenv();

    eprintln!("Garmin MCP (Rust) starting...");

    // One authenticated client for the process lifetime — GarminMcpServer is
    // Clone (cheap: shares the same Arc<RwLock<DiSession>> and http client),
    // so every HTTP session reuses this login instead of re-authenticating.
    let server = auth::create_garmin_server().await?;

    let port: u16 = std::env::var("GARMIN_MCP_HTTP_PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(8210);

    let service = StreamableHttpService::new(
        move || Ok(server.clone()),
        LocalSessionManager::default().into(),
        Default::default(),
    );
    let router = axum::Router::new().nest_service("/mcp", service);
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", port)).await?;

    eprintln!("Authenticated. Serving MCP over HTTP on http://127.0.0.1:{port}/mcp");

    axum::serve(listener, router).await?;

    Ok(())
}
