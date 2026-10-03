//! Offline guard: a failed Garmin fetch must fail the whole range call, never
//! become a date-only "no data" row. rust-health-db upserts those rows over
//! stored data, so a swallowed error silently wipes history.
//!
//! Every request is sent through a proxy on a closed port, so no network is used.

use garmin_mcp::client::GarminApiClient;
use garmin_mcp::di_auth::DiSession;
use garmin_mcp::tools::output::OutputFormat;
use garmin_mcp::tools::{activities, research};

#[tokio::test]
async fn failed_fetch_returns_error_not_empty_rows() {
    let http = rquest::Client::builder()
        .proxy(rquest::Proxy::all("http://127.0.0.1:9").unwrap())
        .build()
        .unwrap();
    let session = DiSession {
        access_token: "test".into(),
        refresh_token: "test".into(),
        expires_at: u64::MAX, // never triggers a token refresh
        refresh_expires_at: u64::MAX,
        client_id: "test".into(),
        account: None,
    };
    let api = GarminApiClient::new(http, session, "test".into());
    let (s, e) = ("2026-01-01", "2026-01-02");

    let outputs = [
        research::daily_stats_range(&api, s, e, OutputFormat::Csv).await,
        research::sleep_range(&api, s, e, OutputFormat::Csv).await,
        research::hrv_range(&api, s, e, OutputFormat::Csv).await,
        research::weekly_summary(&api, s, e).await,
        activities::activities_by_date(&api, s, e, None).await,
    ];
    for out in outputs {
        assert!(out.starts_with("Error"), "expected an error, got: {out}");
    }
}
