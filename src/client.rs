use std::collections::HashMap;
use std::num::NonZeroU32;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use governor::{
    clock::DefaultClock,
    state::{InMemoryState, NotKeyed},
    Quota, RateLimiter,
};
use moka::future::Cache;
use serde_json::Value;
use tokio::sync::{Mutex, RwLock};

use crate::di_auth::{self, preview, DiSession};

const API_BASE: &str = "https://connectapi.garmin.com";
/// Garmin's mobile app version, sent on every connectapi request.
const APP_VER: &str = "4.70.2.0";

/// Absolute URL for a connectapi endpoint, with or without a leading slash
/// (`connectapi.garmin.com//path` 404s).
pub(crate) fn api_url(endpoint: &str) -> String {
    format!("{}/{}", API_BASE, endpoint.trim_start_matches('/'))
}

/// The header set every authenticated connectapi request needs. Defined once so
/// that bumping `X-app-ver` is a one-line change rather than a grep.
pub(crate) fn garmin_headers(
    req: rquest::RequestBuilder,
    access_token: &str,
) -> rquest::RequestBuilder {
    req.header("Authorization", format!("Bearer {access_token}"))
        .header("NK", "NT")
        .header("X-app-ver", APP_VER)
        .header("Accept", "application/json")
}

/// Max cached GET responses (LRU-evicted past this).
const CACHE_MAX_ENTRIES: u64 = 1_000;
/// TTL for cached GET responses. 60s coalesces "LLM re-asks the same
/// question in the same conversation" without hiding fresh wearable data.
const CACHE_TTL_SECS: u64 = 60;
/// Per-minute request budget against Garmin Connect. 60 req/min is a
/// conservative starting point. Adjust if tools begin failing with 429.
const RATE_LIMIT_PER_MIN: u32 = 60;
/// First backoff step after a failed DI token refresh.
const REFRESH_BACKOFF_MIN: Duration = Duration::from_secs(30);
/// Ceiling for the exponential backoff between failed refresh attempts, so a
/// long-lived server keeps checking occasionally instead of giving up.
const REFRESH_BACKOFF_MAX: Duration = Duration::from_secs(900);

type Limiter = RateLimiter<NotKeyed, InMemoryState, DefaultClock>;

/// Coordination state for DI access-token refresh.
///
/// Deliberately a separate lock from `RwLock<DiSession>`: it serialises the
/// refresh (so concurrent callers make one network call, not N) *without*
/// anyone holding the session lock across an await.  Holding the session write
/// guard over the refresh means one stalled `diauth.garmin.com` connection
/// blocks every request in the process, with no log line and no recovery.
#[derive(Default)]
struct RefreshState {
    /// Earliest instant at which another refresh may be attempted.
    next_attempt: Option<Instant>,
    /// Consecutive failures; drives the exponential backoff.
    failures: u32,
    /// Set once the refresh token itself is dead, so that warning is printed
    /// once rather than on every single request.
    reported_dead: bool,
}

/// Shared Garmin API session.
///
/// GET path layers (outermost -> innermost):
///
/// ```text
/// moka cache  ->  governor rate-limit  ->  rquest GET  ->  Garmin
/// ```
///
///   * Cache hit  -> return Arc<Value> immediately; no network.
///   * Cache miss -> moka's singleflight: only one task fetches, the rest
///     await the same future.  After the rate-limit wait, the rquest GET
///     carries the DI OAuth2 bearer token.
///
/// Token lives behind `Arc<RwLock<DiSession>>`: concurrent reads for the
/// request path, exclusive refresh when the access token nears expiry.
/// The DI refresh token (~30 day lifetime) auto-renews the access token and
/// the new session is persisted to `.di_session.json`.
pub struct GarminApiClient {
    http: rquest::Client,
    token: Arc<RwLock<DiSession>>,
    pub display_name: String,
    /// Key: `endpoint?k1=v1&k2=v2` (params sorted).  Value: Arc<Value> so
    /// cache reads are Arc-bumps instead of full JSON deep-clones.
    cache: Cache<String, Arc<Value>>,
    /// Shared across api_json and api_write — one budget for all traffic.
    limiter: Arc<Limiter>,
    /// Serialises token refresh and remembers failures; see `RefreshState`.
    refresh: Arc<Mutex<RefreshState>>,
}

impl GarminApiClient {
    /// `http` is the process-wide impersonated client built in `auth.rs`, so
    /// the API layer reuses the cookie jar and connection pool that the SSO
    /// login warmed up.
    pub fn new(http: rquest::Client, session: DiSession, display_name: String) -> Self {
        let cache = Cache::builder()
            .max_capacity(CACHE_MAX_ENTRIES)
            .time_to_live(Duration::from_secs(CACHE_TTL_SECS))
            .build();

        let limiter = Arc::new(RateLimiter::direct(Quota::per_minute(
            NonZeroU32::new(RATE_LIMIT_PER_MIN).expect("RATE_LIMIT_PER_MIN > 0"),
        )));

        Self {
            http,
            token: Arc::new(RwLock::new(session)),
            display_name,
            cache,
            limiter,
            refresh: Arc::new(Mutex::new(RefreshState::default())),
        }
    }

    /// GET via rquest with TTL cache + singleflight + rate limit.
    ///
    /// Repeated calls within `CACHE_TTL_SECS` for the same (endpoint, params)
    /// return the cached Value without hitting Garmin.  Concurrent callers
    /// for the same key share one in-flight fetch.
    pub async fn api_json(
        &self,
        endpoint: &str,
        params: Option<HashMap<String, String>>,
    ) -> Result<Value> {
        let key = build_cache_key(endpoint, params.as_ref());

        // Clone handles so the init future is 'static (moka requirement).
        let http = self.http.clone();
        let token = self.token.clone();
        let limiter = self.limiter.clone();
        let refresh = self.refresh.clone();
        let endpoint_owned = endpoint.trim_start_matches('/').to_string();
        let params_owned = params;

        let init = async move {
            // Rate limit gates the actual network call; cache hits skip this.
            limiter.until_ready().await;

            // Refresh the DI access token if it is about to expire.
            ensure_token_fresh(token.clone(), refresh).await;

            let guard = token.read().await;
            let access_token = guard.access_token.clone();
            drop(guard); // release read lock before the network call

            let url = api_url(&endpoint_owned);
            let mut req = garmin_headers(http.get(&url), &access_token);

            if let Some(p) = params_owned.as_ref() {
                req = req.query(&p);
            }

            let resp = req
                .send()
                .await
                .with_context(|| format!("Garmin API GET {url} failed"))?;
            let status = resp.status();
            let text = resp
                .text()
                .await
                .with_context(|| format!("Garmin API GET {url} body read failed"))?;

            if !status.is_success() {
                anyhow::bail!(
                    "Garmin API GET to {} failed: {} {}",
                    endpoint_owned,
                    status,
                    preview(&text)
                );
            }

            if text.is_empty() {
                return Ok(Arc::new(Value::Null));
            }

            // Garmin sometimes returns 404 with empty body, HTML error pages, or
            // truncated JSON for endpoints that don't exist for a given account.
            // Treat parse failures as "no data" rather than hard errors so the
            // tool layer can render a friendly message.
            let value: Value = match serde_json::from_str::<Value>(&text) {
                Ok(v) => v,
                Err(e) => {
                    let preview: String = text.chars().take(120).collect();
                    eprintln!(
                        "[garmin] {endpoint_owned}: 回應無法解析（{} 字元），視為無資料；preview: {preview}; err: {e}",
                        text.len()
                    );
                    Value::Null
                }
            };

            Ok::<_, anyhow::Error>(Arc::new(value))
        };

        match self.cache.try_get_with(key, init).await {
            Ok(arc) => Ok((*arc).clone()),
            // moka wraps init errors in Arc<E> so concurrent waiters share one.
            // `{:#}` so the `.with_context` chain above survives; plain `{}`
            // prints only the outermost frame and deletes the real cause.
            Err(arc_err) => Err(anyhow::anyhow!("{arc_err:#}")),
        }
    }

    pub async fn api_post_json(&self, endpoint: &str, body: Value) -> Result<Value> {
        self.api_write("POST", endpoint, Some(body)).await
    }

    #[allow(dead_code)]
    pub async fn api_put_json(&self, endpoint: &str, body: Value) -> Result<Value> {
        self.api_write("PUT", endpoint, Some(body)).await
    }

    pub async fn api_delete(&self, endpoint: &str) -> Result<Value> {
        self.api_write("DELETE", endpoint, None).await
    }

    /// POST / PUT / DELETE via rquest.
    ///
    /// On success, invalidates the entire GET cache so the next read sees
    /// post-write state.  Per-key invalidation would be ideal but
    /// write-endpoint -> affected-read-endpoints is messy and writes are rare.
    async fn api_write(&self, method: &str, endpoint: &str, body: Option<Value>) -> Result<Value> {
        // Writes count against the same per-minute budget as reads.
        self.limiter.until_ready().await;

        self.refresh_token_if_needed().await;

        let token = self.token.read().await;
        let access_token = token.access_token.clone();
        drop(token); // release read lock before the network call

        let endpoint = endpoint.trim_start_matches('/');
        let url = api_url(endpoint);

        let req = match method {
            "POST" => self.http.post(&url),
            "PUT" => self.http.put(&url),
            "DELETE" => self.http.delete(&url),
            _ => anyhow::bail!("unsupported HTTP method: {}", method),
        };

        let mut req = garmin_headers(req, &access_token);

        if let Some(b) = body {
            req = req.json(&b);
        }

        let resp = req
            .send()
            .await
            .with_context(|| format!("Garmin API {method} {url} failed"))?;
        let status = resp.status();
        let text = resp
            .text()
            .await
            .with_context(|| format!("Garmin API {method} {url} body read failed"))?;

        if !status.is_success() {
            anyhow::bail!(
                "{} {} returned {}: {}",
                method,
                endpoint,
                status,
                preview(&text)
            );
        }

        // Drop cached GETs so post-write state is visible.
        self.cache.invalidate_all();

        if text.is_empty() {
            return Ok(Value::Null);
        }

        serde_json::from_str(&text).map_err(|e| {
            anyhow::anyhow!(
                "parse error at {} {}: {e}; body: {}",
                method,
                endpoint,
                preview(&text)
            )
        })
    }

    /// Refresh the DI access token when it is about to expire, using the
    /// refresh token, and persist the new session.
    async fn refresh_token_if_needed(&self) {
        ensure_token_fresh(self.token.clone(), self.refresh.clone()).await;
    }

    pub fn require_display_name(&self) -> std::result::Result<&str, String> {
        if self.display_name.is_empty() {
            Err("Error: Garmin display name unknown. Set GARMIN_DISPLAY_NAME=<your Garmin handle> and restart.".to_string())
        } else {
            Ok(&self.display_name)
        }
    }
}

/// Free-function twin of `refresh_token_if_needed` usable from the 'static
/// moka init future (which cannot borrow `&self`).
///
/// The session `RwLock` is never held across the refresh network call.  The
/// `refresh` mutex is what serialises concurrent callers, so exactly one
/// refresh goes out; the session lock is taken only for the (non-await)
/// expiry checks and the final store.
async fn ensure_token_fresh(token: Arc<RwLock<DiSession>>, refresh: Arc<Mutex<RefreshState>>) {
    // Fast path: token is still valid, no coordination needed.
    if !token.read().await.is_expired() {
        return;
    }

    // Slow path: one refresher at a time; everyone else waits here and then
    // re-checks, because the holder may have just refreshed the token.
    let mut state = refresh.lock().await;

    let session = {
        let guard = token.read().await;
        if !guard.is_expired() {
            return;
        }
        if !guard.refresh_is_valid() {
            // No amount of retrying fixes a dead refresh token, so say it once.
            if !state.reported_dead {
                state.reported_dead = true;
                eprintln!(
                    "[client] warning: access token expired and the refresh token is no longer valid; restart to re-login"
                );
            }
            return;
        }
        guard.clone()
    };

    // Back off after a failure instead of re-running the whole refresh (new
    // client, CA bundle parse, POST) on every single request.
    if state.next_attempt.is_some_and(|next| Instant::now() < next) {
        return;
    }

    // Note the absence of any lock guard across this await.
    match di_auth::refresh_di_token(&session).await {
        Ok(new_session) => {
            eprintln!(
                "[client] DI access token refreshed (expires_at={})",
                new_session.expires_at
            );
            if let Err(e) = di_auth::save_session(&new_session) {
                eprintln!("[client] warning: could not persist refreshed session: {e:#}");
            }
            *token.write().await = new_session;
            state.next_attempt = None;
            state.failures = 0;
            state.reported_dead = false;
        }
        Err(e) => {
            state.failures = state.failures.saturating_add(1);
            let backoff = refresh_backoff(state.failures);
            state.next_attempt = Some(Instant::now() + backoff);
            eprintln!(
                "[client] warning: DI token refresh failed ({e:#}); next attempt in {}s",
                backoff.as_secs()
            );
        }
    }
}

/// Exponential backoff between failed refresh attempts: doubles per
/// consecutive failure, capped at `REFRESH_BACKOFF_MAX`.
fn refresh_backoff(failures: u32) -> Duration {
    let steps = failures.saturating_sub(1).min(5);
    REFRESH_BACKOFF_MIN
        .saturating_mul(1 << steps)
        .min(REFRESH_BACKOFF_MAX)
}

/// Build a stable cache key from `endpoint` + sorted params.
fn build_cache_key(endpoint: &str, params: Option<&HashMap<String, String>>) -> String {
    let ep = endpoint.trim_start_matches('/');
    match params {
        None => ep.to_string(),
        Some(p) if p.is_empty() => ep.to_string(),
        Some(p) => {
            let mut entries: Vec<(&str, &str)> =
                p.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
            entries.sort_by_key(|(k, _)| *k);
            let qs: Vec<String> = entries
                .iter()
                .map(|(k, v)| format!("{}={}", k, v))
                .collect();
            format!("{}?{}", ep, qs.join("&"))
        }
    }
}

/// Detect Garmin error envelopes that parsed as JSON (403/404/etc with body).
pub fn detect_garmin_error(value: &Value) -> Option<String> {
    let obj = value.as_object()?;

    if let Some(status) = obj.get("status").and_then(Value::as_u64) {
        if (400..600).contains(&status) {
            let kind = obj
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("HTTP error");
            let msg = obj.get("message").and_then(Value::as_str).unwrap_or("");
            return Some(if msg.is_empty() {
                format!("HTTP {status} {kind}")
            } else {
                format!("HTTP {status} {kind}: {msg}")
            });
        }
    }

    if let Some(exc) = obj.get("exception").and_then(Value::as_str) {
        let msg = obj
            .get("errorMessage")
            .or_else(|| obj.get("message"))
            .and_then(Value::as_str)
            .unwrap_or("");
        return Some(if msg.is_empty() {
            exc.to_string()
        } else {
            format!("{exc}: {msg}")
        });
    }

    if let Some(err) = obj.get("error").and_then(Value::as_str) {
        if err.ends_with("Exception") || obj.contains_key("errorId") {
            let hint = obj
                .get("clientMessage")
                .or_else(|| obj.get("message"))
                .and_then(Value::as_str)
                .unwrap_or("");
            return Some(if hint.is_empty() {
                err.to_string()
            } else {
                format!("{err}: {hint}")
            });
        }
    }

    if let Some(msg) = obj.get("errorMessage").and_then(Value::as_str) {
        return Some(msg.to_string());
    }

    None
}

/// Render a Value as pretty JSON, or return a friendly fallback message
/// when the value is Null (no data) or a Garmin error envelope.
pub fn render_or_friendly(data: &Value, no_data_msg: &str) -> String {
    if data.is_null() {
        return no_data_msg.to_string();
    }
    if let Some(err) = detect_garmin_error(data) {
        return format!("{no_data_msg}（API 訊息：{err}）");
    }
    serde_json::to_string_pretty(data).unwrap_or_else(|e| format!("Error: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_key_sorts_params_so_call_order_does_not_split_the_cache() {
        let mut a = HashMap::new();
        a.insert("b".to_string(), "2".to_string());
        a.insert("a".to_string(), "1".to_string());

        assert_eq!(build_cache_key("/x/y", Some(&a)), "x/y?a=1&b=2");
        assert_eq!(build_cache_key("/x/y", None), "x/y");
        assert_eq!(build_cache_key("x/y", Some(&HashMap::new())), "x/y");
    }

    #[test]
    fn api_url_tolerates_a_leading_slash() {
        // `connectapi.garmin.com//path` 404s.
        assert_eq!(api_url("/a/b"), format!("{API_BASE}/a/b"));
        assert_eq!(api_url("a/b"), format!("{API_BASE}/a/b"));
    }

    #[test]
    fn refresh_backoff_grows_then_caps() {
        assert_eq!(refresh_backoff(1), REFRESH_BACKOFF_MIN);
        assert_eq!(refresh_backoff(2), REFRESH_BACKOFF_MIN * 2);
        assert_eq!(refresh_backoff(3), REFRESH_BACKOFF_MIN * 4);
        assert_eq!(refresh_backoff(99), REFRESH_BACKOFF_MAX);
        // Never zero — a zero backoff would restore the hot-loop this fixes.
        assert!(refresh_backoff(0) >= REFRESH_BACKOFF_MIN);
    }

    #[test]
    fn garmin_error_envelopes_are_detected() {
        let http = serde_json::json!({"status": 404, "error": "Not Found", "message": "no data"});
        assert_eq!(
            detect_garmin_error(&http).as_deref(),
            Some("HTTP 404 Not Found: no data")
        );

        let exc = serde_json::json!({"exception": "NotAllowedException", "errorMessage": "gated"});
        assert_eq!(
            detect_garmin_error(&exc).as_deref(),
            Some("NotAllowedException: gated")
        );

        // A normal payload must not be mistaken for an error envelope.
        assert_eq!(
            detect_garmin_error(&serde_json::json!({"steps": 1200})),
            None
        );
        assert_eq!(
            detect_garmin_error(&serde_json::json!({"status": 200})),
            None
        );
    }

    #[test]
    fn render_or_friendly_explains_absence_instead_of_printing_null() {
        assert_eq!(
            render_or_friendly(&Value::Null, "no data for 2026-01-01"),
            "no data for 2026-01-01"
        );
        assert!(render_or_friendly(&serde_json::json!({"steps": 1}), "x").contains("\"steps\""));
    }
}
