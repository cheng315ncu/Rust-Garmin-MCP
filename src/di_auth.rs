//! DI (Digital Identity) OAuth2 authentication for Garmin Connect.
//!
//! Replaces the deprecated `garmin_client` (garth-based) SSO flow that broke
//! when Garmin enabled Cloudflare TLS fingerprinting in March 2026.

use std::path::PathBuf;
use std::sync::{LazyLock, OnceLock};
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

/// How long an idle pooled connection may be reused. Short, because Garmin and
/// Cloudflare drop server-side sockets well before rquest's 90s default.
const POOL_IDLE_TIMEOUT: Duration = Duration::from_secs(30);

/// How long `read_mfa_code` polls `GARMIN_MFA_CODE_FILE` before giving up.
const MFA_WAIT: Duration = Duration::from_secs(300);

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
///
/// Parsed once per process: the bundle is ~200KB, and `ClientBuilder::cert_store`
/// accepts `Option<&'static CertStore>` (borrowed, no clone or re-parse).
fn system_cert_store() -> Option<&'static rquest::tls::CertStore> {
    static STORE: OnceLock<Option<rquest::tls::CertStore>> = OnceLock::new();

    STORE
        .get_or_init(|| {
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
        })
        .as_ref()
}

/// Build an `rquest::Client` impersonating Chrome 131 on Android with a
/// persistent cookie store, so cookies (including Cloudflare's `__cflb`)
/// are shared across the whole SSO + DI token-exchange flow.
pub fn build_impersonated_client() -> Result<rquest::Client> {
    rquest::Client::builder()
        .emulation(
            EmulationOption::builder()
                .emulation(Emulation::Chrome131)
                .emulation_os(EmulationOS::Android)
                .build(),
        )
        .cookie_store(true)
        // Must follow `.emulation()`; see USER_AGENT.
        .user_agent(USER_AGENT)
        // rquest's builder default is `Policy::none()` despite what its own doc
        // comment says, and the SSO credential POST 302s to the MFA page.
        .redirect(rquest::redirect::Policy::limited(10))
        // Expire idle connections quickly — Garmin/Cloudflare drop sockets left
        // idle across an MFA pause, and reusing a dead one hangs. This replaces
        // `pool_max_idle_per_host(0)`, which disabled pooling outright and made
        // every one of the 77 tools' requests pay a fresh TLS handshake.
        .pool_idle_timeout(POOL_IDLE_TIMEOUT)
        .timeout(HTTP_TIMEOUT)
        .cert_store(system_cert_store())
        .build()
        .context("failed to build rquest impersonated client")
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

    let client = rquest::Client::builder()
        .cookie_store(true)
        .timeout(HTTP_TIMEOUT)
        .pool_idle_timeout(POOL_IDLE_TIMEOUT)
        .cert_store(system_cert_store())
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

static TITLE_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?is)<title[^>]*>(.*?)</title>").expect("valid title regex"));

/// `_csrf` inside a single input tag, in either attribute order.
///
/// Deliberately looser than it looks like it could be: the token is not
/// restricted to `\w` (Garmin has emitted base64- and UUID-shaped values, which
/// contain `-`, `_`, `=` and `.`), attributes may be quoted with either quote
/// character, and `value` does not have to be the attribute right after `name`.
/// A pattern that assumes otherwise does not truncate — it fails to match at
/// all, and the whole login aborts on a perfectly valid page.
static CSRF_RES: LazyLock<[Regex; 2]> = LazyLock::new(|| {
    [
        Regex::new(r#"(?is)name=["']_csrf["'][^>]*?value=["']([^"']+)["']"#)
            .expect("valid csrf regex"),
        Regex::new(r#"(?is)value=["']([^"']+)["'][^>]*?name=["']_csrf["']"#)
            .expect("valid csrf regex"),
    ]
});

/// The service ticket in a `response_url` JS variable: `...?ticket=ST-...`.
///
/// The terminator set has to include the single quote, whitespace, `<` and `\`
/// as well as `"` and `&` — a ticket inside a single-quoted JS string would
/// otherwise be captured together with everything up to the next double quote,
/// and the exchange would fail three times over with a misleading error.
static TICKET_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"[?&]ticket=([^"'&\s<\\]+)"#).expect("valid ticket regex"));

/// Extract the first `<title>...</title>` value from an HTML document.
fn extract_title(html: &str) -> Option<String> {
    TITLE_RE
        .captures(html)
        .and_then(|c| c.get(1).map(|m| m.as_str().trim().to_string()))
}

/// Extract the CSRF token (`_csrf`) from a sign-in HTML page.
fn extract_csrf(html: &str) -> Result<String> {
    CSRF_RES
        .iter()
        .find_map(|re| re.captures(html))
        .and_then(|c| c.get(1).map(|m| m.as_str().to_string()))
        .context("could not find _csrf token in sign-in page")
}

/// Extract the service ticket from an embed/success page.
fn extract_ticket(html: &str) -> Result<String> {
    TICKET_RE
        .captures(html)
        .and_then(|c| c.get(1).map(|m| m.as_str().to_string()))
        .context("could not find service ticket in embed page")
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
    // These traces are the diagnosis path when Garmin changes its markup: the
    // regexes below are the first thing to break, and a body length plus a page
    // title distinguishes "markup moved" from "Cloudflare blocked us".
    eprintln!(
        "[di_auth] signin page: {} bytes, title={:?}",
        signin_html.len(),
        extract_title(&signin_html).unwrap_or_default()
    );
    let csrf = extract_csrf(&signin_html).inspect_err(|_| {
        eprintln!("[di_auth] signin page preview: {}", preview(&signin_html));
    })?;

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
    eprintln!(
        "[di_auth] signin POST -> {post_url} ({} bytes, title={title:?})",
        body.len()
    );

    // 4. MFA required?
    if title.contains("MFA") {
        let csrf = extract_csrf(&body).ok();
        if csrf.is_none() {
            eprintln!(
                "[di_auth] warning: MFA page carried no _csrf token; preview: {}",
                preview(&body)
            );
        }
        return Ok(SsoLoginResult {
            needs_mfa: true,
            ticket: None,
            mfa_csrf: csrf,
            mfa_url: Some(post_url),
        });
    }

    // 5. Hard error on anything other than Success.
    if title != "Success" {
        eprintln!("[di_auth] signin body preview: {}", preview(&body));
        bail!("Garmin SSO login failed (page title: {title:?})");
    }

    // 6. Install the Cloudflare LB cookie.
    let embed_html = portal_embed_body(client).await?;

    // 7. Parse the service ticket.
    let ticket = extract_ticket(&embed_html).inspect_err(|_| {
        eprintln!(
            "[di_auth] portal embed page: {} bytes; preview: {}",
            embed_html.len(),
            preview(&embed_html)
        );
    })?;
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

pub(crate) fn preview(text: &str) -> String {
    text.chars().take(200).collect()
}

/// Where the DI session cache lives.
///
/// `SESSION_FILE` is CWD-relative, and a stdio MCP server launched by a desktop
/// client inherits whatever CWD that client had — often `/`. `GARMIN_SESSION_FILE`
/// pins it to an absolute path.
fn session_path() -> PathBuf {
    non_empty_env("GARMIN_SESSION_FILE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(SESSION_FILE))
}

/// Persist a session, readable only by its owner.
///
/// The file holds a ~30 day refresh token; `fs::write` would create it 0644
/// under a typical umask, i.e. readable by every local account.
pub fn save_session(session: &DiSession) -> Result<()> {
    let path = session_path();
    let json = serde_json::to_string_pretty(session).context("failed to serialize session")?;

    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }

    let mut file = opts
        .open(&path)
        .with_context(|| format!("failed to open {} for writing", path.display()))?;

    // `mode()` only applies to a file this call creates, so tighten an existing
    // one (e.g. written by a version before this).
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = file.set_permissions(std::fs::Permissions::from_mode(0o600));
    }

    std::io::Write::write_all(&mut file, json.as_bytes())
        .with_context(|| format!("failed to write {}", path.display()))
}

/// Persist a session, downgrading a failure to a warning.
///
/// A read-only CWD must not throw away a session that was just obtained at the
/// cost of a full SSO + MFA round trip — it only means the next start logs in
/// again.
pub fn persist_session(session: &DiSession) {
    if let Err(e) = save_session(session) {
        eprintln!("[di_auth] warning: could not persist session ({e:#}); the next start will log in again");
    }
}

/// Load a previously persisted session.
pub fn load_session() -> Result<DiSession> {
    let path = session_path();
    let text = std::fs::read_to_string(&path)
        .with_context(|| format!("failed to read {}", path.display()))?;
    serde_json::from_str(&text).with_context(|| format!("failed to parse {}", path.display()))
}

/// An env var's trimmed value, or `None` when it is unset *or* set to an empty
/// string.
///
/// `FOO=` is a placeholder, not an answer — a `.env` full of empty keys must
/// fall through to the `_FILE` and stdin fallbacks instead of submitting empty
/// credentials and blaming Garmin for the rejection.
pub(crate) fn non_empty_env(key: &str) -> Option<String> {
    std::env::var(key)
        .ok()
        .map(|val| val.trim().to_string())
        .filter(|val| !val.is_empty())
}

/// Read a secret from an env var, falling back to a `_FILE` env var whose
/// value is a path to a file containing the secret.
pub fn read_secret(env_key: &str, file_key: &str) -> Result<String> {
    if let Some(val) = non_empty_env(env_key) {
        return Ok(val);
    }
    if let Some(path) = non_empty_env(file_key) {
        let content = std::fs::read_to_string(&path)
            .map_err(|e| anyhow::anyhow!("could not read {path}: {e}"))?;
        let content = content.trim();
        if content.is_empty() {
            bail!("{file_key} points at {path}, which is empty");
        }
        return Ok(content.to_string());
    }
    bail!("{env_key} or {file_key} environment variable is required")
}

/// Read the MFA code from `GARMIN_MFA_CODE` env, a file pointed to by
/// `GARMIN_MFA_CODE_FILE` (polled for non-interactive/agent runs), or stdin.
/// Blocking: poll/prompt for up to `MFA_WAIT`. Call it from `spawn_blocking`.
pub fn read_mfa_code() -> Result<String> {
    if let Some(val) = non_empty_env("GARMIN_MFA_CODE") {
        return Ok(val);
    }

    // File-based fallback with polling: lets an external agent inject the
    // MFA code into a running test by writing it to a file (stdin is not
    // reachable when the test is spawned in the background).
    if let Some(path) = non_empty_env("GARMIN_MFA_CODE_FILE") {
        eprintln!("[di_auth] waiting for MFA code in file: {path}");
        let deadline = std::time::Instant::now() + MFA_WAIT;
        while std::time::Instant::now() < deadline {
            if let Ok(content) = std::fs::read_to_string(&path) {
                let code = content.trim();
                if !code.is_empty() {
                    // Deleted only once the code has actually been accepted;
                    // see clear_mfa_code_file.
                    return Ok(code.to_string());
                }
            }
            std::thread::sleep(Duration::from_millis(500));
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

/// Remove the injected MFA code file once the code has been accepted.
///
/// Deleting it at read time throws away a still-valid code whenever the POST
/// fails, leaving no way to retry — and destroys whatever file the user pointed
/// the variable at, even a persistent one.
fn clear_mfa_code_file() {
    if let Some(path) = non_empty_env("GARMIN_MFA_CODE_FILE") {
        let _ = std::fs::remove_file(path);
    }
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
///
/// `client` is the process-wide impersonated client, passed in so that the SSO
/// cookies earned here (Cloudflare's `__cflb` in particular) are the same ones
/// the API layer later sends.
pub async fn authenticate(client: &rquest::Client) -> Result<DiSession> {
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
                        persist_session(&new_session);
                        return Ok(new_session);
                    }
                    Err(e) => {
                        eprintln!(
                            "[di_auth] refresh failed ({e:#}); falling through to fresh login"
                        );
                    }
                }
            } else {
                eprintln!("[di_auth] using cached DI session (valid)");
                return Ok(session);
            }
        }
    }

    // Layer 2: service ticket from env.
    if let Some(ticket) = non_empty_env("GARMIN_SERVICE_TICKET") {
        eprintln!("[di_auth] exchanging GARMIN_SERVICE_TICKET for DI session...");
        match exchange_service_ticket(&ticket).await {
            Ok(mut session) => {
                session.account = want_account.clone();
                persist_session(&session);
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

    // Layer 3: email + password (optionally MFA).
    let email = read_secret("GARMIN_EMAIL", "GARMIN_EMAIL_FILE")?;
    let password = read_secret("GARMIN_PASSWORD", "GARMIN_PASSWORD_FILE")?;

    eprintln!("[di_auth] authenticating with Garmin SSO ({SSO_HOST})...");
    let result = sso_login(client, &email, &password).await?;

    let ticket = if result.needs_mfa {
        eprintln!("[di_auth] MFA required.");
        // `read_mfa_code` blocks for up to five minutes (file polling) or
        // indefinitely (stdin). On the current-thread runtime the integration
        // test uses, doing that inline freezes the whole runtime.
        let code = tokio::task::spawn_blocking(read_mfa_code)
            .await
            .context("MFA code reader task panicked")??;
        let csrf = result
            .mfa_csrf
            .context("MFA required but no CSRF token was extracted from the MFA page")?;
        let mfa_url = result
            .mfa_url
            .context("MFA required but no MFA page URL was captured")?;
        let ticket = submit_mfa(client, &code, &csrf, &mfa_url).await?;
        clear_mfa_code_file();
        ticket
    } else {
        result
            .ticket
            .context("SSO login succeeded but no service ticket was returned")?
    };

    eprintln!("[di_auth] exchanging service ticket for DI session...");
    let mut session = exchange_service_ticket(&ticket).await?;
    session.account = Some(email.trim().to_lowercase());
    persist_session(&session);
    Ok(session)
}
