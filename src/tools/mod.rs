mod activities;
mod challenges;
mod data_management;
mod devices;
mod gear;
mod health_wellness;
mod nutrition;
mod output;
mod research;
mod training;
mod user_profile;
mod womens_health;
mod workouts;

use std::sync::Arc;

use rmcp::{handler::server::wrapper::Parameters, schemars, tool, tool_router};
use serde::Deserialize;

use crate::client::GarminApiClient;
use self::output::OutputFormat;

#[derive(Clone)]
pub struct GarminMcpServer {
    api: Arc<GarminApiClient>,
}

impl GarminMcpServer {
    pub fn new(api: GarminApiClient) -> Self {
        Self { api: Arc::new(api) }
    }
}

// ----- Parameter structs -----

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct DateParam {
    /// Date in YYYY-MM-DD format
    date: String,
}

/// Used by clinically-meaningful tools that support CSV export.
/// `format` is optional; omit it to keep the legacy JSON behaviour.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct DateFormatParam {
    /// Date in YYYY-MM-DD format
    date: String,
    /// Output format: "json" (default — pretty-printed object) or
    /// "csv" (one-row summary, or one-row-per-sample for time-series tools)
    #[serde(default)]
    format: Option<OutputFormat>,
}

/// Date-range variant for clinical tools (blood pressure, weigh-ins).
#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct DateRangeFormatParam {
    /// Start date in YYYY-MM-DD format
    start_date: String,
    /// End date in YYYY-MM-DD format
    end_date: String,
    /// Output format: "json" (default — array of events) or
    /// "csv" (header + one row per event)
    #[serde(default)]
    format: Option<OutputFormat>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct DateRangeParam {
    /// Start date in YYYY-MM-DD format
    start_date: String,
    /// End date in YYYY-MM-DD format
    end_date: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct GetActivitiesByDateParams {
    /// Start date in YYYY-MM-DD format
    start_date: String,
    /// End date in YYYY-MM-DD format
    end_date: String,
    /// Optional activity type filter (running, cycling, swimming, hiking, walking, other)
    #[serde(default)]
    activity_type: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct GetActivityParams {
    /// Garmin activity ID (numeric string)
    activity_id: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct GetRecentActivitiesParams {
    /// Number of most recent activities to return (1–50, default 10)
    #[serde(default)]
    limit: Option<u32>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct GetProgressSummaryParams {
    /// Start date in YYYY-MM-DD format
    start_date: String,
    /// End date in YYYY-MM-DD format
    end_date: String,
    /// Optional activity type filter (running, cycling, swimming, etc.)
    #[serde(default)]
    activity_type: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct GetWorkoutsParams {
    /// Starting offset (default 0)
    #[serde(default)]
    start: Option<u32>,
    /// Number of workouts to return (1–100, default 20)
    #[serde(default)]
    limit: Option<u32>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct WorkoutIdParam {
    /// Garmin workout ID
    workout_id: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ScheduleWorkoutParams {
    /// Garmin workout ID
    workout_id: String,
    /// Date to schedule the workout (YYYY-MM-DD)
    date: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct DeviceIdParam {
    /// Garmin device ID
    device_id: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct GearActivityParams {
    /// Gear UUID
    gear_uuid: String,
    /// Garmin activity ID
    activity_id: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct AdhocChallengesParams {
    /// Starting offset (default 0)
    #[serde(default)]
    start: Option<u32>,
    /// Number of results (1–100, default 20)
    #[serde(default)]
    limit: Option<u32>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct GoalsParams {
    /// Optional goal type filter (e.g. "step", "calories", "distance")
    #[serde(default)]
    goal_type: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct AddHydrationParams {
    /// Date in YYYY-MM-DD format
    date: String,
    /// Fluid intake in milliliters
    value_in_ml: f64,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct SetBloodPressureParams {
    /// Date/time (YYYY-MM-DD or YYYY-MM-DDTHH:MM:SS)
    date: String,
    /// Systolic pressure (mmHg)
    systolic: u32,
    /// Diastolic pressure (mmHg)
    diastolic: u32,
    /// Pulse (bpm)
    pulse: u32,
    /// Optional notes
    #[serde(default)]
    notes: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct AddBodyCompositionParams {
    /// Date (YYYY-MM-DD)
    date: String,
    /// Weight in kilograms
    weight_kg: f64,
    /// Body fat percentage (optional)
    #[serde(default)]
    percent_fat: Option<f64>,
    /// Muscle mass in grams (optional)
    #[serde(default)]
    muscle_mass_grams: Option<u32>,
    /// Bone mass in grams (optional)
    #[serde(default)]
    bone_mass_grams: Option<u32>,
}

// ----- MCP Tool implementations -----

#[tool_router(server_handler)]
impl GarminMcpServer {
    // ---- Activities ----

    #[tool(description = "Get a list of activities between two dates, optionally filtered by activity type (running, cycling, swimming, hiking, walking, other)")]
    async fn get_activities_by_date(
        &self,
        Parameters(p): Parameters<GetActivitiesByDateParams>,
    ) -> String {
        activities::activities_by_date(
            &self.api,
            &p.start_date,
            &p.end_date,
            p.activity_type.as_deref(),
        )
        .await
    }

    #[tool(description = "Get detailed summary for a specific activity by its Garmin activity ID")]
    async fn get_activity(&self, Parameters(p): Parameters<GetActivityParams>) -> String {
        activities::activity(&self.api, &p.activity_id).await
    }

    #[tool(description = "Get the N most recent activities (curated fields: id, name, type, start_time, distance, duration, calories, avg/max HR, steps)")]
    async fn get_recent_activities(
        &self,
        Parameters(p): Parameters<GetRecentActivitiesParams>,
    ) -> String {
        activities::recent_activities(&self.api, p.limit.unwrap_or(10)).await
    }

    #[tool(description = "Get activities that occurred on a specific date (uses Garmin's by-date endpoint)")]
    async fn get_activities_fordate(&self, Parameters(p): Parameters<DateParam>) -> String {
        activities::activities_fordate(&self.api, &p.date).await
    }

    #[tool(description = "Get lap/interval splits for a specific activity")]
    async fn get_activity_splits(&self, Parameters(p): Parameters<GetActivityParams>) -> String {
        activities::activity_splits(&self.api, &p.activity_id).await
    }

    #[tool(description = "Get typed splits (by segment type) for a specific activity")]
    async fn get_activity_typed_splits(
        &self,
        Parameters(p): Parameters<GetActivityParams>,
    ) -> String {
        activities::activity_typed_splits(&self.api, &p.activity_id).await
    }

    #[tool(description = "Get split summary statistics for a specific activity")]
    async fn get_activity_split_summaries(
        &self,
        Parameters(p): Parameters<GetActivityParams>,
    ) -> String {
        activities::activity_split_summaries(&self.api, &p.activity_id).await
    }

    #[tool(description = "Get weather conditions recorded during a specific activity")]
    async fn get_activity_weather(&self, Parameters(p): Parameters<GetActivityParams>) -> String {
        activities::activity_weather(&self.api, &p.activity_id).await
    }

    #[tool(description = "Get heart rate time-in-zone breakdown for a specific activity")]
    async fn get_activity_hr_in_timezones(
        &self,
        Parameters(p): Parameters<GetActivityParams>,
    ) -> String {
        activities::activity_hr_in_timezones(&self.api, &p.activity_id).await
    }

    #[tool(description = "Get exercise sets (strength/gym) for a specific activity")]
    async fn get_activity_exercise_sets(
        &self,
        Parameters(p): Parameters<GetActivityParams>,
    ) -> String {
        activities::activity_exercise_sets(&self.api, &p.activity_id).await
    }

    #[tool(description = "Get the gear (shoes, bike, etc.) associated with a specific activity")]
    async fn get_activity_gear(&self, Parameters(p): Parameters<GetActivityParams>) -> String {
        activities::activity_gear(&self.api, &p.activity_id).await
    }

    #[tool(description = "Get training effect (aerobic and anaerobic) for a specific activity")]
    async fn get_activity_training_effect(
        &self,
        Parameters(p): Parameters<GetActivityParams>,
    ) -> String {
        activities::activity_training_effect(&self.api, &p.activity_id).await
    }

    #[tool(description = "Get the total activity count across all time")]
    async fn count_activities(&self) -> String {
        activities::count_activities(&self.api).await
    }

    #[tool(description = "Get the list of all Garmin activity types with their keys and IDs")]
    async fn get_activity_types(&self) -> String {
        activities::activity_types(&self.api).await
    }

    // ---- Health & Wellness ----

    #[tool(description = "Get daily health stats for a date: steps, calories, heart rate, stress levels, body battery, SpO2, and intensity minutes. Set format='csv' for a single-row CSV.")]
    async fn get_stats(&self, Parameters(p): Parameters<DateFormatParam>) -> String {
        health_wellness::stats(&self.api, &p.date, p.format.unwrap_or_default()).await
    }

    #[tool(description = "Get curated sleep summary for a date: total duration, sleep stages (deep/light/REM/awake), respiration, SpO2, and sleep scores. Set format='csv' for a single-row CSV (researchers can concatenate across days).")]
    async fn get_sleep_summary(&self, Parameters(p): Parameters<DateFormatParam>) -> String {
        health_wellness::sleep_summary(&self.api, &p.date, p.format.unwrap_or_default()).await
    }

    #[tool(description = "Get daily heart rate summary for a date: resting/min/max HR and sample count. Set format='csv' for a single-row CSV.")]
    async fn get_daily_heart_rate(&self, Parameters(p): Parameters<DateFormatParam>) -> String {
        health_wellness::daily_heart_rate(&self.api, &p.date, p.format.unwrap_or_default()).await
    }

    #[tool(description = "Get daily stress summary for a date: average and max stress levels, with sample counts. Set format='csv' for a single-row CSV.")]
    async fn get_stress_summary(&self, Parameters(p): Parameters<DateFormatParam>) -> String {
        health_wellness::stress_summary(&self.api, &p.date, p.format.unwrap_or_default()).await
    }

    #[tool(description = "Get body battery summary for a date: charged, drained, starting/ending values, sample count. Set format='csv' for raw timeseries (timestamp_ms,body_battery).")]
    async fn get_body_battery_summary(&self, Parameters(p): Parameters<DateFormatParam>) -> String {
        health_wellness::body_battery_summary(&self.api, &p.date, p.format.unwrap_or_default()).await
    }

    #[tool(description = "Get daily step counts for each day in a date range")]
    async fn get_daily_steps(&self, Parameters(p): Parameters<DateRangeParam>) -> String {
        health_wellness::daily_steps(&self.api, &p.start_date, &p.end_date).await
    }

    #[tool(description = "Get training readiness score and contributing factors for a date")]
    async fn get_training_readiness(&self, Parameters(p): Parameters<DateParam>) -> String {
        health_wellness::training_readiness(&self.api, &p.date).await
    }

    #[tool(description = "Get body battery charge/drain events for a date (meals, sleep, activity, stress)")]
    async fn get_body_battery_events(&self, Parameters(p): Parameters<DateParam>) -> String {
        health_wellness::body_battery_events(&self.api, &p.date).await
    }

    #[tool(description = "Get blood pressure readings within a date range. Set format='csv' for header + one row per measurement (timestamp_local,timestamp_gmt,systolic,diastolic,pulse,notes).")]
    async fn get_blood_pressure(&self, Parameters(p): Parameters<DateRangeFormatParam>) -> String {
        health_wellness::blood_pressure(
            &self.api,
            &p.start_date,
            &p.end_date,
            p.format.unwrap_or_default(),
        )
        .await
    }

    #[tool(description = "Get floors climbed data for a date")]
    async fn get_floors(&self, Parameters(p): Parameters<DateParam>) -> String {
        health_wellness::floors(&self.api, &p.date).await
    }

    #[tool(description = "Get resting heart rate for a specific day")]
    async fn get_rhr_day(&self, Parameters(p): Parameters<DateParam>) -> String {
        health_wellness::rhr_day(&self.api, &p.date).await
    }

    #[tool(description = "Get daily hydration data (fluid intake) for a date")]
    async fn get_hydration_data(&self, Parameters(p): Parameters<DateParam>) -> String {
        health_wellness::hydration_data(&self.api, &p.date).await
    }

    #[tool(description = "Get respiration (breathing rate) summary for a date: lowest/highest/avg waking/avg sleep BPM. Set format='csv' for a single-row CSV.")]
    async fn get_respiration_data(&self, Parameters(p): Parameters<DateFormatParam>) -> String {
        health_wellness::respiration_data(&self.api, &p.date, p.format.unwrap_or_default()).await
    }

    #[tool(description = "Get SpO2 (blood oxygen saturation) summary for a date: avg/lowest/latest SpO2, 7-day avg, sleep SpO2. Set format='csv' for a single-row CSV.")]
    async fn get_spo2_data(&self, Parameters(p): Parameters<DateFormatParam>) -> String {
        health_wellness::spo2_data(&self.api, &p.date, p.format.unwrap_or_default()).await
    }

    #[tool(description = "Get all-day wellness events (activity detections, move alerts, etc.) for a date")]
    async fn get_all_day_events(&self, Parameters(p): Parameters<DateParam>) -> String {
        health_wellness::all_day_events(&self.api, &p.date).await
    }

    #[tool(description = "Get HRV (Heart Rate Variability) summary and status for a date. Default JSON returns the summary + reading count; set format='csv' to receive raw 5-minute HRV readings (reading_time_gmt,hrv_value) for biosignal analysis.")]
    async fn get_hrv_data(&self, Parameters(p): Parameters<DateFormatParam>) -> String {
        health_wellness::hrv_data(&self.api, &p.date, p.format.unwrap_or_default()).await
    }

    #[tool(description = "Get fitness age data and contributing factors for a date")]
    async fn get_fitnessage_data(&self, Parameters(p): Parameters<DateParam>) -> String {
        health_wellness::fitnessage_data(&self.api, &p.date).await
    }

    #[tool(description = "Get endurance score and detail for a date")]
    async fn get_endurance_score(&self, Parameters(p): Parameters<DateParam>) -> String {
        health_wellness::endurance_score(&self.api, &p.date).await
    }

    #[tool(description = "Get hill score (climbing ability metric) for a date")]
    async fn get_hill_score(&self, Parameters(p): Parameters<DateParam>) -> String {
        health_wellness::hill_score(&self.api, &p.date).await
    }

    #[tool(description = "Get lactate threshold data for a date")]
    async fn get_lactate_threshold(&self, Parameters(p): Parameters<DateParam>) -> String {
        health_wellness::lactate_threshold(&self.api, &p.date).await
    }

    #[tool(description = "Get weigh-in records within a date range. Set format='csv' for header + one row per weigh-in (date,timestamp_gmt,weight,bmi,bodyFat,bodyWater,boneMass,muscleMass,…).")]
    async fn get_daily_weigh_ins(&self, Parameters(p): Parameters<DateRangeFormatParam>) -> String {
        health_wellness::daily_weigh_ins(
            &self.api,
            &p.start_date,
            &p.end_date,
            p.format.unwrap_or_default(),
        )
        .await
    }

    // ---- Training ----

    #[tool(description = "Get training status for a date: VO2 max, load focus, training status phrase, and acute/chronic load")]
    async fn get_training_status(&self, Parameters(p): Parameters<DateParam>) -> String {
        training::training_status(&self.api, &p.date).await
    }

    #[tool(description = "Get weekly progress summary between two dates, optionally filtered by activity type")]
    async fn get_progress_summary_between_dates(
        &self,
        Parameters(p): Parameters<GetProgressSummaryParams>,
    ) -> String {
        training::progress_summary(&self.api, &p.start_date, &p.end_date, p.activity_type.as_deref()).await
    }

    #[tool(description = "Get race time predictions (5K, 10K, half-marathon, marathon) based on recent training")]
    async fn get_race_predictions(&self) -> String {
        training::race_predictions(&self.api).await
    }

    // ---- Workouts ----

    #[tool(description = "Get a list of saved workout plans (start offset and limit supported)")]
    async fn get_workouts(&self, Parameters(p): Parameters<GetWorkoutsParams>) -> String {
        workouts::get_workouts(&self.api, p.start.unwrap_or(0), p.limit.unwrap_or(20)).await
    }

    #[tool(description = "Get a specific workout plan by its ID")]
    async fn get_workout_by_id(&self, Parameters(p): Parameters<WorkoutIdParam>) -> String {
        workouts::get_workout_by_id(&self.api, &p.workout_id).await
    }

    #[tool(description = "Get scheduled workouts within a date range")]
    async fn get_scheduled_workouts(&self, Parameters(p): Parameters<DateRangeParam>) -> String {
        workouts::get_scheduled_workouts(&self.api, &p.start_date, &p.end_date).await
    }

    #[tool(description = "Delete a workout plan by its ID (irreversible)")]
    async fn delete_workout(&self, Parameters(p): Parameters<WorkoutIdParam>) -> String {
        workouts::delete_workout(&self.api, &p.workout_id).await
    }

    #[tool(description = "Schedule an existing workout plan on a specific date")]
    async fn schedule_workout(&self, Parameters(p): Parameters<ScheduleWorkoutParams>) -> String {
        workouts::schedule_workout(&self.api, &p.workout_id, &p.date).await
    }

    // ---- Challenges & Badges ----

    #[tool(description = "Get all badges earned by the user")]
    async fn get_earned_badges(&self) -> String {
        challenges::earned_badges(&self.api).await
    }

    #[tool(description = "Get available badge challenges the user can join")]
    async fn get_available_badge_challenges(&self) -> String {
        challenges::available_badge_challenges(&self.api).await
    }

    #[tool(description = "Get badge challenges the user has joined")]
    async fn get_badge_challenges(&self) -> String {
        challenges::badge_challenges(&self.api).await
    }

    #[tool(description = "Get badge challenges the user has not yet completed")]
    async fn get_non_completed_badge_challenges(&self) -> String {
        challenges::non_completed_badge_challenges(&self.api).await
    }

    #[tool(description = "Get ad-hoc fitness challenges (supports pagination via start/limit)")]
    async fn get_adhoc_challenges(
        &self,
        Parameters(p): Parameters<AdhocChallengesParams>,
    ) -> String {
        challenges::adhoc_challenges(&self.api, p.start.unwrap_or(0), p.limit.unwrap_or(20)).await
    }

    #[tool(description = "Get virtual challenges currently in progress")]
    async fn get_inprogress_virtual_challenges(&self) -> String {
        challenges::inprogress_virtual_challenges(&self.api).await
    }

    #[tool(description = "Get active user goals, optionally filtered by goal type")]
    async fn get_goals(&self, Parameters(p): Parameters<GoalsParams>) -> String {
        challenges::goals(&self.api, p.goal_type.as_deref()).await
    }

    #[tool(description = "Get personal records (PRs) across all activity types")]
    async fn get_personal_records(&self) -> String {
        challenges::personal_records(&self.api).await
    }

    // ---- Devices ----

    #[tool(description = "Get all registered Garmin devices for this account")]
    async fn get_devices(&self) -> String {
        devices::get_devices(&self.api).await
    }

    #[tool(description = "Get the most recently used Garmin device")]
    async fn get_device_last_used(&self) -> String {
        devices::get_device_last_used(&self.api).await
    }

    #[tool(description = "Get settings and configuration for a specific device by device ID")]
    async fn get_device_settings(&self, Parameters(p): Parameters<DeviceIdParam>) -> String {
        devices::get_device_settings(&self.api, &p.device_id).await
    }

    #[tool(description = "Get the primary training device configured for this account")]
    async fn get_primary_training_device(&self) -> String {
        devices::get_primary_training_device(&self.api).await
    }

    #[tool(description = "Get solar charging data for a solar-enabled device by device ID")]
    async fn get_device_solar_data(&self, Parameters(p): Parameters<DeviceIdParam>) -> String {
        devices::get_device_solar_data(&self.api, &p.device_id).await
    }

    #[tool(description = "Get alarms configured on a specific device by device ID")]
    async fn get_device_alarms(&self, Parameters(p): Parameters<DeviceIdParam>) -> String {
        devices::get_device_alarms(&self.api, &p.device_id).await
    }

    // ---- Gear ----

    #[tool(description = "Get all gear (shoes, bikes, etc.) associated with this account")]
    async fn get_gear(&self) -> String {
        gear::get_gear(&self.api).await
    }

    #[tool(description = "Associate a piece of gear with an activity by gear UUID and activity ID")]
    async fn add_gear_to_activity(&self, Parameters(p): Parameters<GearActivityParams>) -> String {
        gear::add_gear_to_activity(&self.api, &p.gear_uuid, &p.activity_id).await
    }

    #[tool(description = "Remove a gear association from an activity by gear UUID and activity ID")]
    async fn remove_gear_from_activity(
        &self,
        Parameters(p): Parameters<GearActivityParams>,
    ) -> String {
        gear::remove_gear_from_activity(&self.api, &p.gear_uuid, &p.activity_id).await
    }

    // ---- User Profile ----

    #[tool(description = "Get the user's full Garmin profile information")]
    async fn get_user_profile(&self) -> String {
        user_profile::get_user_profile(&self.api).await
    }

    #[tool(description = "Get the user's profile settings (privacy, language, etc.)")]
    async fn get_userprofile_settings(&self) -> String {
        user_profile::get_userprofile_settings(&self.api).await
    }

    #[tool(description = "Get the user's full name and display name")]
    async fn get_full_name(&self) -> String {
        user_profile::get_full_name(&self.api).await
    }

    #[tool(description = "Get the user's preferred measurement system (metric or statute/imperial)")]
    async fn get_unit_system(&self) -> String {
        user_profile::get_unit_system(&self.api).await
    }

    // ---- Women's Health ----

    #[tool(description = "Get menstrual cycle data for a specific date")]
    async fn get_menstrual_data_for_date(&self, Parameters(p): Parameters<DateParam>) -> String {
        womens_health::get_menstrual_data_for_date(&self.api, &p.date).await
    }

    #[tool(description = "Get menstrual calendar history within a date range")]
    async fn get_menstrual_calendar_data(
        &self,
        Parameters(p): Parameters<DateRangeParam>,
    ) -> String {
        womens_health::get_menstrual_calendar_data(&self.api, &p.start_date, &p.end_date).await
    }

    #[tool(description = "Get pregnancy tracking summary and milestones")]
    async fn get_pregnancy_summary(&self) -> String {
        womens_health::get_pregnancy_summary(&self.api).await
    }

    // ---- Nutrition ----

    #[tool(description = "Get the daily food/nutrition log for a specific date")]
    async fn get_nutrition_daily_food_log(&self, Parameters(p): Parameters<DateParam>) -> String {
        nutrition::get_nutrition_daily_food_log(&self.api, &p.date).await
    }

    #[tool(description = "Get nutrition goal settings (calorie/macro targets)")]
    async fn get_nutrition_daily_settings(&self) -> String {
        nutrition::get_nutrition_daily_settings(&self.api).await
    }

    #[tool(description = "Get the user's custom food database")]
    async fn get_custom_foods(&self) -> String {
        nutrition::get_custom_foods(&self.api).await
    }

    // ---- Research / Longitudinal Analysis ----

    #[tool(description = "Fetch daily health stats (steps, calories, HR, stress, body battery, SpO2, respiration — 20 metrics) for each day in a date range. Returns a JSON array or set format='csv' for a panel dataset suitable for pandas/R import. Max 366 days per call. Days without data appear as date-only rows.")]
    async fn get_daily_stats_range(&self, Parameters(p): Parameters<DateRangeFormatParam>) -> String {
        research::daily_stats_range(&self.api, &p.start_date, &p.end_date, p.format.unwrap_or_default()).await
    }

    #[tool(description = "Fetch sleep summary (total/deep/light/REM/awake seconds, SpO2, respiration, stress, awake count — 16 metrics) for each day in a date range. Returns a JSON array or set format='csv' for longitudinal sleep analysis. Max 366 days per call.")]
    async fn get_sleep_range(&self, Parameters(p): Parameters<DateRangeFormatParam>) -> String {
        research::sleep_range(&self.api, &p.start_date, &p.end_date, p.format.unwrap_or_default()).await
    }

    #[tool(description = "Fetch HRV summary (weekly avg, last night, 5-min high/low, baseline, status, feedback) for each day in a date range. Returns a JSON array or set format='csv' for autonomic nervous system research. Days without HRV data appear as date-only rows. Max 366 days per call.")]
    async fn get_hrv_range(&self, Parameters(p): Parameters<DateRangeFormatParam>) -> String {
        research::hrv_range(&self.api, &p.start_date, &p.end_date, p.format.unwrap_or_default()).await
    }

    #[tool(description = "Compute per-ISO-week statistics (mean, std, min, max) of 12 key health metrics (steps, calories, HR, stress, body battery, SpO2, respiration) over a date range. Returns a JSON array with one object per week including week label, week_start/end dates, and stat columns. Max 366 days per call.")]
    async fn get_weekly_summary(&self, Parameters(p): Parameters<DateRangeParam>) -> String {
        research::weekly_summary(&self.api, &p.start_date, &p.end_date).await
    }

    // ---- Data Management (write operations) ----

    #[tool(description = "Log hydration data for a date. value_in_ml is fluid intake in milliliters.")]
    async fn add_hydration_data(
        &self,
        Parameters(p): Parameters<AddHydrationParams>,
    ) -> String {
        data_management::add_hydration_data(&self.api, &p.date, p.value_in_ml).await
    }

    #[tool(description = "Record a blood pressure measurement. date can be YYYY-MM-DD or YYYY-MM-DDTHH:MM:SS.")]
    async fn set_blood_pressure(
        &self,
        Parameters(p): Parameters<SetBloodPressureParams>,
    ) -> String {
        data_management::set_blood_pressure(
            &self.api,
            &p.date,
            p.systolic,
            p.diastolic,
            p.pulse,
            p.notes.as_deref(),
        )
        .await
    }

    #[tool(description = "Record body composition data (weight required; body fat %, muscle mass, bone mass optional). Weight in kg; masses in grams.")]
    async fn add_body_composition(
        &self,
        Parameters(p): Parameters<AddBodyCompositionParams>,
    ) -> String {
        data_management::add_body_composition(
            &self.api,
            &p.date,
            p.weight_kg,
            p.percent_fat,
            p.muscle_mass_grams,
            p.bone_mass_grams,
        )
        .await
    }
}
