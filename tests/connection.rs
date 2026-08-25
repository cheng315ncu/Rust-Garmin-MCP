//! Live connection test: performs a real DI OAuth2 login against Garmin
//! Connect using the same path as `main.rs`. Needs either a cached
//! `.di_session.json` with a valid refresh token, a `GARMIN_SERVICE_TICKET`
//! env var, or `GARMIN_EMAIL`/`GARMIN_PASSWORD` (plus `GARMIN_MFA_CODE` if
//! MFA is enabled) in the environment or a `.env` file in the project root.
//! Ignored by default since it hits the real network — run explicitly:
//!
//!   cargo test --test connection -- --ignored --nocapture

use garmin_mcp::auth::create_garmin_server;

#[tokio::test]
#[ignore]
async fn connects_and_authenticates_with_garmin() {
    let _ = dotenvy::dotenv();

    if let Err(e) = create_garmin_server().await {
        panic!(
            "Garmin connection test failed: {e:#}\n\
             Authentication options:\n\
             1. Cached DI session (.di_session.json with valid refresh token)\n\
             2. GARMIN_SERVICE_TICKET=<ticket> env var\n\
             3. GARMIN_EMAIL + GARMIN_PASSWORD (+ GARMIN_MFA_CODE if MFA enabled)"
        );
    }
}
