use std::collections::HashMap;
use std::num::NonZeroU32;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use governor::{
    clock::DefaultClock,
    state::{InMemoryState, NotKeyed},
    Quota, RateLimiter,
};
use moka::future::Cache;
use serde_json::Value;
use tokio::sync::RwLock;

use crate::di_auth::{self, DiSession};

pub const API_BASE: &str = "https://connectapi.garmin.com";

/// Max cached GET responses (LRU-evicted past this).
const CACHE_MAX_ENTRIES: u64 = 1_000;
/// TTL for cached GET responses. 60s coalesces "LLM re-asks the same
/// question in the same conversation" without hiding fresh wearable data.
const CACHE_TTL_SECS: u64 = 60;
/// Per-minute request budget against Garmin Connect. 60 req/min is a
/// conservative starting point. Adjust if tools begin failing with 429.
const RATE_LIMIT_PER_MIN: u32 = 60;

type Limiter = RateLimiter<NotKeyed, InMemoryState, DefaultClock>;

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
}

impl GarminApiClient {
    pub fn new(session: DiSession, display_name: String) -> Self {
        let cache = Cache::builder()
            .max_capacity(CACHE_MAX_ENTRIES)
            .time_to_live(Duration::from_secs(CACHE_TTL_SECS))
            .build();

        let limiter = Arc::new(RateLimiter::direct(Quota::per_minute(
            NonZeroU32::new(RATE_LIMIT_PER_MIN).expect("RATE_LIMIT_PER_MIN > 0"),
        )));

        Self {
            http: di_auth::build_impersonated_client()
                .expect("rquest impersonated client build failed"),
            token: Arc::new(RwLock::new(session)),
            display_name,
            cache,
            limiter,
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
        let endpoint_owned = endpoint.trim_start_matches('/').to_string();
        let params_owned = params;

        let init = async move {
            // Rate limit gates the actual network call; cache hits skip this.
            limiter.until_ready().await;

            // Refresh the DI access token if it is about to expire.
            ensure_token_fresh(token.clone()).await;

            let guard = token.read().await;
            let auth_header = format!("Bearer {}", guard.access_token);
            drop(guard); // release read lock before the network call

            let url = format!("{}/{}", API_BASE, endpoint_owned);
            let mut req = http
                .get(&url)
                .header("Authorization", auth_header)
                .header("NK", "NT")
                .header("X-app-ver", "4.70.2.0")
                .header("Accept", "application/json");

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
            Err(arc_err) => Err(anyhow::anyhow!("{}", arc_err)),
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
        let auth_header = format!("Bearer {}", token.access_token);
        drop(token); // release read lock before the network call

        let endpoint = endpoint.trim_start_matches('/');
        let url = format!("{}/{}", API_BASE, endpoint);

        let mut req = match method {
            "POST" => self.http.post(&url),
            "PUT" => self.http.put(&url),
            "DELETE" => self.http.delete(&url),
            _ => anyhow::bail!("unsupported HTTP method: {}", method),
        };

        req = req
            .header("Authorization", auth_header)
            .header("NK", "NT")
            .header("X-app-ver", "4.70.2.0")
            .header("Accept", "application/json");

        if let Some(b) = body {
            req = req.json(&b);
        }

        let resp = req.send().await?;
        let status = resp.status();
        let text = resp.text().await?;

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
        ensure_token_fresh(self.token.clone()).await;
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
/// moka init future (which cannot borrow `&self`).  Double-checked locking:
/// fast path takes a read lock, slow path upgrades to a write lock and
/// re-checks so only one task performs the refresh.
async fn ensure_token_fresh(token: Arc<RwLock<DiSession>>) {
    // Fast path: read lock — token is still valid.
    {
        let guard = token.read().await;
        if !guard.is_expired() {
            return;
        }
    }

    // Slow path: upgrade to write lock (only one task proceeds).
    let mut guard = token.write().await;

    // Re-check under write lock.
    if !guard.is_expired() {
        return;
    }

    if !guard.refresh_is_valid() {
        eprintln!(
            "[client] warning: access token expired and refresh token no longer valid; re-login needed"
        );
        return;
    }

    match di_auth::refresh_di_token(&guard).await {
        Ok(new_session) => {
            eprintln!(
                "[client] DI access token refreshed (expires_at={})",
                new_session.expires_at
            );
            let _ = di_auth::save_session(&new_session);
            *guard = new_session;
        }
        Err(e) => {
            eprintln!("[client] warning: DI token refresh failed: {e}");
        }
    }
}

fn preview(text: &str) -> String {
    text.chars().take(200).collect()
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
            let qs: Vec<String> = entries.iter().map(|(k, v)| format!("{}={}", k, v)).collect();
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
