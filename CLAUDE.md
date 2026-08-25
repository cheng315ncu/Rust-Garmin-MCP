# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

A Model Context Protocol server (`garmin-mcp` binary) exposing 77 Garmin Connect tools over
**Streamable HTTP** (`axum` + `rmcp`'s `StreamableHttpService`, not stdio) — one long-lived
daemon, authenticated once, that every MCP client points at via URL. Listens on
`127.0.0.1:$GARMIN_MCP_HTTP_PORT` (default `8210`) at `/mcp`.
`src/lib.rs` is the library crate; `src/main.rs` is a thin binary; `tests/` consumes the library.

## Commands

```bash
cargo build --release          # binary at target/release/garmin-mcp
cargo check                    # fast type-check (~12s warm)
cargo clippy
cargo fmt

cargo test                     # unit tests (SSO parsers, session logic, cache key)

# The integration test hits the real network and is #[ignore]d, so `cargo test`
# compiles but skips it. Run it explicitly:
cargo test --test connection -- --ignored --nocapture
```

### Cold-build toolchain (BoringSSL)

`rquest` pulls in `boring-sys2`, which compiles BoringSSL from source and runs bindgen. A fresh
clone or post-`cargo clean` build needs `cmake`, `go`, and `libclang`. Incremental builds against a
warm `target/` do not need them, so `cargo check` succeeding is not evidence a cold build will.

On the original aarch64/WSL2 host there is no sudo/apt, so these live under `~/.local/opt/` and the
build needs:

```bash
export PATH="$HOME/.local/opt/go/bin:$HOME/.local/opt/cmake-3.31.8-linux-aarch64/bin:$PATH"
export LIBCLANG_PATH="$HOME/.local/opt/libclang/extracted/usr/lib/aarch64-linux-gnu"
export BINDGEN_EXTRA_CLANG_ARGS="-I/usr/lib/gcc/aarch64-linux-gnu/15/include"
```

Adjust or drop those on a machine where the toolchain is installed normally.

### Do not "fix" the rquest dependency

`rquest` is fully yanked from crates.io (all versions). It is pulled as a git dep on tag `v5.1.0`,
plus a `[patch.crates-io]` override so `rquest-util`'s transitive `rquest >=3` requirement resolves
to the same git source. Removing either half breaks resolution. (`rquest` v5.3.0+ was renamed
`wreq` — a different crate.)

## Architecture

```
main.rs → auth::create_garmin_server()
            ├─ build_impersonated_client()  → ONE rquest::Client for the process
            ├─ di_auth::authenticate(&http) → DiSession (OAuth2 tokens)
            ├─ resolve_display_name(&http)  → probes 3 userprofile endpoints
            └─ GarminApiClient::new(http, session, display_name)
                 └─ GarminMcpServer (tools/mod.rs, #[tool_router])
       → axum::serve — StreamableHttpService wrapping the server at /mcp,
         GARMIN_MCP_HTTP_PORT (default 8210). GarminMcpServer is Clone (cheap:
         shares the Arc<RwLock<DiSession>> + http client), so every HTTP
         session from every connected MCP client reuses this one login.
```

### Auth (`src/di_auth.rs`)

Garmin's garth-style SSO broke in March 2026 when Cloudflare TLS fingerprinting was enabled, so
`garmin_client` was replaced with a hand-rolled DI (Digital Identity) OAuth2 flow. Two distinct
HTTP clients, and the distinction matters:

- `build_impersonated_client()` — Chrome 131 / Android TLS emulation + cookie store. Required for
  anything behind Cloudflare: `sso.garmin.com` (HTML login) and `connectapi.garmin.com` (all API
  traffic). Built **once** in `create_garmin_server` and shared, so the SSO login, the display-name
  probe and every API call use the same cookie jar — Cloudflare's `__cflb` has to reach connectapi.
- `plain_di_client()` — a plain `rquest::Client` for `diauth.garmin.com` token exchange/refresh.
  That endpoint is standard OAuth2 and Chrome emulation causes handshake trouble there. Do not
  unify these.

Two rquest defaults bite here and are overridden deliberately: `timeout` is `None` (an unbounded
refresh once wedged every request in the process) and `redirect` is `Policy::none()` despite the
method's own doc comment, though the SSO POST 302s to the MFA page. Do not set
`pool_max_idle_per_host(0)` — that disables pooling entirely rather than capping it; the short
`pool_idle_timeout` is what guards against Garmin dropping idle sockets.

The `USER_AGENT` override must stay **after** `.emulation()`, which `mem::swap`s the whole header
map. **It deliberately does not match the emulated fingerprint** — a mobile Safari UA over a Chrome
131 / Android TLS fingerprint. Making them agree looks obviously correct and breaks the login:
tested live, a Chrome-131-Android UA gets the credential POST bounced back as the sign-in page with
"An unexpected error has occurred.", while the Safari UA reaches the MFA page. Don't tidy it.

Both call `system_cert_store()` to load the OS CA bundle, because BoringSSL's built-in webpki roots
won't contain a proxy/VPN's intercepting CA. It is parsed once behind a `OnceLock`.

`authenticate()` is a three-layer fallback: cached `.di_session.json` (refresh if stale) →
`GARMIN_SERVICE_TICKET` env → `GARMIN_EMAIL`/`GARMIN_PASSWORD` SSO login (→ MFA if the page title
contains "MFA"). The MFA CSRF token is carried out of `sso_login` in `SsoLoginResult` rather than
re-fetched — the session is already in MFA state and a fresh GET returns a different page. Always
`.trim()` MFA codes, and read them via `spawn_blocking` — `read_mfa_code` blocks for up to five
minutes and would otherwise freeze a current-thread runtime.

`DiSession.account` records which account the session was minted for, and layer 1 refuses a cached
session that doesn't match the configured `GARMIN_EMAIL`. Without it, editing the env and restarting
keeps serving the previous account's data for the refresh token's ~30 day life.

`refresh_token` is optional in a refresh response (RFC 6749 §6), so `parse_di_session` carries the
previous one forward rather than erroring — and only moves `refresh_expires_at` when the server
actually restates it.

The SSO flow parses HTML with regexes (`extract_title` / `extract_csrf` / `extract_ticket`); these
are the first things to break when Garmin changes markup. Keep them looser than they look like they
could be: a too-strict pattern doesn't truncate, it fails to match and aborts the whole login. The
`eprintln!("[di_auth] …")` traces through `sso_login` exist for exactly that diagnosis — keep them.
`src/di_auth.rs`'s unit tests pin the cases that previously broke.

### Request pipeline (`src/client.rs`)

```
GET   : moka cache (60s TTL, 1000 entries, singleflight) → governor (60 req/min) → rquest
POST/PUT/DELETE : governor → rquest → cache.invalidate_all()
```

Cache key is `endpoint?k=v` with params sorted; values are `Arc<Value>` so hits are Arc-bumps.
Cache hits skip the rate limiter entirely. The token lives in `Arc<RwLock<DiSession>>`;
`ensure_token_fresh` is a free function (not a method) because moka's init future must be `'static`
and cannot borrow `&self` — `refresh_token_if_needed` just delegates to it.

**Never hold the session lock across the refresh await.** A separate `Mutex<RefreshState>` is what
serialises refreshers; the `RwLock` is taken only for the non-await expiry checks and the final
store. `RefreshState` also carries exponential backoff, so a rejected refresh token doesn't re-run a
client build + CA parse + POST on every request.

Every connectapi request goes through `api_url()` and `garmin_headers()` — one place for the
`X-app-ver` bump.

**`GarminApiClient`'s public surface — `api_json` / `api_post_json` / `api_put_json` / `api_delete`
/ `require_display_name`, plus the free functions `render_or_friendly` and `detect_garmin_error` —
is the contract with `src/tools/*`. Do not change these signatures when refactoring the auth or HTTP
layer.** (`new` is not part of it; `auth.rs` is its only caller.)

A non-JSON or unparseable GET body is downgraded to `Value::Null` rather than an error, because
Garmin returns HTML error pages and empty 404s for endpoints an account doesn't have.

### Tool layer (`src/tools/`)

Every tool is split across two places:

1. `mod.rs` — a `schemars::JsonSchema` param struct, plus a `#[tool(description = "…")]` method
   inside the single `#[tool_router(server_handler)] impl GarminMcpServer` block. That macro
   generates the `ServerHandler` impl; there is no hand-written one.
2. A domain module (`activities.rs`, `health_wellness.rs`, …) holding the actual `pub async fn`
   that takes `&GarminApiClient` and returns `String`.

Adding a tool means editing both. Counts: health_wellness 21, activities 14, challenges 8, devices
6, workouts 5, research 4, user_profile 4, training/gear/nutrition/womens_health/data_management 3
each.

**Tools return `String`, never `Result`.** Errors, missing data, and account-gated features are all
rendered as human-readable text, so MCP `isError` is always `false`. Use
`client::render_or_friendly(&data, "No X data for {date} — <why>")`, which folds in
`detect_garmin_error()`'s parsing of Garmin's JSON error envelopes. Match the existing "no data"
message style: say *why* the data is absent (device not worn, feature not enabled). Note two
user-facing strings in `client.rs` are Traditional Chinese — match surrounding language when
editing those lines.

Tools needing `display_name` must start with `api.require_display_name()` and early-return its
`Err` string.

### Output formats (`src/tools/output.rs`)

Clinically-meaningful tools take an optional `format: "json" | "csv"` (`OutputFormat`, defaults to
JSON). Implement via the `ClinicalExport` trait — pick the existing payload shape (`FlatSummary`,
`HrvPayload`, `TimeseriesArray`, `EventTable`) instead of hand-rolling `to_string_pretty`. EDF is a
deliberately-unfilled slot in this trait; the module doc explains why.

`research.rs` is separate from the per-day tools: it fans out day-by-day over a date range
(`MAX_DAYS = 366`), emits date-only rows for days with no data so the series is never truncated,
and has its own CSV helpers.

## Environment

`GARMIN_EMAIL` / `GARMIN_PASSWORD` (each also accepts a `_FILE` variant pointing at a file
containing the secret), optional `GARMIN_DISPLAY_NAME` override, `GARMIN_SERVICE_TICKET`,
`GARMIN_MFA_CODE`, and `GARMIN_MFA_CODE_FILE` (polled for 5 minutes — lets an agent inject a code
into a backgrounded run where stdin is unreachable). `.env` is loaded via `dotenvy`; process env
wins. See `.env.example`.

Read every one of them through `non_empty_env()`: a variable set to the empty string counts as
unset, so a `FOO=` placeholder falls through to the `_FILE` or stdin fallback instead of submitting
an empty credential.

`.di_session.json` caches the OAuth2 session (0600, gitignored) in the CWD, which for a stdio server
launched by a desktop client can be anywhere — `GARMIN_SESSION_FILE` pins it.
