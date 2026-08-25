use anyhow::Result;

use crate::client::{self, GarminApiClient};
use crate::di_auth;
use crate::tools::user_profile;
use crate::tools::GarminMcpServer;

pub async fn create_garmin_client() -> Result<GarminApiClient> {
    let _ = dotenvy::dotenv();

    // One impersonated client for the whole process. The SSO login, the
    // display-name probe and every API call then share its cookie jar — the
    // Cloudflare `__cflb` cookie earned during login has to reach connectapi —
    // and its connection pool.
    let http = di_auth::build_impersonated_client()?;

    let session = di_auth::authenticate(&http).await?;

    let display_name = resolve_display_name(&http, &session.access_token).await;
    if display_name.is_empty() {
        eprintln!("Warning: display name is empty.");
        eprintln!("  Set GARMIN_DISPLAY_NAME=<handle> in .env to override.");
    } else {
        eprintln!("Logged in as: {display_name}");
    }

    Ok(GarminApiClient::new(http, session, display_name))
}

pub async fn create_garmin_server() -> Result<GarminMcpServer> {
    let api = create_garmin_client().await?;
    Ok(GarminMcpServer::new(api))
}

/// Try env var override first, then probe multiple Garmin API endpoints with
/// the DI access token as a Bearer credential.
async fn resolve_display_name(client: &rquest::Client, access_token: &str) -> String {
    if let Some(name) = di_auth::non_empty_env("GARMIN_DISPLAY_NAME") {
        return name;
    }

    // socialProfile first — it is the only one of the three that answers today
    // (the other two 404, confirmed live; see src/tools/user_profile.rs). They
    // stay as fallbacks in case Garmin restores them, but probing them first
    // spent a guaranteed-404 Cloudflare round-trip on every process start.
    let endpoints = [
        user_profile::SOCIAL_PROFILE,
        "/userprofile-service/userprofile/v2/information",
        "/userprofile-service/userprofile",
    ];

    for endpoint in &endpoints {
        if let Some(name) = try_display_name(client, endpoint, access_token).await {
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
    let url = client::api_url(endpoint);

    let resp = client::garmin_headers(client.get(&url), access_token)
        .send()
        .await
        .ok()?;

    let status = resp.status();
    let text = resp.text().await.ok()?;

    eprintln!(
        "[auth] {endpoint} -> status={status}, body_len={}",
        text.len()
    );

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
