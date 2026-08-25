//! One-shot live dump of every read-only Garmin MCP tool's output, so a
//! schema reference can be written from real responses instead of guessed
//! shapes. Ignored by default since it hits the real network — run
//! explicitly with an output directory (outside the repo — the dumped
//! files contain real personal health data and must never be committed; an
//! in-repo path is rejected at startup, and each file is written 0600):
//!
//!   SCHEMA_DUMP_DIR=/path/to/dir cargo test --test schema_dump -- --ignored --nocapture
//!
//! Write/mutating tools (add_hydration_data, set_blood_pressure,
//! add_body_composition, add_gear_to_activity, remove_gear_from_activity,
//! schedule_workout, delete_workout) are deliberately NOT exercised here —
//! they would insert fake data into, or delete data from, a real account.

use std::fs;
use std::path::Path;

use chrono::{Days, Local};
use garmin_mcp::auth::create_garmin_client;
use garmin_mcp::tools::output::OutputFormat;
use garmin_mcp::tools::{
    activities, challenges, devices, gear, health_wellness, nutrition, research, training,
    user_profile, womens_health, workouts,
};
use serde_json::Value;

fn save(dir: &Path, domain: &str, tool: &str, content: &str) {
    let path = dir.join(format!("{domain}__{tool}.txt"));
    fs::write(&path, content).unwrap_or_else(|e| panic!("write {path:?}: {e}"));
    // GPS traces, HRV series, device serials and the account email — same
    // sensitivity as .di_session.json, so the same 0600, not the 0644 a default
    // umask would leave behind.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600))
            .unwrap_or_else(|e| panic!("chmod {path:?}: {e}"));
    }
    println!("[dump] {domain}::{tool} -> {} bytes", content.len());
}

/// Depth-bounded search for the first of `keys` present anywhere near the
/// top of `v`, preferring the first array element when one is found. Used
/// to pull a real id out of a raw (uncurated) Garmin response whose exact
/// wrapping shape isn't known ahead of time.
fn find_first_id(v: &Value, keys: &[&str]) -> Option<String> {
    fn walk(v: &Value, keys: &[&str], depth: u8) -> Option<String> {
        if depth > 4 {
            return None;
        }
        match v {
            Value::Object(map) => {
                for k in keys {
                    // Scalars only: the result is interpolated into a URL path,
                    // and a nested object under an `id` key would serialize to
                    // `{"a":1}` and produce a malformed request.
                    match map.get(*k) {
                        Some(Value::String(s)) => return Some(s.clone()),
                        Some(n @ Value::Number(_)) => return Some(n.to_string()),
                        _ => {}
                    }
                }
                map.values().find_map(|val| walk(val, keys, depth + 1))
            }
            // Every element, not just the first — one leading entry without the
            // key (a paired phone in the device list, say) would otherwise skip
            // a whole domain of the dump and look like "the account has none".
            Value::Array(arr) => arr.iter().find_map(|e| walk(e, keys, depth + 1)),
            _ => None,
        }
    }
    walk(v, keys, 0)
}

#[tokio::test]
#[ignore]
async fn dump_all_read_tool_schemas() {
    let _ = dotenvy::dotenv();

    let out_dir = std::path::PathBuf::from(
        std::env::var("SCHEMA_DUMP_DIR").expect("set SCHEMA_DUMP_DIR to an out-of-repo path"),
    );
    fs::create_dir_all(&out_dir).expect("create output dir");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&out_dir, fs::Permissions::from_mode(0o700)).expect("chmod output dir");
    }

    // cargo runs integration tests with the package root as cwd, so a relative
    // SCHEMA_DUMP_DIR lands in the working tree. .gitignore cannot cover an
    // arbitrary path, and these files hold real health data — refuse outright
    // rather than trusting the caller to have read the module doc.
    let repo = fs::canonicalize(env!("CARGO_MANIFEST_DIR")).expect("canonicalize repo root");
    let resolved = fs::canonicalize(&out_dir).expect("canonicalize SCHEMA_DUMP_DIR");
    assert!(
        !resolved.starts_with(&repo),
        "SCHEMA_DUMP_DIR ({}) is inside the repo ({}). The dump contains real GPS, heart-rate \
         and identity data — point it somewhere outside the working tree.",
        resolved.display(),
        repo.display(),
    );

    let api = create_garmin_client()
        .await
        .expect("Failed to create authenticated Garmin API client");

    // One `now`, derived with calendar arithmetic: five separate Local::now()
    // calls disagree across midnight, and `- Duration::days(n)` subtracts fixed
    // 24h spans that shift the local date across a DST boundary.
    let today_date = Local::now().date_naive();
    let day = |n: u64| (today_date - Days::new(n)).to_string();
    let today = today_date.to_string();
    let yesterday = day(1);
    let d10 = day(10);
    let d30 = day(30);
    let plus30 = (today_date + Days::new(30)).to_string();

    // ----- resolve dependency ids first -----

    let recent = activities::recent_activities(&api, 10).await;
    save(&out_dir, "activities", "recent_activities", &recent);
    let activity_id: Option<String> = serde_json::from_str::<Value>(&recent)
        .ok()
        .and_then(|v| find_first_id(&v, &["id"]));
    println!("[dump] resolved activity_id = {activity_id:?}");

    let dev_list = devices::get_devices(&api).await;
    save(&out_dir, "devices", "get_devices", &dev_list);
    let device_id: Option<String> = serde_json::from_str::<Value>(&dev_list)
        .ok()
        .and_then(|v| find_first_id(&v, &["deviceId", "unitId"]));
    println!("[dump] resolved device_id = {device_id:?}");

    let workout_list = workouts::get_workouts(&api, 0, 10).await;
    save(&out_dir, "workouts", "get_workouts", &workout_list);
    let workout_id: Option<String> = serde_json::from_str::<Value>(&workout_list)
        .ok()
        .and_then(|v| find_first_id(&v, &["workoutId"]));
    println!("[dump] resolved workout_id = {workout_id:?}");

    // ----- activities.rs -----

    save(
        &out_dir,
        "activities",
        "activities_by_date",
        &activities::activities_by_date(&api, &d10, &today, None).await,
    );
    if let Some(id) = &activity_id {
        save(
            &out_dir,
            "activities",
            "activity",
            &activities::activity(&api, id).await,
        );
        save(
            &out_dir,
            "activities",
            "activity_splits",
            &activities::activity_splits(&api, id).await,
        );
        save(
            &out_dir,
            "activities",
            "activity_typed_splits",
            &activities::activity_typed_splits(&api, id).await,
        );
        save(
            &out_dir,
            "activities",
            "activity_split_summaries",
            &activities::activity_split_summaries(&api, id).await,
        );
        save(
            &out_dir,
            "activities",
            "activity_weather",
            &activities::activity_weather(&api, id).await,
        );
        save(
            &out_dir,
            "activities",
            "activity_hr_in_timezones",
            &activities::activity_hr_in_timezones(&api, id).await,
        );
        save(
            &out_dir,
            "activities",
            "activity_exercise_sets",
            &activities::activity_exercise_sets(&api, id).await,
        );
        save(
            &out_dir,
            "activities",
            "activity_gear",
            &activities::activity_gear(&api, id).await,
        );
        save(
            &out_dir,
            "activities",
            "activity_training_effect",
            &activities::activity_training_effect(&api, id).await,
        );
    } else {
        println!("[dump] skipping activity_id-dependent activities::* tools — no activity found");
    }
    save(
        &out_dir,
        "activities",
        "activities_fordate",
        &activities::activities_fordate(&api, &yesterday).await,
    );
    save(
        &out_dir,
        "activities",
        "count_activities",
        &activities::count_activities(&api).await,
    );
    save(
        &out_dir,
        "activities",
        "activity_types",
        &activities::activity_types(&api).await,
    );

    // ----- challenges.rs -----

    save(
        &out_dir,
        "challenges",
        "earned_badges",
        &challenges::earned_badges(&api).await,
    );
    save(
        &out_dir,
        "challenges",
        "available_badge_challenges",
        &challenges::available_badge_challenges(&api).await,
    );
    save(
        &out_dir,
        "challenges",
        "badge_challenges",
        &challenges::badge_challenges(&api).await,
    );
    save(
        &out_dir,
        "challenges",
        "non_completed_badge_challenges",
        &challenges::non_completed_badge_challenges(&api).await,
    );
    save(
        &out_dir,
        "challenges",
        "adhoc_challenges",
        &challenges::adhoc_challenges(&api, 0, 10).await,
    );
    save(
        &out_dir,
        "challenges",
        "inprogress_virtual_challenges",
        &challenges::inprogress_virtual_challenges(&api).await,
    );
    save(
        &out_dir,
        "challenges",
        "goals",
        &challenges::goals(&api, None).await,
    );
    save(
        &out_dir,
        "challenges",
        "personal_records",
        &challenges::personal_records(&api).await,
    );

    // ----- devices.rs (get_devices already dumped above) -----

    save(
        &out_dir,
        "devices",
        "get_device_last_used",
        &devices::get_device_last_used(&api).await,
    );
    save(
        &out_dir,
        "devices",
        "get_primary_training_device",
        &devices::get_primary_training_device(&api).await,
    );
    if let Some(id) = &device_id {
        save(
            &out_dir,
            "devices",
            "get_device_settings",
            &devices::get_device_settings(&api, id).await,
        );
        save(
            &out_dir,
            "devices",
            "get_device_solar_data",
            &devices::get_device_solar_data(&api, id).await,
        );
        save(
            &out_dir,
            "devices",
            "get_device_alarms",
            &devices::get_device_alarms(&api, id).await,
        );
    } else {
        println!("[dump] skipping device_id-dependent devices::* tools — no device found");
    }

    // ----- gear.rs -----

    save(&out_dir, "gear", "get_gear", &gear::get_gear(&api).await);

    // ----- health_wellness.rs -----

    save(
        &out_dir,
        "health_wellness",
        "stats",
        &health_wellness::stats(&api, &yesterday, OutputFormat::Json).await,
    );
    save(
        &out_dir,
        "health_wellness",
        "sleep_summary",
        &health_wellness::sleep_summary(&api, &yesterday, OutputFormat::Json).await,
    );
    save(
        &out_dir,
        "health_wellness",
        "daily_heart_rate",
        &health_wellness::daily_heart_rate(&api, &yesterday, OutputFormat::Json).await,
    );
    save(
        &out_dir,
        "health_wellness",
        "stress_summary",
        &health_wellness::stress_summary(&api, &yesterday, OutputFormat::Json).await,
    );
    save(
        &out_dir,
        "health_wellness",
        "body_battery_summary",
        &health_wellness::body_battery_summary(&api, &yesterday, OutputFormat::Json).await,
    );
    save(
        &out_dir,
        "health_wellness",
        "daily_steps",
        &health_wellness::daily_steps(&api, &d10, &today).await,
    );
    save(
        &out_dir,
        "health_wellness",
        "training_readiness",
        &health_wellness::training_readiness(&api, &yesterday).await,
    );
    save(
        &out_dir,
        "health_wellness",
        "body_battery_events",
        &health_wellness::body_battery_events(&api, &yesterday).await,
    );
    save(
        &out_dir,
        "health_wellness",
        "blood_pressure",
        &health_wellness::blood_pressure(&api, &d10, &today, OutputFormat::Json).await,
    );
    save(
        &out_dir,
        "health_wellness",
        "floors",
        &health_wellness::floors(&api, &yesterday).await,
    );
    save(
        &out_dir,
        "health_wellness",
        "rhr_day",
        &health_wellness::rhr_day(&api, &yesterday).await,
    );
    save(
        &out_dir,
        "health_wellness",
        "hydration_data",
        &health_wellness::hydration_data(&api, &yesterday).await,
    );
    save(
        &out_dir,
        "health_wellness",
        "respiration_data",
        &health_wellness::respiration_data(&api, &yesterday, OutputFormat::Json).await,
    );
    save(
        &out_dir,
        "health_wellness",
        "spo2_data",
        &health_wellness::spo2_data(&api, &yesterday, OutputFormat::Json).await,
    );
    save(
        &out_dir,
        "health_wellness",
        "all_day_events",
        &health_wellness::all_day_events(&api, &yesterday).await,
    );
    save(
        &out_dir,
        "health_wellness",
        "hrv_data",
        &health_wellness::hrv_data(&api, &yesterday, OutputFormat::Json).await,
    );
    save(
        &out_dir,
        "health_wellness",
        "fitnessage_data",
        &health_wellness::fitnessage_data(&api, &yesterday).await,
    );
    save(
        &out_dir,
        "health_wellness",
        "endurance_score",
        &health_wellness::endurance_score(&api, &yesterday).await,
    );
    save(
        &out_dir,
        "health_wellness",
        "hill_score",
        &health_wellness::hill_score(&api, &yesterday).await,
    );
    save(
        &out_dir,
        "health_wellness",
        "lactate_threshold",
        &health_wellness::lactate_threshold(&api, &yesterday).await,
    );
    save(
        &out_dir,
        "health_wellness",
        "daily_weigh_ins",
        &health_wellness::daily_weigh_ins(&api, &d30, &today, OutputFormat::Json).await,
    );

    // ----- nutrition.rs -----

    save(
        &out_dir,
        "nutrition",
        "get_nutrition_daily_food_log",
        &nutrition::get_nutrition_daily_food_log(&api, &yesterday).await,
    );
    save(
        &out_dir,
        "nutrition",
        "get_nutrition_daily_settings",
        &nutrition::get_nutrition_daily_settings(&api).await,
    );
    save(
        &out_dir,
        "nutrition",
        "get_custom_foods",
        &nutrition::get_custom_foods(&api).await,
    );

    // ----- research.rs (short ranges to bound request count) -----

    save(
        &out_dir,
        "research",
        "daily_stats_range",
        &research::daily_stats_range(&api, &d10, &today, OutputFormat::Json).await,
    );
    save(
        &out_dir,
        "research",
        "sleep_range",
        &research::sleep_range(&api, &d10, &today, OutputFormat::Json).await,
    );
    save(
        &out_dir,
        "research",
        "hrv_range",
        &research::hrv_range(&api, &d10, &today, OutputFormat::Json).await,
    );
    save(
        &out_dir,
        "research",
        "weekly_summary",
        &research::weekly_summary(&api, &d10, &today).await,
    );

    // ----- training.rs -----

    save(
        &out_dir,
        "training",
        "training_status",
        &training::training_status(&api, &yesterday).await,
    );
    save(
        &out_dir,
        "training",
        "progress_summary",
        &training::progress_summary(&api, &d30, &today, None).await,
    );
    save(
        &out_dir,
        "training",
        "race_predictions",
        &training::race_predictions(&api).await,
    );

    // ----- user_profile.rs -----

    save(
        &out_dir,
        "user_profile",
        "get_user_profile",
        &user_profile::get_user_profile(&api).await,
    );
    save(
        &out_dir,
        "user_profile",
        "get_userprofile_settings",
        &user_profile::get_userprofile_settings(&api).await,
    );
    save(
        &out_dir,
        "user_profile",
        "get_full_name",
        &user_profile::get_full_name(&api).await,
    );
    save(
        &out_dir,
        "user_profile",
        "get_unit_system",
        &user_profile::get_unit_system(&api).await,
    );

    // ----- womens_health.rs -----

    save(
        &out_dir,
        "womens_health",
        "get_menstrual_data_for_date",
        &womens_health::get_menstrual_data_for_date(&api, &yesterday).await,
    );
    save(
        &out_dir,
        "womens_health",
        "get_menstrual_calendar_data",
        &womens_health::get_menstrual_calendar_data(&api, &d30, &today).await,
    );
    save(
        &out_dir,
        "womens_health",
        "get_pregnancy_summary",
        &womens_health::get_pregnancy_summary(&api).await,
    );

    // ----- workouts.rs (get_workouts already dumped above) -----

    if let Some(id) = &workout_id {
        save(
            &out_dir,
            "workouts",
            "get_workout_by_id",
            &workouts::get_workout_by_id(&api, id).await,
        );
    } else {
        println!("[dump] skipping get_workout_by_id — no workout found");
    }
    save(
        &out_dir,
        "workouts",
        "get_scheduled_workouts",
        &workouts::get_scheduled_workouts(&api, &today, &plus30).await,
    );

    println!("\n[dump] complete. Files written to {}", out_dir.display());
}
