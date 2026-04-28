use anyhow::{bail, Result};
use garmin_client::GarminClient;

use crate::client::{read_session_file, GarminApiClient};
use crate::tools::GarminMcpServer;

pub async fn create_garmin_server() -> Result<GarminMcpServer> {
    let email = read_secret("GARMIN_EMAIL", "GARMIN_EMAIL_FILE")?;
    let password = read_secret("GARMIN_PASSWORD", "GARMIN_PASSWORD_FILE")?;

    // Workaround for garmin_client 0.2.1 bug (lib.rs:534):
    //   retrieve_json_session() calls Value::to_string() on the cached token,
    //   re-quoting the JWT so every request sends `Authorization: Bearer "eyJ..."`.
    //   Force a fresh OAuth handshake each launch until fixed upstream.
    let _ = std::fs::remove_file(".garmin_session.json");

    eprintln!("Authenticating with Garmin Connect...");
    let mut client = GarminClient::new();
    if !client.login(&email, &password).await {
        bail!("Garmin authentication failed. Check GARMIN_EMAIL and GARMIN_PASSWORD.");
    }

    // After login(), garmin_client writes .garmin_session.json with the raw
    // token.  read_session_file() uses .as_str() (not .to_string()) so the
    // JWT is never re-quoted.
    let (bearer_token, token_expires_at) = read_session_file().unwrap_or_else(|e| {
        eprintln!("Warning: could not read session file ({e}); write operations may fail after ~1 h.");
        (String::new(), 0)
    });

    let display_name = resolve_display_name(&mut client).await;
    if display_name.is_empty() {
        eprintln!("Warning: display name is empty.");
        eprintln!("  Tools requiring display name (get_stats, get_sleep_summary, …)");
        eprintln!("  will fail. Fix: set GARMIN_DISPLAY_NAME=<your Garmin handle> in env.");
    } else {
        eprintln!("Logged in as: {display_name}");
    }

    let api = GarminApiClient::new(client, display_name, bearer_token, token_expires_at);
    Ok(GarminMcpServer::new(api))
}

/// Try env var override first, then probe multiple Garmin API endpoints.
async fn resolve_display_name(client: &mut GarminClient) -> String {
    if let Ok(name) = std::env::var("GARMIN_DISPLAY_NAME") {
        let name = name.trim().to_string();
        if !name.is_empty() {
            return name;
        }
    }

    let endpoints = [
        "/userprofile-service/userprofile/v2/information",
        "/userprofile-service/socialProfile",
        "/userprofile-service/userprofile",
    ];

    for endpoint in &endpoints {
        if let Some(name) = try_display_name(client, endpoint).await {
            return name;
        }
    }

    String::new()
}

async fn try_display_name(client: &mut GarminClient, endpoint: &str) -> Option<String> {
    // garmin_client's url_builder prepends "/", so a leading slash produces "//path" → 404/error.
    let endpoint = endpoint.trim_start_matches('/');
    let ok = client.api_request(endpoint, None, true, None).await;
    let text = client.get_last_resp_text().to_string();

    eprintln!("[auth] {endpoint} → ok={ok}, body_len={}", text.len());

    if text.is_empty() {
        return None;
    }

    let json: serde_json::Value = serde_json::from_str(&text).ok()?;

    json.get("displayName")
        .or_else(|| json.get("userName"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

fn read_secret(env_key: &str, file_env_key: &str) -> Result<String> {
    if let Ok(val) = std::env::var(env_key) {
        return Ok(val.trim().to_string());
    }
    if let Ok(path) = std::env::var(file_env_key) {
        let content = std::fs::read_to_string(&path)
            .map_err(|e| anyhow::anyhow!("Could not read {path}: {e}"))?;
        return Ok(content.trim().to_string());
    }
    bail!("{env_key} or {file_env_key} environment variable is required")
}
