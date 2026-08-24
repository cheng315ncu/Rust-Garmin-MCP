use anyhow::Result;

use crate::client::GarminApiClient;
use crate::di_auth;
use crate::tools::GarminMcpServer;

pub async fn create_garmin_server() -> Result<GarminMcpServer> {
    let _ = dotenvy::dotenv();

    let session = di_auth::authenticate().await?;

    let display_name = resolve_display_name(&session.access_token).await;
    if display_name.is_empty() {
        eprintln!("Warning: display name is empty.");
        eprintln!("  Set GARMIN_DISPLAY_NAME=<handle> in .env to override.");
    } else {
        eprintln!("Logged in as: {display_name}");
    }

    let api = GarminApiClient::new(session, display_name);
    Ok(GarminMcpServer::new(api))
}

/// Try env var override first, then probe multiple Garmin API endpoints with
/// the DI access token as a Bearer credential.
async fn resolve_display_name(access_token: &str) -> String {
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

    let client = di_auth::build_impersonated_client().unwrap_or_else(|e| {
        eprintln!("[auth] warning: could not build probe client ({e})");
        rquest::Client::new()
    });

    for endpoint in &endpoints {
        if let Some(name) = try_display_name(&client, endpoint, access_token).await {
            return name;
        }
    }

    String::new()
}

async fn try_display_name(
    client: &rquest::Client,
    endpoint: &str,
    access_token: &str,
) -> Option<String> {
    let endpoint = endpoint.trim_start_matches('/');
    let url = format!("{}/{}", crate::client::API_BASE, endpoint);

    let resp = client
        .get(&url)
        .header("Authorization", format!("Bearer {access_token}"))
        .header("NK", "NT")
        .header("X-app-ver", "4.70.2.0")
        .header("Accept", "application/json")
        .send()
        .await
        .ok()?;

    let status = resp.status();
    let text = resp.text().await.ok()?;

    eprintln!("[auth] {endpoint} -> status={status}, body_len={}", text.len());

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
