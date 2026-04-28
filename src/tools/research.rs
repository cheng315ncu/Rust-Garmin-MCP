use std::collections::HashMap;

use chrono::{Datelike, Duration, NaiveDate};
use serde_json::{Map, Number, Value};

use crate::client::GarminApiClient;
use crate::tools::output::OutputFormat;

const MAX_DAYS: i64 = 366;

// ----- date helpers -----

fn date_range_vec(start: &str, end: &str) -> Result<Vec<NaiveDate>, String> {
    let s = NaiveDate::parse_from_str(start, "%Y-%m-%d")
        .map_err(|_| format!("Invalid start_date '{start}' — expected YYYY-MM-DD"))?;
    let e = NaiveDate::parse_from_str(end, "%Y-%m-%d")
        .map_err(|_| format!("Invalid end_date '{end}' — expected YYYY-MM-DD"))?;
    if e < s {
        return Err(format!("end_date {end} must be ≥ start_date {start}"));
    }
    let days = (e - s).num_days() + 1;
    if days > MAX_DAYS {
        return Err(format!(
            "Range spans {days} days (max {MAX_DAYS}). Use a shorter range or split into multiple calls."
        ));
    }
    Ok((0..days).map(|i| s + Duration::days(i)).collect())
}

// ----- CSV / JSON rendering -----

fn rows_to_csv(rows: &[Map<String, Value>], columns: &[&str]) -> String {
    let mut out = columns.join(",");
    out.push('\n');
    for row in rows {
        let cells: Vec<String> = columns
            .iter()
            .map(|col| row.get(*col).map(value_to_csv_cell).unwrap_or_default())
            .collect();
        out.push_str(&cells.join(","));
        out.push('\n');
    }
    out
}

fn rows_to_json(rows: Vec<Map<String, Value>>) -> String {
    serde_json::to_string_pretty(&Value::Array(rows.into_iter().map(Value::Object).collect()))
        .unwrap_or_else(|e| format!("Error: {e}"))
}

fn value_to_csv_cell(v: &Value) -> String {
    match v {
        Value::Null => String::new(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        Value::String(s) => csv_quote(s),
        other => csv_quote(&other.to_string()),
    }
}

fn csv_quote(s: &str) -> String {
    if s.contains(',') || s.contains('"') || s.contains('\n') {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

fn round2(x: f64) -> f64 {
    (x * 100.0).round() / 100.0
}

fn to_json_num(x: f64) -> Value {
    Number::from_f64(round2(x))
        .map(Value::Number)
        .unwrap_or(Value::Null)
}

// ----- API → column mappings -----

const STATS_API_FIELDS: &[(&str, &str)] = &[
    ("calendarDate", "date"),
    ("totalSteps", "total_steps"),
    ("totalKilocalories", "total_calories"),
    ("activeKilocalories", "active_calories"),
    ("totalDistanceMeters", "distance_meters"),
    ("highlyActiveSeconds", "highly_active_seconds"),
    ("activeSeconds", "active_seconds"),
    ("sedentarySeconds", "sedentary_seconds"),
    ("moderateIntensityMinutes", "moderate_intensity_minutes"),
    ("vigorousIntensityMinutes", "vigorous_intensity_minutes"),
    ("restingHeartRate", "resting_heart_rate_bpm"),
    ("averageStressLevel", "avg_stress_level"),
    ("maxStressLevel", "max_stress_level"),
    ("bodyBatteryChargedValue", "body_battery_charged"),
    ("bodyBatteryDrainedValue", "body_battery_drained"),
    ("bodyBatteryHighestValue", "body_battery_highest"),
    ("bodyBatteryLowestValue", "body_battery_lowest"),
    ("averageSpo2", "avg_spo2_percent"),
    ("lowestSpo2", "lowest_spo2_percent"),
    ("avgWakingRespirationValue", "avg_waking_respiration"),
];

const STATS_COLS: &[&str] = &[
    "date",
    "total_steps",
    "total_calories",
    "active_calories",
    "distance_meters",
    "highly_active_seconds",
    "active_seconds",
    "sedentary_seconds",
    "moderate_intensity_minutes",
    "vigorous_intensity_minutes",
    "resting_heart_rate_bpm",
    "avg_stress_level",
    "max_stress_level",
    "body_battery_charged",
    "body_battery_drained",
    "body_battery_highest",
    "body_battery_lowest",
    "avg_spo2_percent",
    "lowest_spo2_percent",
    "avg_waking_respiration",
];

const SLEEP_API_FIELDS: &[(&str, &str)] = &[
    ("calendarDate", "date"),
    ("sleepTimeSeconds", "total_sleep_seconds"),
    ("deepSleepSeconds", "deep_sleep_seconds"),
    ("lightSleepSeconds", "light_sleep_seconds"),
    ("remSleepSeconds", "rem_sleep_seconds"),
    ("awakeSleepSeconds", "awake_seconds"),
    ("sleepStartTimestampLocal", "sleep_start_local"),
    ("sleepEndTimestampLocal", "sleep_end_local"),
    ("averageSpo2Value", "avg_spo2_percent"),
    ("lowestSpo2Value", "lowest_spo2_percent"),
    ("averageRespirationValue", "avg_respiration"),
    ("lowestRespirationValue", "lowest_respiration"),
    ("highestRespirationValue", "highest_respiration"),
    ("awakeCount", "awake_count"),
    ("restlessMomentsCount", "restless_moments_count"),
    ("avgSleepStress", "avg_sleep_stress"),
];

const SLEEP_COLS: &[&str] = &[
    "date",
    "total_sleep_seconds",
    "deep_sleep_seconds",
    "light_sleep_seconds",
    "rem_sleep_seconds",
    "awake_seconds",
    "sleep_start_local",
    "sleep_end_local",
    "avg_spo2_percent",
    "lowest_spo2_percent",
    "avg_respiration",
    "lowest_respiration",
    "highest_respiration",
    "awake_count",
    "restless_moments_count",
    "avg_sleep_stress",
];

const HRV_API_FIELDS: &[(&str, &str)] = &[
    ("calendarDate", "date"),
    ("weeklyAvg", "weekly_avg"),
    ("lastNight", "last_night"),
    ("lastNight5MinHigh", "last_night_5min_high"),
    ("lastNight5MinLow", "last_night_5min_low"),
    ("baselineBalancedLow", "baseline_balanced_low"),
    ("baselineBalancedUpper", "baseline_balanced_upper"),
    ("status", "status"),
    ("feedback", "feedback"),
];

const HRV_COLS: &[&str] = &[
    "date",
    "weekly_avg",
    "last_night",
    "last_night_5min_high",
    "last_night_5min_low",
    "baseline_balanced_low",
    "baseline_balanced_upper",
    "status",
    "feedback",
];

// Numeric columns used in weekly aggregation
const WEEKLY_NUMERIC_COLS: &[&str] = &[
    "total_steps",
    "total_calories",
    "active_calories",
    "distance_meters",
    "moderate_intensity_minutes",
    "vigorous_intensity_minutes",
    "resting_heart_rate_bpm",
    "avg_stress_level",
    "body_battery_highest",
    "body_battery_lowest",
    "avg_spo2_percent",
    "avg_waking_respiration",
];

// ----- Tool functions -----

/// Fetch daily health stats for each day in a date range.
/// Returns a JSON array or CSV with 20 metrics per row.
pub async fn daily_stats_range(
    api: &GarminApiClient,
    start: &str,
    end: &str,
    fmt: OutputFormat,
) -> String {
    let dates = match date_range_vec(start, end) {
        Ok(d) => d,
        Err(e) => return e,
    };
    let display_name = match api.require_display_name() {
        Ok(n) => n,
        Err(e) => return e,
    };

    let mut rows: Vec<Map<String, Value>> = Vec::with_capacity(dates.len());

    for date in &dates {
        let date_str = date.format("%Y-%m-%d").to_string();
        let mut row = Map::new();
        row.insert("date".to_string(), Value::String(date_str.clone()));

        let endpoint = format!("/usersummary-service/usersummary/daily/{}", display_name);
        let params = HashMap::from([("calendarDate".to_string(), date_str)]);

        if let Ok(data) = api.api_json(&endpoint, Some(params)).await {
            if !data.is_null() && crate::client::detect_garmin_error(&data).is_none() {
                for &(src, dst) in STATS_API_FIELDS {
                    if let Some(v) = data.get(src) {
                        if !v.is_null() {
                            row.insert(dst.to_string(), v.clone());
                        }
                    }
                }
            }
        }

        rows.push(row);
    }

    match fmt {
        OutputFormat::Csv => rows_to_csv(&rows, STATS_COLS),
        OutputFormat::Json => rows_to_json(rows),
    }
}

/// Fetch sleep summary for each day in a date range.
/// Returns a JSON array or CSV with 16 sleep metrics per row.
pub async fn sleep_range(
    api: &GarminApiClient,
    start: &str,
    end: &str,
    fmt: OutputFormat,
) -> String {
    let dates = match date_range_vec(start, end) {
        Ok(d) => d,
        Err(e) => return e,
    };
    let display_name = match api.require_display_name() {
        Ok(n) => n,
        Err(e) => return e,
    };

    let mut rows: Vec<Map<String, Value>> = Vec::with_capacity(dates.len());

    for date in &dates {
        let date_str = date.format("%Y-%m-%d").to_string();
        let mut row = Map::new();
        row.insert("date".to_string(), Value::String(date_str.clone()));

        let endpoint = format!("/wellness-service/wellness/dailySleepData/{}", display_name);
        let params = HashMap::from([
            ("date".to_string(), date_str),
            ("nonSleepBufferMinutes".to_string(), "60".to_string()),
        ]);

        if let Ok(data) = api.api_json(&endpoint, Some(params)).await {
            let dto = data.get("dailySleepDTO").unwrap_or(&data);
            if !dto.is_null() {
                for &(src, dst) in SLEEP_API_FIELDS {
                    if let Some(v) = dto.get(src) {
                        if !v.is_null() {
                            row.insert(dst.to_string(), v.clone());
                        }
                    }
                }
            }
        }

        rows.push(row);
    }

    match fmt {
        OutputFormat::Csv => rows_to_csv(&rows, SLEEP_COLS),
        OutputFormat::Json => rows_to_json(rows),
    }
}

/// Fetch HRV summary for each day in a date range.
/// Days without HRV data appear as date-only rows.
/// Returns a JSON array or CSV with 9 HRV columns per row.
pub async fn hrv_range(
    api: &GarminApiClient,
    start: &str,
    end: &str,
    fmt: OutputFormat,
) -> String {
    let dates = match date_range_vec(start, end) {
        Ok(d) => d,
        Err(e) => return e,
    };

    let mut rows: Vec<Map<String, Value>> = Vec::with_capacity(dates.len());

    for date in &dates {
        let date_str = date.format("%Y-%m-%d").to_string();
        let mut row = Map::new();
        row.insert("date".to_string(), Value::String(date_str.clone()));

        let endpoint = format!("/hrv-service/hrv/{}", date_str);

        if let Ok(data) = api.api_json(&endpoint, None).await {
            if let Some(Value::Object(summary)) = data.get("hrvSummary") {
                for &(src, dst) in HRV_API_FIELDS {
                    if let Some(v) = summary.get(src) {
                        if !v.is_null() {
                            row.insert(dst.to_string(), v.clone());
                        }
                    }
                }
            }
        }

        rows.push(row);
    }

    match fmt {
        OutputFormat::Csv => rows_to_csv(&rows, HRV_COLS),
        OutputFormat::Json => rows_to_json(rows),
    }
}

/// Compute per-ISO-week statistics (mean, std, min, max) for 12 key health
/// metrics over a date range.  Returns a JSON array, one object per week.
pub async fn weekly_summary(api: &GarminApiClient, start: &str, end: &str) -> String {
    let dates = match date_range_vec(start, end) {
        Ok(d) => d,
        Err(e) => return e,
    };
    let display_name = match api.require_display_name() {
        Ok(n) => n,
        Err(e) => return e,
    };

    // Fetch one daily-summary row per date
    let mut daily: Vec<(NaiveDate, Map<String, Value>)> = Vec::with_capacity(dates.len());

    for date in &dates {
        let date_str = date.format("%Y-%m-%d").to_string();
        let mut row = Map::new();
        row.insert("date".to_string(), Value::String(date_str.clone()));

        let endpoint = format!("/usersummary-service/usersummary/daily/{}", display_name);
        let params = HashMap::from([("calendarDate".to_string(), date_str)]);

        if let Ok(data) = api.api_json(&endpoint, Some(params)).await {
            if !data.is_null() && crate::client::detect_garmin_error(&data).is_none() {
                for &(src, dst) in STATS_API_FIELDS {
                    if let Some(v) = data.get(src) {
                        if !v.is_null() {
                            row.insert(dst.to_string(), v.clone());
                        }
                    }
                }
            }
        }

        daily.push((*date, row));
    }

    // Group by ISO week (sequential — dates are in order so this is stable)
    let mut weeks: Vec<(String, Vec<Map<String, Value>>)> = Vec::new();
    for (date, row) in daily {
        let iw = date.iso_week();
        let label = format!("{}-W{:02}", iw.year(), iw.week());
        match weeks.last_mut() {
            Some(last) if last.0 == label => last.1.push(row),
            _ => weeks.push((label, vec![row])),
        }
    }

    // Build one output object per week
    let mut result: Vec<Map<String, Value>> = Vec::with_capacity(weeks.len());

    for (label, rows) in weeks {
        let mut w = Map::new();
        w.insert("week".to_string(), Value::String(label));
        w.insert(
            "days_with_data".to_string(),
            Value::Number(rows.len().into()),
        );

        if let Some(s) = rows.first().and_then(|r| r.get("date")).and_then(|v| v.as_str()) {
            w.insert("week_start".to_string(), Value::String(s.to_string()));
        }
        if let Some(e) = rows.last().and_then(|r| r.get("date")).and_then(|v| v.as_str()) {
            w.insert("week_end".to_string(), Value::String(e.to_string()));
        }

        for &col in WEEKLY_NUMERIC_COLS {
            let vals: Vec<f64> = rows
                .iter()
                .filter_map(|r| r.get(col).and_then(|v| v.as_f64()))
                .collect();
            if vals.is_empty() {
                continue;
            }
            let n = vals.len() as f64;
            let mean = vals.iter().sum::<f64>() / n;
            let var = vals.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / n;
            let std = var.sqrt();
            let min = vals.iter().cloned().fold(f64::INFINITY, f64::min);
            let max = vals.iter().cloned().fold(f64::NEG_INFINITY, f64::max);

            w.insert(format!("{col}_mean"), to_json_num(mean));
            w.insert(format!("{col}_std"), to_json_num(std));
            w.insert(format!("{col}_min"), to_json_num(min));
            w.insert(format!("{col}_max"), to_json_num(max));
        }

        result.push(w);
    }

    rows_to_json(result)
}
