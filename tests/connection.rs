//! Live connection test: performs a real OAuth login against Garmin Connect
//! using the same path as `main.rs`. Needs GARMIN_EMAIL/GARMIN_PASSWORD (or
//! _FILE variants) in the environment or a `.env` file in the project root.
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
            "Garmin connection test failed: {e}\n\
             Check that GARMIN_EMAIL/GARMIN_PASSWORD (or _FILE) are set and correct."
        );
    }
}
