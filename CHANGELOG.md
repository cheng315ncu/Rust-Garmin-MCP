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
  refresh and persistence to `.di_session.json`.
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

### Fixed

- Authentication failure ("Invalid sign in. (Passwords are case sensitive.)")
  caused by Garmin's March 2026 Cloudflare TLS-fingerprinting rollout.
- MFA code trailing-newline bug from the old `garmin_client` handler (codes are
  now trimmed).

## [0.1.2] - 2026

- Fix Windows release asset upload and macOS x64 release scheduling.
- Remove presentation files from version control.

## [0.1.1] - 2026

- Fix Windows release asset upload.

## [0.1.0] - 2026

- Initial Garmin Connect MCP server (77 tools over stdio) with release
  automation.
