//! Live connection test: performs a real DI OAuth2 login against Garmin
//! Connect using the same path as `main.rs`. Needs either a cached
//! `.di_session.json` with a valid refresh token, a `GARMIN_SERVICE_TICKET`
//! env var, or `GARMIN_EMAIL`/`GARMIN_PASSWORD` (plus `GARMIN_MFA_CODE` if
//! MFA is enabled) in the environment or a `.env` file in the project root.
//! Ignored by default since it hits the real network — run explicitly:
//!
//!   cargo test --test connection -- --ignored --nocapture

use chrono::{Duration, Local};
use garmin_mcp::auth::{create_garmin_client, create_garmin_server};
use garmin_mcp::tools::output::OutputFormat;
use garmin_mcp::tools::{activities, devices, health_wellness, user_profile};

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

#[tokio::test]
#[ignore]
async fn test_query_garmin_data() {
    let _ = dotenvy::dotenv();

    let api = create_garmin_client()
        .await
        .expect("Failed to create authenticated Garmin API client");

    let today = Local::now().format("%Y-%m-%d").to_string();
    let yesterday = (Local::now() - Duration::days(1))
        .format("%Y-%m-%d")
        .to_string();
    let seven_days_ago = (Local::now() - Duration::days(7))
        .format("%Y-%m-%d")
        .to_string();

    println!("\n=================== Garmin Query Test ===================");
    println!("Testing with Date: {today} (yesterday: {yesterday})");

    // 1. User Profile & Account
    println!("\n--- [1] User Profile ---");
    let profile = user_profile::get_user_profile(&api).await;
    println!("User Profile Result:\n{profile}");

    let full_name = user_profile::get_full_name(&api).await;
    println!("Full Name: {full_name}");

    // 2. Devices
    println!("\n--- [2] Devices ---");
    let dev_list = devices::get_devices(&api).await;
    println!("Devices:\n{dev_list}");

    // 3. Health & Wellness Data
    println!("\n--- [3] Daily Stats & Steps ---");
    let stats = health_wellness::stats(&api, &yesterday, OutputFormat::Json).await;
    println!("Daily Stats ({yesterday}):\n{stats}");

    let steps = health_wellness::daily_steps(&api, &seven_days_ago, &today).await;
    println!("Daily Steps ({seven_days_ago} ~ {today}):\n{steps}");

    let sleep = health_wellness::sleep_summary(&api, &yesterday, OutputFormat::Json).await;
    println!("Sleep Summary ({yesterday}):\n{sleep}");

    let hr = health_wellness::daily_heart_rate(&api, &yesterday, OutputFormat::Json).await;
    println!("Daily Heart Rate ({yesterday}):\n{hr}");

    // 4. Activities
    println!("\n--- [4] Recent Activities ---");
    let recent = activities::recent_activities(&api, 5).await;
    println!("Recent Activities:\n{recent}");

    let activity_count = activities::count_activities(&api).await;
    println!("Activity Count:\n{activity_count}");

    println!("\n=========================================================\n");
}
