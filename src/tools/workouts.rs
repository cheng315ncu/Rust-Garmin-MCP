use std::collections::HashMap;

use crate::client::GarminApiClient;

pub async fn get_workouts(api: &GarminApiClient, start: u32, limit: u32) -> String {
    let params = HashMap::from([
        ("start".to_string(), start.to_string()),
        ("limit".to_string(), limit.clamp(1, 100).to_string()),
    ]);
    match api.api_json("/workout-service/workout", Some(params)).await {
        Ok(data) => crate::client::render_or_friendly(
            &data,
            "No workouts available — this endpoint may require Garmin Connect+ or coach permissions.",
        ),
        Err(e) => format!("Error retrieving workouts: {e}"),
    }
}

pub async fn get_workout_by_id(api: &GarminApiClient, workout_id: &str) -> String {
    let endpoint = format!("/workout-service/workout/{}", workout_id);
    match api.api_json(&endpoint, None).await {
        Ok(data) => {
            crate::client::render_or_friendly(&data, &format!("Workout {workout_id} not found."))
        }
        Err(e) => format!("Error retrieving workout {workout_id}: {e}"),
    }
}

pub async fn get_scheduled_workouts(
    api: &GarminApiClient,
    start_date: &str,
    end_date: &str,
) -> String {
    let params = HashMap::from([
        ("startDate".to_string(), start_date.to_string()),
        ("endDate".to_string(), end_date.to_string()),
    ]);
    match api
        .api_json("/workout-service/schedule", Some(params))
        .await
    {
        Ok(data) => crate::client::render_or_friendly(
            &data,
            &format!("No scheduled workouts between {start_date} and {end_date}."),
        ),
        Err(e) => {
            format!("Error retrieving scheduled workouts from {start_date} to {end_date}: {e}")
        }
    }
}

pub async fn delete_workout(api: &GarminApiClient, workout_id: &str) -> String {
    let endpoint = format!("/workout-service/workout/{}", workout_id);
    match api.api_delete(&endpoint).await {
        Ok(_) => format!("Workout {workout_id} deleted successfully"),
        Err(e) => format!("Error deleting workout {workout_id}: {e}"),
    }
}

pub async fn schedule_workout(api: &GarminApiClient, workout_id: &str, date: &str) -> String {
    let endpoint = format!("/workout-service/schedule/{}", workout_id);
    let body = serde_json::json!({ "date": date });
    match api.api_post_json(&endpoint, body).await {
        Ok(data) => serde_json::to_string_pretty(&data).unwrap_or_else(|e| format!("Error: {e}")),
        Err(e) => format!("Error scheduling workout {workout_id} on {date}: {e}"),
    }
}
