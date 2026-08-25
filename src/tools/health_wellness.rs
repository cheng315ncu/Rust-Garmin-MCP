use std::collections::HashMap;

use serde_json::{Map, Value};

use crate::client::GarminApiClient;
use crate::tools::output::{
    ClinicalExport, EventTable, FlatSummary, HrvPayload, OutputFormat, TimeseriesArray,
};

pub async fn stats(api: &GarminApiClient, date: &str, fmt: OutputFormat) -> String {
    let name = match api.require_display_name() {
        Ok(n) => n,
        Err(e) => return e,
    };
    let endpoint = format!("/usersummary-service/usersummary/daily/{}", name);
    let params = HashMap::from([("calendarDate".to_string(), date.to_string())]);

    match api.api_json(&endpoint, Some(params)).await {
        Ok(data) => FlatSummary {
            fields: pluck_into_map(&data, STATS_FIELDS),
        }
        .render(fmt),
        Err(e) => format!("Error retrieving stats for {date}: {e}"),
    }
}

pub async fn sleep_summary(api: &GarminApiClient, date: &str, fmt: OutputFormat) -> String {
    let name = match api.require_display_name() {
        Ok(n) => n,
        Err(e) => return e,
    };
    let endpoint = format!("/wellness-service/wellness/dailySleepData/{}", name);
    let params = HashMap::from([
        ("date".to_string(), date.to_string()),
        ("nonSleepBufferMinutes".to_string(), "60".to_string()),
    ]);

    match api.api_json(&endpoint, Some(params)).await {
        Ok(data) => {
            let dto = data.get("dailySleepDTO").unwrap_or(&data);
            let fields = pluck_into_map(dto, SLEEP_FIELDS);
            if fields.len() <= 1 {
                return format!(
                    "No sleep data for {date} — watch may not have been worn that night."
                );
            }
            FlatSummary { fields }.render(fmt)
        }
        Err(e) => format!("Error retrieving sleep summary for {date}: {e}"),
    }
}

pub async fn daily_heart_rate(api: &GarminApiClient, date: &str, fmt: OutputFormat) -> String {
    let name = match api.require_display_name() {
        Ok(n) => n,
        Err(e) => return e,
    };
    let endpoint = format!("/wellness-service/wellness/dailyHeartRate/{}", name);
    let params = HashMap::from([("date".to_string(), date.to_string())]);

    match api.api_json(&endpoint, Some(params)).await {
        Ok(data) => {
            let mut out = pluck_into_map(&data, HEART_RATE_FIELDS);
            if let Some(Value::Array(arr)) = data.get("heartRateValues") {
                out.insert(
                    "heart_rate_sample_count".to_string(),
                    Value::Number(arr.len().into()),
                );
            }
            FlatSummary { fields: out }.render(fmt)
        }
        Err(e) => format!("Error retrieving heart rate for {date}: {e}"),
    }
}

pub async fn stress_summary(api: &GarminApiClient, date: &str, fmt: OutputFormat) -> String {
    let endpoint = format!("/wellness-service/wellness/dailyStress/{}", date);
    match api.api_json(&endpoint, None).await {
        Ok(data) => {
            let mut out = pluck_into_map(&data, STRESS_FIELDS);
            if let Some(Value::Array(arr)) = data.get("stressValuesArray") {
                out.insert(
                    "stress_sample_count".to_string(),
                    Value::Number(arr.len().into()),
                );
            }
            if let Some(Value::Array(arr)) = data.get("bodyBatteryValuesArray") {
                out.insert(
                    "body_battery_sample_count".to_string(),
                    Value::Number(arr.len().into()),
                );
            }
            FlatSummary { fields: out }.render(fmt)
        }
        Err(e) => format!("Error retrieving stress summary for {date}: {e}"),
    }
}

pub async fn body_battery_summary(api: &GarminApiClient, date: &str, fmt: OutputFormat) -> String {
    let endpoint = "/wellness-service/wellness/bodyBattery/reports/daily";
    let params = HashMap::from([
        ("startDate".to_string(), date.to_string()),
        ("endDate".to_string(), date.to_string()),
    ]);

    match api.api_json(endpoint, Some(params)).await {
        Ok(data) => {
            let entry = match &data {
                Value::Array(arr) => arr.first().cloned().unwrap_or(Value::Null),
                other => other.clone(),
            };
            if entry.is_null() {
                return format!("No body battery data for {date}");
            }

            let mut summary = pluck_into_map(&entry, BODY_BATTERY_FIELDS);
            let samples = parse_ts_value_pairs(entry.get("bodyBatteryValuesArray"));
            if let (Some((_, first)), Some((_, last))) = (samples.first(), samples.last()) {
                summary.insert("starting_value".to_string(), first.clone());
                summary.insert("ending_value".to_string(), last.clone());
            }
            TimeseriesArray {
                date: date.to_string(),
                value_column: "body_battery",
                samples,
                summary,
            }
            .render(fmt)
        }
        Err(e) => format!("Error retrieving body battery for {date}: {e}"),
    }
}

pub async fn daily_steps(api: &GarminApiClient, start_date: &str, end_date: &str) -> String {
    let endpoint = format!(
        "/usersummary-service/stats/steps/daily/{}/{}",
        start_date, end_date
    );
    match api.api_json(&endpoint, None).await {
        Ok(data) => serde_json::to_string_pretty(&data).unwrap_or_else(|e| format!("Error: {e}")),
        Err(e) => format!("Error retrieving daily steps from {start_date} to {end_date}: {e}"),
    }
}

pub async fn training_readiness(api: &GarminApiClient, date: &str) -> String {
    let endpoint = format!("/training-readiness-service/training-readiness/{}", date);
    match api.api_json(&endpoint, None).await {
        Ok(data) => crate::client::render_or_friendly(
            &data,
            &format!("No training readiness data for {date} — feature requires a compatible Garmin device with recent activity."),
        ),
        Err(e) => format!("Error retrieving training readiness for {date}: {e}"),
    }
}

pub async fn body_battery_events(api: &GarminApiClient, date: &str) -> String {
    let params = HashMap::from([
        ("startDate".to_string(), date.to_string()),
        ("endDate".to_string(), date.to_string()),
    ]);
    match api
        .api_json(
            "/wellness-service/wellness/bodyBattery/events",
            Some(params),
        )
        .await
    {
        Ok(data) => {
            crate::client::render_or_friendly(&data, &format!("No body battery events for {date}."))
        }
        Err(e) => format!("Error retrieving body battery events for {date}: {e}"),
    }
}

pub async fn blood_pressure(
    api: &GarminApiClient,
    start_date: &str,
    end_date: &str,
    fmt: OutputFormat,
) -> String {
    let endpoint = format!(
        "/biometric-service/blood-pressure/dates/{}/{}",
        start_date, end_date
    );
    match api.api_json(&endpoint, None).await {
        Ok(data) => {
            let rows = extract_event_rows(
                &data,
                &["measurementSummaries", "bloodPressureValues", "items"],
            );
            EventTable {
                columns: vec![
                    "measurementTimestampLocal",
                    "measurementTimestampGMT",
                    "systolic",
                    "diastolic",
                    "pulse",
                    "notes",
                ],
                rows,
            }
            .render(fmt)
        }
        Err(e) => format!("Error retrieving blood pressure from {start_date} to {end_date}: {e}"),
    }
}

pub async fn floors(api: &GarminApiClient, date: &str) -> String {
    let endpoint = format!("/wellness-service/wellness/floorsChartData/daily/{}", date);
    match api.api_json(&endpoint, None).await {
        Ok(data) => serde_json::to_string_pretty(&data).unwrap_or_else(|e| format!("Error: {e}")),
        Err(e) => format!("Error retrieving floors for {date}: {e}"),
    }
}

pub async fn rhr_day(api: &GarminApiClient, date: &str) -> String {
    let name = match api.require_display_name() {
        Ok(n) => n,
        Err(e) => return e,
    };
    let endpoint = format!("/userstats-service/wellness/daily/{}", name);
    let params = HashMap::from([
        ("fromDate".to_string(), date.to_string()),
        ("untilDate".to_string(), date.to_string()),
        ("metricId".to_string(), "60".to_string()),
    ]);
    match api.api_json(&endpoint, Some(params)).await {
        Ok(data) => serde_json::to_string_pretty(&data).unwrap_or_else(|e| format!("Error: {e}")),
        Err(e) => format!("Error retrieving RHR for {date}: {e}"),
    }
}

pub async fn hydration_data(api: &GarminApiClient, date: &str) -> String {
    let endpoint = format!("/usersummary-service/usersummary/hydration/daily/{}", date);
    match api.api_json(&endpoint, None).await {
        Ok(data) => serde_json::to_string_pretty(&data).unwrap_or_else(|e| format!("Error: {e}")),
        Err(e) => format!("Error retrieving hydration data for {date}: {e}"),
    }
}

pub async fn respiration_data(api: &GarminApiClient, date: &str, fmt: OutputFormat) -> String {
    let endpoint = format!("/wellness-service/wellness/daily/respiration/{}", date);
    match api.api_json(&endpoint, None).await {
        Ok(data) => FlatSummary {
            fields: pluck_into_map(&data, RESPIRATION_FIELDS),
        }
        .render(fmt),
        Err(e) => format!("Error retrieving respiration data for {date}: {e}"),
    }
}

pub async fn spo2_data(api: &GarminApiClient, date: &str, fmt: OutputFormat) -> String {
    let endpoint = format!("/wellness-service/wellness/daily/spo2/{}", date);
    match api.api_json(&endpoint, None).await {
        Ok(data) => {
            let fields = pluck_into_map(&data, SPO2_FIELDS);
            if fields.len() <= 1 {
                return format!("No SpO2 data for {date} — requires overnight SpO2 monitoring enabled on the watch.");
            }
            FlatSummary { fields }.render(fmt)
        }
        Err(e) => format!("Error retrieving SpO2 data for {date}: {e}"),
    }
}

pub async fn all_day_events(api: &GarminApiClient, date: &str) -> String {
    let endpoint = format!("/wellness-service/wellness/dailyEvents/{}", date);
    match api.api_json(&endpoint, None).await {
        Ok(data) => serde_json::to_string_pretty(&data).unwrap_or_else(|e| format!("Error: {e}")),
        Err(e) => format!("Error retrieving all-day events for {date}: {e}"),
    }
}

pub async fn hrv_data(api: &GarminApiClient, date: &str, fmt: OutputFormat) -> String {
    let endpoint = format!("/hrv-service/hrv/{}", date);
    match api.api_json(&endpoint, None).await {
        Ok(data)
            if data.is_null()
                || crate::client::detect_garmin_error(&data).is_some()
                || data.get("hrvSummary").is_none() =>
        {
            format!("No HRV data for {date} — HRV tracking requires sleeping with an HRV-capable watch.")
        }
        Ok(data) => {
            let summary = data
                .get("hrvSummary")
                .and_then(|v| v.as_object())
                .cloned()
                .unwrap_or_default();
            let readings = data
                .get("hrvReadings")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_object().cloned())
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            let payload = HrvPayload {
                date: date.to_string(),
                summary,
                readings,
            };
            payload.render(fmt)
        }
        Err(e) => format!("Error retrieving HRV data for {date}: {e}"),
    }
}

pub async fn fitnessage_data(api: &GarminApiClient, date: &str) -> String {
    let endpoint = format!("/metrics-service/metrics/fitnessAge/detail/{}", date);
    match api.api_json(&endpoint, None).await {
        Ok(data) => serde_json::to_string_pretty(&data).unwrap_or_else(|e| format!("Error: {e}")),
        Err(e) => format!("Error retrieving fitness age data for {date}: {e}"),
    }
}

pub async fn endurance_score(api: &GarminApiClient, date: &str) -> String {
    let params = HashMap::from([("calendarDate".to_string(), date.to_string())]);
    match api
        .api_json(
            "/metrics-service/metrics/enduranceScore/detail",
            Some(params),
        )
        .await
    {
        Ok(data) => serde_json::to_string_pretty(&data).unwrap_or_else(|e| format!("Error: {e}")),
        Err(e) => format!("Error retrieving endurance score for {date}: {e}"),
    }
}

pub async fn hill_score(api: &GarminApiClient, date: &str) -> String {
    let params = HashMap::from([("calendarDate".to_string(), date.to_string())]);
    match api
        .api_json("/metrics-service/metrics/hillScore", Some(params))
        .await
    {
        Ok(data) => serde_json::to_string_pretty(&data).unwrap_or_else(|e| format!("Error: {e}")),
        Err(e) => format!("Error retrieving hill score for {date}: {e}"),
    }
}

pub async fn lactate_threshold(api: &GarminApiClient, date: &str) -> String {
    let params = HashMap::from([("calendarDate".to_string(), date.to_string())]);
    match api
        .api_json(
            "/metrics-service/metrics/lactateThreshold/detail",
            Some(params),
        )
        .await
    {
        Ok(data) => serde_json::to_string_pretty(&data).unwrap_or_else(|e| format!("Error: {e}")),
        Err(e) => format!("Error retrieving lactate threshold for {date}: {e}"),
    }
}

pub async fn daily_weigh_ins(
    api: &GarminApiClient,
    start_date: &str,
    end_date: &str,
    fmt: OutputFormat,
) -> String {
    let params = HashMap::from([
        ("startDate".to_string(), start_date.to_string()),
        ("endDate".to_string(), end_date.to_string()),
    ]);
    match api
        .api_json("/weight-service/weight/dateRange", Some(params))
        .await
    {
        Ok(data) => {
            let rows = extract_event_rows(&data, &["dateWeightList", "weighIns", "items"]);
            EventTable {
                columns: vec![
                    "calendarDate",
                    "weighInTimestampGMT",
                    "weight",
                    "bmi",
                    "bodyFat",
                    "bodyWater",
                    "boneMass",
                    "muscleMass",
                    "metabolicAge",
                    "physiqueRating",
                    "visceralFat",
                    "sourceType",
                ],
                rows,
            }
            .render(fmt)
        }
        Err(e) => format!("Error retrieving weigh-ins from {start_date} to {end_date}: {e}"),
    }
}

// ----- helpers -----

fn pluck_into_map(data: &Value, fields: &[(&str, &str)]) -> Map<String, Value> {
    let mut out = Map::new();
    for (src, dst) in fields {
        if let Some(v) = data.get(*src) {
            if !v.is_null() {
                out.insert((*dst).to_string(), v.clone());
            }
        }
    }
    out
}

const STATS_FIELDS: &[(&str, &str)] = &[
    ("calendarDate", "date"),
    ("totalSteps", "total_steps"),
    ("dailyStepGoal", "daily_step_goal"),
    ("totalDistanceMeters", "distance_meters"),
    ("totalKilocalories", "total_calories"),
    ("activeKilocalories", "active_calories"),
    ("bmrKilocalories", "bmr_calories"),
    ("highlyActiveSeconds", "highly_active_seconds"),
    ("activeSeconds", "active_seconds"),
    ("sedentarySeconds", "sedentary_seconds"),
    ("moderateIntensityMinutes", "moderate_intensity_minutes"),
    ("vigorousIntensityMinutes", "vigorous_intensity_minutes"),
    ("intensityMinutesGoal", "intensity_minutes_goal"),
    ("minHeartRate", "min_heart_rate_bpm"),
    ("maxHeartRate", "max_heart_rate_bpm"),
    ("restingHeartRate", "resting_heart_rate_bpm"),
    (
        "lastSevenDaysAvgRestingHeartRate",
        "last_7_days_avg_resting_hr",
    ),
    ("averageStressLevel", "avg_stress_level"),
    ("maxStressLevel", "max_stress_level"),
    ("stressQualifier", "stress_qualifier"),
    ("bodyBatteryChargedValue", "body_battery_charged"),
    ("bodyBatteryDrainedValue", "body_battery_drained"),
    ("bodyBatteryHighestValue", "body_battery_highest"),
    ("bodyBatteryLowestValue", "body_battery_lowest"),
    ("bodyBatteryMostRecentValue", "body_battery_current"),
    ("averageSpo2", "avg_spo2_percent"),
    ("lowestSpo2", "lowest_spo2_percent"),
    ("avgWakingRespirationValue", "avg_waking_respiration"),
    ("highestRespirationValue", "highest_respiration"),
    ("lowestRespirationValue", "lowest_respiration"),
];

const SLEEP_FIELDS: &[(&str, &str)] = &[
    ("calendarDate", "date"),
    ("sleepTimeSeconds", "total_sleep_seconds"),
    ("napTimeSeconds", "nap_time_seconds"),
    ("deepSleepSeconds", "deep_sleep_seconds"),
    ("lightSleepSeconds", "light_sleep_seconds"),
    ("remSleepSeconds", "rem_sleep_seconds"),
    ("awakeSleepSeconds", "awake_seconds"),
    ("unmeasurableSleepSeconds", "unmeasurable_seconds"),
    ("sleepStartTimestampLocal", "sleep_start_local"),
    ("sleepEndTimestampLocal", "sleep_end_local"),
    ("sleepStartTimestampGMT", "sleep_start_gmt"),
    ("sleepEndTimestampGMT", "sleep_end_gmt"),
    ("averageRespirationValue", "avg_respiration"),
    ("lowestRespirationValue", "lowest_respiration"),
    ("highestRespirationValue", "highest_respiration"),
    ("averageSpo2Value", "avg_spo2_percent"),
    ("lowestSpo2Value", "lowest_spo2_percent"),
    ("awakeCount", "awake_count"),
    ("restlessMomentsCount", "restless_moments_count"),
    ("avgSleepStress", "avg_sleep_stress"),
    ("sleepScores", "sleep_scores"),
];

const HEART_RATE_FIELDS: &[(&str, &str)] = &[
    ("calendarDate", "date"),
    ("restingHeartRate", "resting_heart_rate_bpm"),
    ("maxHeartRate", "max_heart_rate_bpm"),
    ("minHeartRate", "min_heart_rate_bpm"),
    (
        "lastSevenDaysAvgRestingHeartRate",
        "last_7_days_avg_resting_hr",
    ),
    ("startTimestampLocal", "start_local"),
    ("endTimestampLocal", "end_local"),
];

const STRESS_FIELDS: &[(&str, &str)] = &[
    ("calendarDate", "date"),
    ("avgStressLevel", "avg_stress_level"),
    ("maxStressLevel", "max_stress_level"),
    ("stressChartValueOffset", "stress_chart_offset"),
    ("stressChartYAxisOrigin", "stress_chart_y_origin"),
    ("startTimestampLocal", "start_local"),
    ("endTimestampLocal", "end_local"),
];

const BODY_BATTERY_FIELDS: &[(&str, &str)] = &[
    ("date", "date"),
    ("charged", "charged"),
    ("drained", "drained"),
];

const RESPIRATION_FIELDS: &[(&str, &str)] = &[
    ("calendarDate", "date"),
    ("lowestRespirationValue", "lowest_breaths_per_min"),
    ("highestRespirationValue", "highest_breaths_per_min"),
    ("avgWakingRespirationValue", "avg_waking_breaths_per_min"),
    ("avgSleepRespirationValue", "avg_sleep_breaths_per_min"),
];

// Note Garmin's casing inconsistency: the SpO2-dedicated endpoint
// returns "SpO2" (capital O), while the daily-summary endpoint uses
// "Spo2" (lowercase o).  STATS_FIELDS already uses the lowercase form;
// this constant is for the dedicated SpO2 endpoint only.
const SPO2_FIELDS: &[(&str, &str)] = &[
    ("calendarDate", "date"),
    ("averageSpO2", "avg_spo2_percent"),
    ("lowestSpO2", "lowest_spo2_percent"),
    ("latestSpO2", "latest_spo2_percent"),
    ("latestSpO2TimestampLocal", "latest_reading_local"),
    ("lastSevenDaysAvgSpO2", "last_7_days_avg_spo2"),
    ("avgSleepSpO2", "avg_sleep_spo2_percent"),
];

/// Convert Garmin's `[[ts_ms, value], …]` arrays into a typed Vec.
/// Drops any malformed rows (length < 2 or non-integer ts).
fn parse_ts_value_pairs(field: Option<&Value>) -> Vec<(i64, Value)> {
    let arr = match field.and_then(|v| v.as_array()) {
        Some(a) => a,
        None => return Vec::new(),
    };
    arr.iter()
        .filter_map(|row| {
            let pair = row.as_array()?;
            let ts = pair.first()?.as_i64()?;
            let val = pair.get(1).cloned().unwrap_or(Value::Null);
            Some((ts, val))
        })
        .collect()
}

/// Pull the row-array out of a Garmin date-range response.
///
/// Garmin's wrappers vary by endpoint (sometimes a top-level array,
/// sometimes wrapped in `measurementSummaries` / `dateWeightList` / etc.).
/// `wrapper_keys` is a list of possible wrapping field names tried in order.
fn extract_event_rows(data: &Value, wrapper_keys: &[&str]) -> Vec<Map<String, Value>> {
    let array = data.as_array().cloned().or_else(|| {
        wrapper_keys
            .iter()
            .find_map(|k| data.get(*k).and_then(|v| v.as_array()).cloned())
    });
    array
        .unwrap_or_default()
        .into_iter()
        .filter_map(|v| v.as_object().cloned())
        .collect()
}
