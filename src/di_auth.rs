//! DI (Digital Identity) OAuth2 authentication for Garmin Connect.
//!
//! Replaces the deprecated `garmin_client` (garth-based) SSO flow that broke
//! when Garmin enabled Cloudflare TLS fingerprinting in March 2026.

use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{bail, Context, Result};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use regex::Regex;
use rquest_util::{Emulation, EmulationOS, EmulationOption};
use serde::{Deserialize, Serialize};
use serde_json::Value;

const SSO_HOST: &str = "sso.garmin.com";
const DI_TOKEN_URL: &str = "https://diauth.garmin.com/di-oauth2-service/oauth/token";
const DI_GRANT_TYPE: &str =
    "https://connectapi.garmin.com/di-oauth2-service/oauth/grant/service_ticket";
const SERVICE_URL: &str = "https://mobile.integration.garmin.com/gcm/android";
const SESSION_FILE: &str = ".di_session.json";

/// DI client IDs to try in order during ticket exchange (rotated quarterly).
const DI_CLIENT_IDS: &[&str] = &[
    "GARMIN_CONNECT_MOBILE_ANDROID_DI_2025Q2",
    "GARMIN_CONNECT_MOBILE_ANDROID_DI_2024Q4",
    "GARMIN_CONNECT_MOBILE_ANDROID_DI",
];

/// Mobile UA for the SSO HTML flow (kept as explicit override; the rquest
/// Chrome emulation also sets a matching UA for TLS-fingerprint consistency).
const USER_AGENT: &str = "Mozilla/5.0 (iPhone; CPU iPhone OS 17_4 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.4 Mobile/15E148 Safari/604.1";

/// Persisted DI OAuth2 session.
#[derive(Serialize, Deserialize, Clone)]
pub struct DiSession {
    pub access_token: String,
    pub refresh_token: String,
    /// Epoch seconds at which `access_token` expires.
    pub expires_at: u64,
    /// Epoch seconds at which `refresh_token` expires.
    pub refresh_expires_at: u64,
    pub client_id: String,
}

impl DiSession {
    /// True when the access token is within the 60-second safety margin of expiry.
    pub fn is_expired(&self) -> bool {
        now_secs() >= self.expires_at.saturating_sub(60)
    }

    /// True while the refresh token can still mint a new access token.
    pub fn refresh_is_valid(&self) -> bool {
        now_secs() < self.refresh_expires_at
    }
}

/// Outcome of the SSO username/password login.
pub struct SsoLoginResult {
    pub needs_mfa: bool,
    pub ticket: Option<String>,
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// Load the system CA certificate bundle so that rquest (BoringSSL) trusts
/// the same root CAs as the host OS.  This is essential in environments
/// where a proxy, VPN, or corporate firewall intercepts HTTPS with its own
/// CA — BoringSSL's built-in webpki roots won't include that CA, but the
/// system bundle (`/etc/ssl/certs/ca-certificates.crt` on Debian/Ubuntu)
/// does.
fn system_cert_store() -> Option<rquest::tls::CertStore> {
    const CA_PATHS: &[&str] = &[
        "/etc/ssl/certs/ca-certificates.crt", // Debian/Ubuntu
        "/etc/pki/tls/certs/ca-bundle.crt",   // RHEL/Fedora
        "/etc/ssl/cert.pem",                  // macOS / Alpine
    ];
    for path in CA_PATHS {
        if let Ok(store) = rquest::tls::CertStore::from_pem_file(path) {
            eprintln!("[di_auth] loaded system CA bundle from {path}");
            return Some(store);
        }
    }
    eprintln!("[di_auth] warning: no system CA bundle found; TLS verification may fail behind proxies/VPNs");
    None
}

/// Build an `rquest::Client` impersonating Chrome 131 on Android with a
/// persistent cookie store, so cookies (including Cloudflare's `__cflb`)
/// are shared across the whole SSO + DI token-exchange flow.
pub fn build_impersonated_client() -> Result<rquest::Client> {
    let mut builder = rquest::Client::builder()
        .emulation(
            EmulationOption::builder()
                .emulation(Emulation::Chrome131)
                .emulation_os(EmulationOS::Android)
                .build(),
        )
        .cookie_store(true)
        .user_agent(USER_AGENT);

    if let Some(store) = system_cert_store() {
        builder = builder.cert_store(store);
    }

    let client = builder
        .build()
        .context("failed to build rquest impersonated client")?;
    Ok(client)
}

/// Sign-in page query params. Garmin's embed widget expects every redirect
/// field to point back at the embed host.
fn signin_query_params() -> Vec<(&'static str, &'static str)> {
    let embed = "https://sso.garmin.com/sso/embed";
    vec![
        ("id", "gauth-widget"),
        ("embedWidget", "true"),
        ("gauthHost", embed),
        ("service", embed),
        ("source", embed),
        ("redirectAfterAccountLoginUrl", embed),
        ("redirectAfterAccountCreationUrl", embed),
    ]
}

fn signin_url() -> String {
    let qs: Vec<String> = signin_query_params()
        .iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect();
    format!("https://{SSO_HOST}/sso/signin?{}", qs.join("&"))
}

/// Extract the first `<title>...</title>` value from an HTML document.
fn extract_title(html: &str) -> Option<String> {
    let re = Regex::new(r"(?is)<title[^>]*>(.*?)</title>").ok()?;
    re.captures(html)
        .and_then(|c| c.get(1).map(|m| m.as_str().trim().to_string()))
}

/// Extract the CSRF token (`_csrf`) from a sign-in HTML page.
fn extract_csrf(html: &str) -> Result<String> {
    let re = Regex::new(r#"name="_csrf"\s+value="(\w+)""#).context("invalid csrf regex")?;
    let cap = re
        .captures(html)
        .and_then(|c| c.get(1).map(|m| m.as_str().to_string()))
        .context("could not find _csrf token in sign-in page")?;
    Ok(cap)
}

/// Extract the service ticket from an embed HTML page.
fn extract_ticket(html: &str) -> Result<String> {
    let re = Regex::new(r#"embed\?ticket=([^"&]+)"#).context("invalid ticket regex")?;
    let cap = re
        .captures(html)
        .and_then(|c| c.get(1).map(|m| m.as_str().to_string()))
        .context("could not find service ticket in embed page")?;
    Ok(cap)
}

/// GET the embed page that seeds the SSO cookies before the sign-in flow.
async fn prime_embed_cookies(client: &rquest::Client) -> Result<()> {
    let url = "https://sso.garmin.com/sso/embed?id=gauth-widget&embedWidget=true&gauthHost=https://sso.garmin.com/sso/embed";
    let resp = client.get(url).send().await.context("embed prime GET failed")?;
    let _ = resp.text().await.context("embed prime body read failed")?;
    Ok(())
}

/// GET the portal embed page that installs Cloudflare's `__cflb` cookie and
/// returns the page body (which carries the service ticket).
async fn portal_embed_body(client: &rquest::Client) -> Result<String> {
    let url = format!("https://{SSO_HOST}/portal/sso/embed");
    let resp = client
        .get(&url)
        .send()
        .await
        .with_context(|| format!("portal embed GET {url} failed"))?;
    resp.text()
        .await
        .with_context(|| format!("portal embed body read {url} failed"))
}

/// Run the SSO username/password login. Returns either a service ticket or
/// an indication that MFA is required.
pub async fn sso_login(
    client: &rquest::Client,
    email: &str,
    password: &str,
) -> Result<SsoLoginResult> {
    // 1. Seed SSO cookies.
    prime_embed_cookies(client).await?;

    // 2. Fetch the sign-in page and extract the CSRF token.
    let url = signin_url();
    let resp = client
        .get(&url)
        .send()
        .await
        .with_context(|| format!("signin page GET {url} failed"))?;
    let signin_html = resp
        .text()
        .await
        .with_context(|| format!("signin page body read {url} failed"))?;
    let csrf = extract_csrf(&signin_html)?;

    // 3. POST credentials to the sign-in endpoint.
    let form = [
        ("username", email.to_string()),
        ("password", password.to_string()),
        ("embed", "true".to_string()),
        ("_csrf", csrf),
    ];
    let resp = client
        .post(&url)
        .form(&form)
        .send()
        .await
        .with_context(|| format!("signin POST {url} failed"))?;
    let body = resp
        .text()
        .await
        .with_context(|| format!("signin POST body read {url} failed"))?;

    let title = extract_title(&body).unwrap_or_default();

    // 4. MFA required?
    if title.contains("MFA") {
        return Ok(SsoLoginResult {
            needs_mfa: true,
            ticket: None,
        });
    }

    // 5. Hard error on anything other than Success.
    if title != "Success" {
        bail!("Garmin SSO login failed (page title: {title:?})");
    }

    // 6. Install the Cloudflare LB cookie.
    let embed_html = portal_embed_body(client).await?;

    // 7. Parse the service ticket.
    let ticket = extract_ticket(&embed_html)?;
    Ok(SsoLoginResult {
        needs_mfa: false,
        ticket: Some(ticket),
    })
}

/// Submit the MFA code and return the resulting service ticket.
pub async fn submit_mfa(client: &rquest::Client, mfa_code: &str) -> Result<String> {
    // 1. Fresh CSRF token from the sign-in page.
    let url = signin_url();
    let resp = client
        .get(&url)
        .send()
        .await
        .with_context(|| format!("signin page GET {url} failed"))?;
    let signin_html = resp
        .text()
        .await
        .with_context(|| format!("signin page body read {url} failed"))?;
    let csrf = extract_csrf(&signin_html)?;

    // 2. POST the (trimmed!) MFA code.
    let form = [
        ("mfa-code", mfa_code.trim().to_string()),
        ("fromPage", "setupEnterMfaCode".to_string()),
        ("embed", "true".to_string()),
        ("_csrf", csrf),
    ];
    let verify_url = format!("https://{SSO_HOST}/sso/verifyMFA/loginEnterMfaCode");
    let resp = client
        .post(&verify_url)
        .form(&form)
        .send()
        .await
        .with_context(|| format!("MFA verify POST {verify_url} failed"))?;
    let body = resp
        .text()
        .await
        .with_context(|| format!("MFA verify body read {verify_url} failed"))?;

    let title = extract_title(&body).unwrap_or_default();
    if title != "Success" {
        bail!("Garmin MFA verification failed (page title: {title:?})");
    }

    // 3. Cloudflare LB cookie.
    let embed_html = portal_embed_body(client).await?;

    // 4. Parse the ticket.
    extract_ticket(&embed_html)
}

/// Exchange a service ticket for a DI OAuth2 session. Tries each client ID
/// in turn until one succeeds.
pub async fn exchange_service_ticket(ticket: &str) -> Result<DiSession> {
    // The DI token endpoint is a standard OAuth2 endpoint on diauth.garmin.com.
    // It does NOT need TLS fingerprint impersonation (unlike sso.garmin.com which
    // is behind Cloudflare).  Using a plain rquest client avoids potential TLS
    // handshake issues caused by Chrome emulation on this endpoint.
    let mut builder = rquest::Client::builder().cookie_store(true);
    if let Some(store) = system_cert_store() {
        builder = builder.cert_store(store);
    }
    let client = builder
        .build()
        .context("failed to build plain rquest client for DI exchange")?;

    let mut last_err: Option<anyhow::Error> = None;
    for &client_id in DI_CLIENT_IDS {
        match try_exchange(&client, client_id, ticket).await {
            Ok(session) => return Ok(session),
            Err(e) => {
                eprintln!("[di_auth] ticket exchange with {client_id} failed: {e:#}");
                last_err = Some(e);
            }
        }
    }
    Err(last_err.unwrap_or_else(|| anyhow::anyhow!("no DI client IDs configured")))
}

async fn try_exchange(
    client: &rquest::Client,
    client_id: &str,
    ticket: &str,
) -> Result<DiSession> {
    let basic = STANDARD.encode(format!("{client_id}:"));
    let form = [
        ("client_id", client_id.to_string()),
        ("service_ticket", ticket.to_string()),
        ("grant_type", DI_GRANT_TYPE.to_string()),
        ("service_url", SERVICE_URL.to_string()),
    ];

    let resp = client
        .post(DI_TOKEN_URL)
        .header("Authorization", format!("Basic {basic}"))
        .header("Accept", "application/json,text/html;q=0.9,*/*;q=0.8")
        .header("Cache-Control", "no-cache")
        .form(&form)
        .send()
        .await
        .with_context(|| format!("DI token POST {DI_TOKEN_URL} failed"))?;

    let status = resp.status();
    let text = resp
        .text()
        .await
        .with_context(|| format!("DI token body read {DI_TOKEN_URL} failed"))?;

    if !status.is_success() {
        bail!("DI token exchange failed ({status}): {}", preview(&text));
    }

    parse_di_session(&text, client_id)
}

/// Refresh an expired access token using the refresh token.
pub async fn refresh_di_token(session: &DiSession) -> Result<DiSession> {
    let mut builder = rquest::Client::builder().cookie_store(true);
    if let Some(store) = system_cert_store() {
        builder = builder.cert_store(store);
    }
    let client = builder
        .build()
        .context("failed to build plain rquest client for DI refresh")?;
    let basic = STANDARD.encode(format!("{}:", session.client_id));
    let form = [
        ("client_id", session.client_id.clone()),
        ("refresh_token", session.refresh_token.clone()),
        ("grant_type", "refresh_token".to_string()),
    ];

    let resp = client
        .post(DI_TOKEN_URL)
        .header("Authorization", format!("Basic {basic}"))
        .header("Accept", "application/json,text/html;q=0.9,*/*;q=0.8")
        .header("Cache-Control", "no-cache")
        .form(&form)
        .send()
        .await
        .with_context(|| format!("DI refresh POST {DI_TOKEN_URL} failed"))?;

    let status = resp.status();
    let text = resp
        .text()
        .await
        .with_context(|| format!("DI refresh body read {DI_TOKEN_URL} failed"))?;

    if !status.is_success() {
        bail!("DI token refresh failed ({status}): {}", preview(&text));
    }

    parse_di_session(&text, &session.client_id)
}

/// Parse a DI token JSON response into a `DiSession`, computing absolute
/// expiry timestamps from the relative `expires_in` / `refresh_expires_in`.
fn parse_di_session(text: &str, client_id: &str) -> Result<DiSession> {
    let v: Value = serde_json::from_str(text)
        .with_context(|| format!("DI token response is not JSON: {}", preview(text)))?;

    let access_token = v
        .get("access_token")
        .and_then(Value::as_str)
        .context("DI token response missing access_token")?
        .to_string();
    let refresh_token = v
        .get("refresh_token")
        .and_then(Value::as_str)
        .context("DI token response missing refresh_token")?
        .to_string();

    let expires_in = v.get("expires_in").and_then(Value::as_u64).unwrap_or(3600);
    let refresh_expires_in = v
        .get("refresh_expires_in")
        .and_then(Value::as_u64)
        .unwrap_or(2_592_000); // ~30 days

    let now = now_secs();
    Ok(DiSession {
        access_token,
        refresh_token,
        expires_at: now + expires_in,
        refresh_expires_at: now + refresh_expires_in,
        client_id: client_id.to_string(),
    })
}

fn preview(text: &str) -> String {
    text.chars().take(200).collect()
}

/// Persist a session to `.di_session.json`.
pub fn save_session(session: &DiSession) -> Result<()> {
    let json = serde_json::to_string_pretty(session).context("failed to serialize session")?;
    std::fs::write(SESSION_FILE, json).with_context(|| format!("failed to write {SESSION_FILE}"))?;
    Ok(())
}

/// Load a previously persisted session.
pub fn load_session() -> Result<DiSession> {
    let text =
        std::fs::read_to_string(SESSION_FILE).with_context(|| format!("failed to read {SESSION_FILE}"))?;
    let session: DiSession =
        serde_json::from_str(&text).with_context(|| format!("failed to parse {SESSION_FILE}"))?;
    Ok(session)
}

/// Read a secret from an env var, falling back to a `_FILE` env var whose
/// value is a path to a file containing the secret.
pub fn read_secret(env_key: &str, file_key: &str) -> Result<String> {
    if let Ok(val) = std::env::var(env_key) {
        return Ok(val.trim().to_string());
    }
    if let Ok(path) = std::env::var(file_key) {
        let content = std::fs::read_to_string(&path)
            .map_err(|e| anyhow::anyhow!("could not read {path}: {e}"))?;
        return Ok(content.trim().to_string());
    }
    bail!("{env_key} or {file_key} environment variable is required")
}

/// Read the MFA code from `GARMIN_MFA_CODE` env, otherwise from stdin.
pub fn read_mfa_code() -> Result<String> {
    if let Ok(val) = std::env::var("GARMIN_MFA_CODE") {
        return Ok(val.trim().to_string());
    }
    eprintln!("MFA code required: enter it and press Enter:");
    let mut input = String::new();
    std::io::stdin()
        .read_line(&mut input)
        .context("failed to read MFA code from stdin")?;
    Ok(input.trim().to_string())
}

/// Three-layer authentication fallback:
///   1. Cached session with a valid refresh token (refresh if expired).
///   2. `GARMIN_SERVICE_TICKET` env var → exchange for a session.
///   3. `GARMIN_EMAIL` + `GARMIN_PASSWORD` (→ MFA if needed) → exchange.
pub async fn authenticate() -> Result<DiSession> {
    // Layer 1: cached session.
    if let Ok(session) = load_session() {
        if session.refresh_is_valid() {
            if session.is_expired() {
                eprintln!("[di_auth] cached access token expired; refreshing...");
                match refresh_di_token(&session).await {
                    Ok(new_session) => {
                        let _ = save_session(&new_session);
                        return Ok(new_session);
                    }
                    Err(e) => {
                        eprintln!("[di_auth] refresh failed ({e}); falling through to fresh login");
                    }
                }
            } else {
                eprintln!("[di_auth] using cached DI session (valid)");
                return Ok(session);
            }
        } else {
            eprintln!("[di_auth] cached session's refresh token is expired; fresh login needed");
        }
    }

    // Layer 2: service ticket from env.
    if let Ok(ticket) = std::env::var("GARMIN_SERVICE_TICKET") {
        let ticket = ticket.trim();
        if !ticket.is_empty() {
            eprintln!("[di_auth] exchanging GARMIN_SERVICE_TICKET for DI session...");
            let session = exchange_service_ticket(ticket).await?;
            save_session(&session)?;
            return Ok(session);
        }
    }

    // Layer 3: email + password (optionally MFA).
    let email = read_secret("GARMIN_EMAIL", "GARMIN_EMAIL_FILE")?;
    let password = read_secret("GARMIN_PASSWORD", "GARMIN_PASSWORD_FILE")?;

    eprintln!("[di_auth] authenticating with Garmin SSO ({SSO_HOST})...");
    let client = build_impersonated_client()?;
    let result = sso_login(&client, &email, &password).await?;

    let ticket = if result.needs_mfa {
        eprintln!("[di_auth] MFA required.");
        let code = read_mfa_code()?;
        submit_mfa(&client, &code).await?
    } else {
        result
            .ticket
            .context("SSO login succeeded but no service ticket was returned")?
    };

    eprintln!("[di_auth] exchanging service ticket for DI session...");
    let session = exchange_service_ticket(&ticket).await?;
    save_session(&session)?;
    Ok(session)
}

