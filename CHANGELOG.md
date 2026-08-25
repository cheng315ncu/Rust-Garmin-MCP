# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

In March 2026 Garmin enabled Cloudflare TLS fingerprinting on `sso.garmin.com`
and `connectapi.garmin.com`, which broke the previous `garmin_client` /
`garth`-based OAuth1 mobile SSO flow (every login returned
"Invalid sign in. (Passwords are case sensitive.)"). This release replaces that
flow with a hand-rolled DI (Digital Identity) OAuth2 module backed by the `rquest`
TLS-impersonation crate.

### Added

- **DI OAuth2 authentication module** (`src/di_auth.rs`): exchanges a Garmin SSO
  service ticket for access/refresh tokens on `diauth.garmin.com`, with token
  refresh and persistence to `.di_session.json`. Garmin currently issues a ~24
  hour access token and a ~30 day refresh token.
- **TLS fingerprint impersonation** via `rquest` (Chrome 131 / Android emulation)
  with a cookie store, used for everything behind Cloudflare (`sso.garmin.com`
  HTML login and all `connectapi.garmin.com` API traffic).
- **System CA bundle loading** for both rquest clients, so HTTPS-intercepting
  proxies / VPNs (and WSL2) whose intercepting CA is not in BoringSSL's built-in
  webpki roots no longer fail with `CERTIFICATE_VERIFY_FAILED`.
- **MFA support** in the SSO flow: detects the MFA page by title, carries the
  CSRF token and page URL out of `sso_login`, and submits the code to the
  correct URL. MFA codes are always trimmed before submission.
- **File-based MFA polling** (`GARMIN_MFA_CODE_FILE`): a non-interactive run
  (e.g. a backgrounded test with no stdin) polls the file for 5 minutes so an
  external agent can inject the time-sensitive code.
- **Three-layer authentication fallback** in `authenticate()`:
  1. cached `.di_session.json` (refreshed if stale),
  2. `GARMIN_SERVICE_TICKET` env var,
  3. `GARMIN_EMAIL` / `GARMIN_PASSWORD` SSO login (→ MFA if required).
- **Library crate extraction** (`src/lib.rs`) so `tests/` can consume the
  library without depending on the binary.
- **Connection integration test** (`tests/connection.rs`, `#[ignore]`d by
  default) that exercises the live auth + API path.
- **Unit tests** for the SSO HTML parsers, `parse_di_session`, the cached-session
  account gate, the cache key, and the refresh backoff. `cargo test` previously
  ran nothing, since the only test in the repo was the ignored live one.
- **Account binding** on the session cache: `DiSession` records the account it
  was minted for, and a cached session is not reused when `GARMIN_EMAIL`
  changes.
- **`GARMIN_SESSION_FILE`** to pin the session cache path, for stdio servers
  launched with an arbitrary working directory.
- **`.env.example`**, which `.gitignore` had a `!.env.example` exception for but
  which never existed.

### Changed

- Replaced the `garmin_client` (garth-based OAuth1 / mobile SSO) dependency with
  the new DI OAuth2 flow.
- HTTP layer switched from `reqwest` to `rquest` for TLS fingerprint
  impersonation; the DI token endpoint (`diauth.garmin.com`) deliberately uses a
  plain `rquest::Client` (no Chrome emulation) because it is a standard OAuth2
  endpoint and emulation causes handshake trouble there.
- `GarminApiClient` now holds an `Arc<RwLock<DiSession>>` with double-checked
  token refresh and session persistence; the public surface
  (`api_json` / `api_post_json` / `api_put_json` / `api_delete` /
  `require_display_name`) is unchanged so `src/tools/*` is unaffected.
- `.di_session.json` replaces the old `.garmin_session.json` session cache
  (gitignored).

### Removed

- `garmin_client` and `reqwest` dependencies.
- The "Known `garmin_client 0.2.1` bugs" README section, along with the rest of
  the pre-refactor documentation it belonged to.

### Fixed

- Authentication failure ("Invalid sign in. (Passwords are case sensitive.)")
  caused by Garmin's March 2026 Cloudflare TLS-fingerprinting rollout.
- MFA code trailing-newline bug from the old `garmin_client` handler (codes are
  now trimmed).
- **The account gate could fail open.** An unreadable `GARMIN_EMAIL_FILE` — a
  secrets mount that is not ready yet, say — collapsed to "no account
  configured", which is the value that means "nothing can contradict the cached
  session". It is an error now.
- **MFA required with no code available no longer reads stdin blindly.** The
  server speaks MCP over stdio, so prompting there consumes protocol traffic;
  it prompts only on a real terminal and otherwise says which variable to set.
- **A rejected MFA code is cleared from `GARMIN_MFA_CODE_FILE`.** Codes are
  single-use, so keeping the file after a failed submission made every
  subsequent run read the same dead code back.
- **A refreshed token that is already expired no longer clears the backoff**,
  which would otherwise turn the next request into another refresh — a hot loop
  against Garmin's auth endpoint.
- **`sso_login` failures now name the reason.** The page title is the same on a
  rejected sign-in as on a successful one, so a failure reported only
  `page title: "GARMIN Authentication Application"`. The visible error banner is
  extracted instead, which distinguishes "Invalid sign in. (Passwords are case
  sensitive.)" (wrong credentials) from "An unexpected error has occurred."
  (Garmin rejected the request itself).
- **A stalled token refresh could hang the whole server.** The DI clients set no
  request timeout (rquest defaults to none) and the refresh was awaited while
  holding the session write lock, so one half-open connection to
  `diauth.garmin.com` blocked every request permanently and silently. There is
  now a 30-second timeout, and refresh is serialised by a dedicated mutex so the
  session lock is never held across an await.
- **A rejected refresh token was retried on every single request**, each time
  rebuilding a client and re-parsing the CA bundle. Failures now back off
  exponentially (30s to 15min) and report once.
- **`refresh_token` is optional on a refresh** (RFC 6749 §6); treating it as
  mandatory would have broken every refresh had Garmin disabled rotation. The
  previous token and its real expiry are carried forward instead — an absent
  `refresh_expires_in` no longer silently re-extends the believed window by 30
  days.
- **A cached session outlived its credentials.** Changing `GARMIN_EMAIL` kept
  serving the previous account's health data for the ~30 day refresh lifetime,
  because layer 1 never consulted the environment.
- **Connection pooling was disabled**, not merely capped: `pool_max_idle_per_host(0)`
  turns rquest's pool off, so every request paid a fresh TCP + BoringSSL +
  Chrome-emulation handshake — 366 of them for a year-long research range. A
  30-second `pool_idle_timeout` covers the stale-socket case it was guarding.
- **Three impersonated clients meant three cookie jars**, so the Cloudflare
  `__cflb` cookie earned during SSO never reached `connectapi`. One client is now
  shared across login, the display-name probe, and all API traffic — which also
  removes a `.expect()` panic that fired after a completed MFA login, and a
  silent fallback to a bare `rquest::Client` with no CA store, no emulation and
  no redirect policy.
- **The `_csrf` and ticket regexes were stricter than Garmin's markup.** `\w+`
  rejects base64- and UUID-shaped tokens outright (the match fails, it does not
  truncate), and requiring `value` directly after `name` breaks on attribute
  reordering or single quotes. The ticket pattern likewise now stops at `'`,
  whitespace, `<` and `\`.
- **An environment variable set to the empty string counted as provided**,
  skipping the `_FILE` and stdin fallbacks and submitting an empty credential.
- **`read_mfa_code` blocked the async runtime** for up to five minutes, freezing
  the current-thread runtime the integration test uses; it now runs on
  `spawn_blocking`. The injected code file is deleted only after Garmin accepts
  the code, instead of at read time.
- **`.di_session.json` was created world-readable** (0644 via `fs::write`) while
  holding a ~30 day refresh token; it is now 0600. A failed write no longer
  aborts authentication outright, discarding a session that cost a full MFA
  round trip.
- **Error messages dropped their cause.** `{}` on an `anyhow::Error` prints only
  the outermost frame, deleting the `.with_context` chain; `{:#}` is used
  throughout now, and the write path gained the context it was missing.
- **`sso_login` emitted no diagnostics**, despite the module documentation
  pointing at its traces as the way to diagnose a Garmin markup change.

## [0.1.2] - 2026

- Fix Windows release asset upload and macOS x64 release scheduling.
- Remove presentation files from version control.

## [0.1.1] - 2026

- Fix Windows release asset upload.

## [0.1.0] - 2026

- Initial Garmin Connect MCP server (77 tools over stdio) with release
  automation.
