use garmin_mcp::auth;
use rmcp::{transport::stdio, ServiceExt};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Load .env from cwd if present.  Existing process-env vars win, so
    // explicit `GARMIN_EMAIL=… cargo run` still overrides .env.  No-op
    // (.ok()) if the file is absent — production deployments use real env.
    let _ = dotenvy::dotenv();

    eprintln!("Garmin MCP (Rust) starting...");

    let server = auth::create_garmin_server().await?;

    eprintln!("Authenticated. Starting MCP server on stdio...");

    let service = server
        .serve(stdio())
        .await
        .inspect_err(|e| eprintln!("Server startup error: {e}"))?;

    service.waiting().await?;

    Ok(())
}
