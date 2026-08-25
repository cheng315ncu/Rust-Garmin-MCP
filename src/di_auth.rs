//! DI (Digital Identity) OAuth2 authentication for Garmin Connect.
//!
//! Replaces the deprecated `garmin_client` (garth-based) SSO flow that broke
//! when Garmin enabled Cloudflare TLS fingerprinting in March 2026.

use std::sync::OnceLock;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

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
const SERVICE_URL: &str = "https://sso.garmin.com/sso/embed";
const SESSION_FILE: &str = ".di_session.json";

/// DI client IDs to try in order during ticket exchange (rotated quarterly).
const DI_CLIENT_IDS: &[&str] = &[
    "GARMIN_CONNECT_MOBILE_ANDROID_DI_2025Q2",
    "GARMIN_CONNECT_MOBILE_ANDROID_DI_2024Q4",
    "GARMIN_CONNECT_MOBILE_ANDROID_DI",
];

/// User-Agent for every impersonated request.  It must agree with the TLS /
/// HTTP2 fingerprint of `Emulation::Chrome131` + `EmulationOS::Android` and
/// with the `sec-ch-ua*` client hints that emulation installs, or the three
/// together are a stronger bot signal than no emulation at all.
///
/// We override the emulation's own UA because rquest-util 2.2.1 ships a
/// malformed literal for this arm — `Mozilla/5.0 (Linux: Android 10; K) …
/// Chrome/131.0.0.0 Safari/537.36`, with a colon after `Linux` and no `Mobile`
/// token (see `rquest-util/src/emulation/device/chrome.rs`, the `v131` Android
/// tuple).  The string below is what real Chrome 131 on Android sends.
///
/// Order matters: `.user_agent()` must stay AFTER `.emulation()`.  `emulation()`
/// `mem::swap`s the whole header map, so a UA set before it is discarded.
const USER_AGENT: &str = "Mozilla/5.0 (Linux; Android 10; K) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Mobile Safari/537.36";

/// Request timeout for every Garmin HTTP call.  rquest's builder default is
/// `timeout: None`; an unbounded DI refresh is what wedges the client layer.
const HTTP_TIMEOUT: Duration = Duration::from_secs(30);

/// Fallback access-token lifetime when the DI response omits `expires_in`.
const DEFAULT_ACCESS_TTL_SECS: u64 = 3_600;

/// Fallback refresh-token lifetime (~30 days), used only when the DI response
/// omits `refresh_expires_in` and there is no previous value to carry forward.
const DEFAULT_REFRESH_TTL_SECS: u64 = 2_592_000;

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
    /// Garmin account this session was minted for, lowercased, so a cached
    /// session is never reused after `GARMIN_EMAIL` changes.  `None` in
    /// session files written before this field existed.
    #[serde(default)]
    pub account: Option<String>,
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
    /// CSRF token extracted from the MFA page (when `needs_mfa` is true).
    pub mfa_csrf: Option<String>,
    /// The MFA page URL (with query params) — the form has no `action`
    /// attribute, so it submits back to this URL.
    pub mfa_url: Option<String>,
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
        .user_agent(USER_AGENT)
        .redirect(rquest::redirect::Policy::limited(10))
        .pool_idle_timeout(Some(std::time::Duration::from_secs(5)))
        .pool_max_idle_per_host(0)
        .timeout(HTTP_TIMEOUT);

    if let Some(store) = system_cert_store() {
        builder = builder.cert_store(store);
    }

    let client = builder
        .build()
        .context("failed to build rquest impersonated client")?;
    Ok(client)
}

/// The DI token endpoint is a standard OAuth2 endpoint on `diauth.garmin.com`.
/// It does NOT need TLS fingerprint impersonation (unlike `sso.garmin.com`,
/// which is behind Cloudflare) — Chrome emulation causes handshake trouble
/// there.  Do not unify this with `build_impersonated_client`.
///
/// One process-wide client: the CA bundle is parsed once and the connection is
/// reused across the initial exchange and every later refresh.  Crucially it
/// carries `HTTP_TIMEOUT`, without which a half-open connection to
/// diauth.garmin.com hangs the refresh forever.
fn plain_di_client() -> Result<rquest::Client> {
    static CLIENT: OnceLock<rquest::Client> = OnceLock::new();

    if let Some(client) = CLIENT.get() {
        return Ok(client.clone());
    }

    let mut builder = rquest::Client::builder()
        .cookie_store(true)
        .timeout(HTTP_TIMEOUT);
    if let Some(store) = system_cert_store() {
        builder = builder.cert_store(store);
    }
    let client = builder
        .build()
        .context("failed to build plain rquest client for the DI token endpoint")?;

    Ok(CLIENT.get_or_init(|| client).clone())
}

/// Sign-in page query params. Garmin's embed widget expects every redirect
/// field to point back at the embed host. The `service` parameter is
/// overridden by the server to match `gauthHost`/`redirectAfterAccountLoginUrl`.
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

/// Extract the service ticket from an embed/success page. The ticket appears
/// in a `response_url` JavaScript variable as `...?ticket=ST-...`.
fn extract_ticket(html: &str) -> Result<String> {
    let re = Regex::new(r#"[?&]ticket=([^"&]+)"#).context("invalid ticket regex")?;
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
    let post_url = resp.url().to_string();
    let body = resp
        .text()
        .await
        .with_context(|| format!("signin POST body read {url} failed"))?;

    let title = extract_title(&body).unwrap_or_default();

    // 4. MFA required?
    if title.contains("MFA") {
        let csrf = extract_csrf(&body).ok();
        return Ok(SsoLoginResult {
            needs_mfa: true,
            ticket: None,
            mfa_csrf: csrf,
            mfa_url: Some(post_url),
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
        mfa_csrf: None,
        mfa_url: None,
    })
}

/// Submit the MFA code and return the resulting service ticket.
///
/// `csrf` is the CSRF token extracted from the MFA page (passed in from
/// `sso_login`) — we do NOT re-GET the signin page here because the session
/// is already in the MFA state and a fresh GET may fail or return a
/// different page.
///
/// `mfa_url` is the MFA page URL (with query params). The form has no
/// `action` attribute, so it submits back to the same URL.
pub async fn submit_mfa(
    client: &rquest::Client,
    mfa_code: &str,
    csrf: &str,
    mfa_url: &str,
) -> Result<String> {
    // 1. POST the (trimmed!) MFA code with the CSRF from the MFA page.
    let form = [
        ("mfa-code", mfa_code.trim().to_string()),
        ("fromPage", "setupEnterMfaCode".to_string()),
        ("embed", "true".to_string()),
        ("_csrf", csrf.to_string()),
    ];
    let resp = client
        .post(mfa_url)
        .form(&form)
        .send()
        .await
        .with_context(|| format!("MFA verify POST {mfa_url} failed"))?;
    let body = resp
        .text()
        .await
        .with_context(|| format!("MFA verify body read {mfa_url} failed"))?;

    let title = extract_title(&body).unwrap_or_default();
    if title != "Success" {
        bail!("Garmin MFA verification failed (page title: {title:?})");
    }

    // 2. Try to extract the ticket from the MFA POST response body first.
    //    The Success page contains `response_url = "...?ticket=ST-..."`.
    if let Ok(ticket) = extract_ticket(&body) {
        return Ok(ticket);
    }

    // 3. Fall back: get the Cloudflare LB cookie + embed page.
    let embed_html = portal_embed_body(client).await?;
    extract_ticket(&embed_html)
}

/// Exchange a service ticket for a DI OAuth2 session. Tries each client ID
/// in turn until one succeeds.
pub async fn exchange_service_ticket(ticket: &str) -> Result<DiSession> {
    let client = plain_di_client()?;

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

    parse_di_session(&text, client_id, None)
}

/// Refresh an expired access token using the refresh token.
pub async fn refresh_di_token(session: &DiSession) -> Result<DiSession> {
    let client = plain_di_client()?;
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

    parse_di_session(&text, &session.client_id, Some(session))
}

/// Parse a DI token JSON response into a `DiSession`, computing absolute
/// expiry timestamps from the relative `expires_in` / `refresh_expires_in`.
///
/// `prev` is the session being refreshed, when there is one.  RFC 6749 §6 makes
/// `refresh_token` OPTIONAL in a refresh response: a provider with rotation
/// disabled returns only a new access token, and the existing refresh token
/// stays valid.  Treating that as an error would break every refresh, so the
/// previous token and its expiry are carried forward instead.
fn parse_di_session(text: &str, client_id: &str, prev: Option<&DiSession>) -> Result<DiSession> {
    let v: Value = serde_json::from_str(text)
        .with_context(|| format!("DI token response is not JSON: {}", preview(text)))?;

    let access_token = v
        .get("access_token")
        .and_then(Value::as_str)
        .context("DI token response missing access_token")?
        .to_string();

    let now = now_secs();
    let refresh_expires_in = v.get("refresh_expires_in").and_then(Value::as_u64);

    let (refresh_token, refresh_expires_at) = match v.get("refresh_token").and_then(Value::as_str) {
        Some(token) => (
            token.to_string(),
            now + refresh_expires_in.unwrap_or(DEFAULT_REFRESH_TTL_SECS),
        ),
        None => {
            // No prior session means this was an initial exchange, where the
            // refresh token really is mandatory.
            let prev = prev.context("DI token response missing refresh_token")?;
            (
                prev.refresh_token.clone(),
                // Only move the deadline when the server actually restated it;
                // otherwise keep the real one rather than optimistically
                // extending the window by another 30 days.
                refresh_expires_in.map_or(prev.refresh_expires_at, |ttl| now + ttl),
            )
        }
    };

    let expires_in = v
        .get("expires_in")
        .and_then(Value::as_u64)
        .unwrap_or(DEFAULT_ACCESS_TTL_SECS);

    Ok(DiSession {
        access_token,
        refresh_token,
        expires_at: now + expires_in,
        refresh_expires_at,
        client_id: client_id.to_string(),
        account: prev.and_then(|p| p.account.clone()),
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

/// Read the MFA code from `GARMIN_MFA_CODE` env, a file pointed to by
/// `GARMIN_MFA_CODE_FILE` (polled for non-interactive/agent runs), or stdin.
pub fn read_mfa_code() -> Result<String> {
    if let Ok(val) = std::env::var("GARMIN_MFA_CODE") {
        return Ok(val.trim().to_string());
    }

    // File-based fallback with polling: lets an external agent inject the
    // MFA code into a running test by writing it to a file (stdin is not
    // reachable when the test is spawned in the background).
    if let Ok(path) = std::env::var("GARMIN_MFA_CODE_FILE") {
        eprintln!("[di_auth] waiting for MFA code in file: {path}");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(300);
        while std::time::Instant::now() < deadline {
            if let Ok(content) = std::fs::read_to_string(&path) {
                let code = content.trim();
                if !code.is_empty() {
                    let _ = std::fs::remove_file(&path);
                    return Ok(code.to_string());
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(500));
        }
        bail!("timed out waiting for MFA code in {path}");
    }

    eprintln!("MFA code required: enter it and press Enter:");
    let mut input = String::new();
    std::io::stdin()
        .read_line(&mut input)
        .context("failed to read MFA code from stdin")?;
    Ok(input.trim().to_string())
}

/// The account a cached session must belong to: `GARMIN_EMAIL` (or its `_FILE`
/// variant), lowercased.  `None` when neither is set — e.g. a
/// `GARMIN_SERVICE_TICKET`-only deployment, where there is nothing to compare
/// against.
fn configured_account() -> Option<String> {
    read_secret("GARMIN_EMAIL", "GARMIN_EMAIL_FILE")
        .ok()
        .map(|email| email.trim().to_lowercase())
        .filter(|email| !email.is_empty())
}

/// A cached session may only be reused when it was minted for the account this
/// process is configured with.  Without this check, editing `GARMIN_EMAIL` and
/// restarting keeps serving the *previous* account's health data for as long as
/// its refresh token lives (~30 days), because layer 1 never reads the env at
/// all.
///
/// Sessions written before `account` existed carry `None`; they are rejected
/// whenever an account IS configured, since we cannot prove they belong to it
/// and one extra login is cheaper than the wrong person's data.
fn session_matches_account(session: &DiSession, want: Option<&str>) -> bool {
    match (session.account.as_deref(), want) {
        (_, None) => true,
        (Some(have), Some(want)) if have == want => true,
        (Some(have), Some(want)) => {
            eprintln!(
                "[di_auth] cached session belongs to {have}, but the configured account is {want}; re-authenticating"
            );
            false
        }
        (None, Some(_)) => {
            eprintln!(
                "[di_auth] cached session predates account binding; re-authenticating to confirm ownership"
            );
            false
        }
    }
}

/// Three-layer authentication fallback:
///   1. Cached session with a valid refresh token (refresh if expired).
///   2. `GARMIN_SERVICE_TICKET` env var → exchange for a session.
///   3. `GARMIN_EMAIL` + `GARMIN_PASSWORD` (→ MFA if needed) → exchange.
pub async fn authenticate() -> Result<DiSession> {
    let want_account = configured_account();

    // Layer 1: cached session, but only if it belongs to the configured account.
    if let Ok(session) = load_session() {
        if session_matches_account(&session, want_account.as_deref()) {
            if !session.refresh_is_valid() {
                eprintln!(
                    "[di_auth] cached session's refresh token is expired; fresh login needed"
                );
            } else if session.is_expired() {
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
        }
    }

    // Layer 2: service ticket from env.
    if let Ok(ticket) = std::env::var("GARMIN_SERVICE_TICKET") {
        let ticket = ticket.trim();
        if !ticket.is_empty() {
            eprintln!("[di_auth] exchanging GARMIN_SERVICE_TICKET for DI session...");
            match exchange_service_ticket(ticket).await {
                Ok(mut session) => {
                    session.account = want_account.clone();
                    save_session(&session)?;
                    return Ok(session);
                }
                Err(e) => {
                    eprintln!(
                        "[di_auth] service ticket exchange failed ({e:#}); \
                         falling through to email/password login"
                    );
                }
            }
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
        let csrf = result
            .mfa_csrf
            .context("MFA required but no CSRF token was extracted from the MFA page")?;
        let mfa_url = result
            .mfa_url
            .context("MFA required but no MFA page URL was captured")?;
        submit_mfa(&client, &code, &csrf, &mfa_url).await?
    } else {
        result
            .ticket
            .context("SSO login succeeded but no service ticket was returned")?
    };

    eprintln!("[di_auth] exchanging service ticket for DI session...");
    let mut session = exchange_service_ticket(&ticket).await?;
    session.account = Some(email.trim().to_lowercase());
    save_session(&session)?;
    Ok(session)
}
