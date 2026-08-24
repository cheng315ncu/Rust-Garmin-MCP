use std::collections::HashMap;
use std::num::NonZeroU32;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::Result;
use garmin_client::GarminClient;
use governor::{
    clock::DefaultClock,
    state::{InMemoryState, NotKeyed},
    Quota, RateLimiter,
};
use moka::future::Cache;
use serde_json::Value;
use tokio::sync::{Mutex, RwLock};

const API_BASE: &str = "https://connectapi.garmin.com";

/// Max cached GET responses (LRU-evicted past this).
const CACHE_MAX_ENTRIES: u64 = 1_000;
/// TTL for cached GET responses.  60s coalesces "LLM re-asks the same
/// question in the same conversation" without hiding fresh wearable data
/// for long.  Tune via observation if researchers report stale reads.
const CACHE_TTL_SECS: u64 = 60;
/// Per-minute request budget against Garmin Connect.  Garmin does not
/// document a hard limit; 60 req/min is a conservative starting point.
/// Adjust by observation if tools begin failing with 429.
const RATE_LIMIT_PER_MIN: u32 = 60;

type Limiter = RateLimiter<NotKeyed, InMemoryState, DefaultClock>;

/// Holds a bearer token together with its expiry epoch (seconds).
struct BearerToken {
    value: String,
    expires_at: u64,
}

impl BearerToken {
    fn is_expired(&self) -> bool {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        now >= self.expires_at.saturating_sub(60) // 60-second safety margin
    }
}

/// Shared Garmin API session.
///
/// GET path layers (outermost → innermost):
///
/// ```text
/// moka cache  →  governor rate-limit  →  Mutex<GarminClient>  →  Garmin
/// ```
///
///   * Cache hit  → return Arc<Value> immediately; no Mutex, no network.
///   * Cache miss → moka's singleflight: only one task fetches, the rest
///     await the same future.  After the rate-limit wait, the Mutex
///     serialises the actual GarminClient call (the library is not
///     Send-safe and api_request takes &mut).
///   * Rate limit → applied to BOTH read and write paths so the per-minute
///     budget covers all Garmin-bound traffic.
///
/// Token (parallel to the above) lives behind Arc<RwLock<BearerToken>>:
/// concurrent reads for the write path, exclusive refresh.  garmin_client
/// only exposes GET, so POST/PUT/DELETE go through reqwest directly using
/// the cached bearer token.
pub struct GarminApiClient {
    inner: Arc<Mutex<GarminClient>>,
    http: reqwest::Client,
    token: Arc<RwLock<BearerToken>>,
    pub display_name: String,
    /// Key: `endpoint?k1=v1&k2=v2` (params sorted).  Value: Arc<Value> so
    /// cache reads are Arc-bumps instead of full JSON deep-clones.
    cache: Cache<String, Arc<Value>>,
    /// Shared across api_json and api_write — one budget for all traffic.
    limiter: Arc<Limiter>,
}

impl GarminApiClient {
    pub fn new(
        client: GarminClient,
        display_name: String,
        bearer_token: String,
        token_expires_at: u64,
    ) -> Self {
        let cache = Cache::builder()
            .max_capacity(CACHE_MAX_ENTRIES)
            .time_to_live(Duration::from_secs(CACHE_TTL_SECS))
            .build();

        let limiter = Arc::new(RateLimiter::direct(Quota::per_minute(
            NonZeroU32::new(RATE_LIMIT_PER_MIN).expect("RATE_LIMIT_PER_MIN > 0"),
        )));

        Self {
            inner: Arc::new(Mutex::new(client)),
            http: reqwest::Client::builder()
                .cookie_store(true) // mirror garmin_client behaviour
                .build()
                .expect("reqwest::Client build failed"),
            token: Arc::new(RwLock::new(BearerToken {
                value: bearer_token,
                expires_at: token_expires_at,
            })),
            display_name,
            cache,
            limiter,
        }
    }

    /// GET via garmin_client with TTL cache + singleflight + rate limit.
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

        // Clone Arc handles so the init future is 'static (moka requirement).
        let inner = self.inner.clone();
        let limiter = self.limiter.clone();
        let endpoint_owned = endpoint.trim_start_matches('/').to_string();
        let params_owned = params;

        let init = async move {
            // Rate limit gates the actual network call; cache hits skip this.
            limiter.until_ready().await;

            let mut guard = inner.lock().await;

            let ok = match params_owned.as_ref() {
                Some(p) => {
                    let p_ref: HashMap<&str, &str> =
                        p.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
                    guard
                        .api_request(&endpoint_owned, Some(p_ref), true, None)
                        .await
                }
                None => guard.api_request(&endpoint_owned, None, true, None).await,
            };

            let text = guard.get_last_resp_text().to_string();
            drop(guard); // release Mutex before parsing

            if !ok {
                anyhow::bail!("Garmin API GET to {} failed", endpoint_owned);
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

    /// POST / PUT / DELETE via reqwest (garmin_client is GET-only).
    ///
    /// On success, invalidates the entire GET cache so the next read sees
    /// post-write state.  Per-key invalidation would be ideal but
    /// write-endpoint → affected-read-endpoints is messy (e.g. POST /weight
    /// affects /weight-service/weight/dateRange) and writes are rare.
    async fn api_write(&self, method: &str, endpoint: &str, body: Option<Value>) -> Result<Value> {
        // Writes count against the same per-minute budget as reads.
        self.limiter.until_ready().await;

        self.refresh_token_if_needed().await;

        let token = self.token.read().await;
        let auth_header = format!("Bearer {}", token.value);
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
                &text[..text.len().min(200)]
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
                &text[..text.len().min(200)]
            )
        })
    }

    /// Refresh the cached bearer token when it is about to expire.
    ///
    /// Limitation (unchanged): garmin_client does NOT rewrite the session
    /// file after an in-process refresh — only on login.  Until upstream
    /// fixes that, this only helps after a server restart.  Long-lived
    /// daemons still die at the ~1-hour token lifetime.
    async fn refresh_token_if_needed(&self) {
        // Fast path: read lock — token is still valid.
        {
            let guard = self.token.read().await;
            if !guard.is_expired() {
                return;
            }
        }

        // Slow path: upgrade to write lock (only one task proceeds).
        let mut guard = self.token.write().await;

        // Re-check under write lock.
        if !guard.is_expired() {
            return;
        }

        // Trigger garmin_client's internal OAuth refresh via a cheap GET.
        {
            let mut inner = self.inner.lock().await;
            inner
                .api_request("userprofile-service/userprofile", None, true, None)
                .await;
        }

        if let Ok((new_token, new_exp)) = read_session_file() {
            eprintln!("[client] bearer token refreshed (expires_at={new_exp})");
            guard.value = new_token;
            guard.expires_at = new_exp;
        } else {
            eprintln!("[client] warning: token expired but could not re-read session file");
        }
    }

    pub fn require_display_name(&self) -> std::result::Result<&str, String> {
        if self.display_name.is_empty() {
            Err("Error: Garmin display name unknown. Set GARMIN_DISPLAY_NAME=<your Garmin handle> and restart.".to_string())
        } else {
            Ok(&self.display_name)
        }
    }
}

/// Build a stable cache key from `endpoint` + sorted params.
///
/// HashMap iteration order is unspecified, so two identical calls would
/// otherwise produce different keys and miss each other in the cache.
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

/// Read (token_value, expires_at) from .garmin_session.json.
/// Uses `.as_str()` — NOT `.to_string()` — to avoid garmin_client's
/// re-quoting bug where the JWT becomes `"eyJ..."` (with quotes).
pub fn read_session_file() -> Result<(String, u64)> {
    let text = std::fs::read_to_string(".garmin_session.json")?;
    let map: serde_json::Value = serde_json::from_str(&text)?;

    let token = map["token"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("token field missing in session file"))?
        .to_string();

    let expires_at = map["expires_at"]
        .as_str()
        .and_then(|s| s.parse::<u64>().ok())
        .or_else(|| map["expires_at"].as_u64())
        .unwrap_or(0);

    Ok((token, expires_at))
}

/// Detect Garmin error envelopes that parsed as JSON (403/404/etc with body).
/// Returns Some(short_message) when the value looks like an error envelope.
pub fn detect_garmin_error(value: &Value) -> Option<String> {
    let obj = value.as_object()?;

    // {status: 403, error: "Forbidden", message: "..."}
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

    // {exception: "ForbiddenException", ...}
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

    // {error: "NotAllowedException", clientMessage: "...", errorId: "..."}
    // Garmin uses this shape when the account doesn't have access to a feature.
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

    // {errorMessage: "..."}（no status / exception field）
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
