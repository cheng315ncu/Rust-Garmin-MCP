use std::collections::HashMap;

use serde_json::Value;

use crate::client::GarminApiClient;

const ACTIVITIES_SEARCH: &str = "/activitylist-service/activities/search/activities";

pub async fn activities_by_date(
    api: &GarminApiClient,
    start_date: &str,
    end_date: &str,
    activity_type: Option<&str>,
) -> String {
    let limit = 20usize;
    let mut all: Vec<Value> = Vec::new();
    let mut start = 0usize;

    loop {
        let mut params = HashMap::from([
            ("startDate".to_string(), start_date.to_string()),
            ("endDate".to_string(), end_date.to_string()),
            ("start".to_string(), start.to_string()),
            ("limit".to_string(), limit.to_string()),
        ]);
        if let Some(at) = activity_type {
            if !at.is_empty() {
                params.insert("activityType".to_string(), at.to_string());
            }
        }

        match api.api_json(ACTIVITIES_SEARCH, Some(params)).await {
            Ok(Value::Array(arr)) if !arr.is_empty() => {
                all.extend(arr);
                start += limit;
            }
            _ => break,
        }
    }

    if all.is_empty() {
        return format!(
            "No activities found between {} and {}",
            start_date, end_date
        );
    }

    let curated: Vec<Value> = all.iter().map(curate_activity).collect();
    let result = serde_json::json!({
        "count": curated.len(),
        "date_range": { "start": start_date, "end": end_date },
        "activities": curated,
    });
    serde_json::to_string_pretty(&result).unwrap_or_else(|e| format!("Serialization error: {e}"))
}

pub async fn activity(api: &GarminApiClient, activity_id: &str) -> String {
    let endpoint = format!("/activity-service/activity/{}", activity_id);
    match api.api_json(&endpoint, None).await {
        Ok(data) => serde_json::to_string_pretty(&data).unwrap_or_else(|e| format!("Error: {e}")),
        Err(e) => format!("Error retrieving activity {activity_id}: {e}"),
    }
}

pub async fn recent_activities(api: &GarminApiClient, limit: u32) -> String {
    let limit = limit.clamp(1, 50);
    let params = HashMap::from([
        ("start".to_string(), "0".to_string()),
        ("limit".to_string(), limit.to_string()),
    ]);

    match api.api_json(ACTIVITIES_SEARCH, Some(params)).await {
        Ok(Value::Array(arr)) if !arr.is_empty() => {
            let curated: Vec<Value> = arr.iter().map(curate_activity).collect();
            let result = serde_json::json!({
                "count": curated.len(),
                "activities": curated,
            });
            serde_json::to_string_pretty(&result)
                .unwrap_or_else(|e| format!("Serialization error: {e}"))
        }
        Ok(_) => "No recent activities found".to_string(),
        Err(e) => format!("Error retrieving recent activities: {e}"),
    }
}

pub async fn activities_fordate(api: &GarminApiClient, date: &str) -> String {
    let endpoint = format!("/activitylist-service/activities/fordate/{}", date);
    match api.api_json(&endpoint, None).await {
        Ok(data) => serde_json::to_string_pretty(&data).unwrap_or_else(|e| format!("Error: {e}")),
        Err(e) => format!("Error retrieving activities for date {date}: {e}"),
    }
}

pub async fn activity_splits(api: &GarminApiClient, activity_id: &str) -> String {
    let endpoint = format!("/activity-service/activity/{}/splits", activity_id);
    match api.api_json(&endpoint, None).await {
        Ok(data) => serde_json::to_string_pretty(&data).unwrap_or_else(|e| format!("Error: {e}")),
        Err(e) => format!("Error retrieving splits for activity {activity_id}: {e}"),
    }
}

pub async fn activity_typed_splits(api: &GarminApiClient, activity_id: &str) -> String {
    let endpoint = format!("/activity-service/activity/{}/typedSplits", activity_id);
    match api.api_json(&endpoint, None).await {
        Ok(data) => serde_json::to_string_pretty(&data).unwrap_or_else(|e| format!("Error: {e}")),
        Err(e) => format!("Error retrieving typed splits for activity {activity_id}: {e}"),
    }
}

pub async fn activity_split_summaries(api: &GarminApiClient, activity_id: &str) -> String {
    let endpoint = format!("/activity-service/activity/{}/splitSummaries", activity_id);
    match api.api_json(&endpoint, None).await {
        Ok(data) => serde_json::to_string_pretty(&data).unwrap_or_else(|e| format!("Error: {e}")),
        Err(e) => format!("Error retrieving split summaries for activity {activity_id}: {e}"),
    }
}

pub async fn activity_weather(api: &GarminApiClient, activity_id: &str) -> String {
    let endpoint = format!("/activity-service/activity/{}/weather", activity_id);
    match api.api_json(&endpoint, None).await {
        Ok(data) => serde_json::to_string_pretty(&data).unwrap_or_else(|e| format!("Error: {e}")),
        Err(e) => format!("Error retrieving weather for activity {activity_id}: {e}"),
    }
}

pub async fn activity_hr_in_timezones(api: &GarminApiClient, activity_id: &str) -> String {
    let endpoint = format!("/activity-service/activity/{}/hrTimeInZones", activity_id);
    match api.api_json(&endpoint, None).await {
        Ok(data) => serde_json::to_string_pretty(&data).unwrap_or_else(|e| format!("Error: {e}")),
        Err(e) => format!("Error retrieving HR time in zones for activity {activity_id}: {e}"),
    }
}

pub async fn activity_exercise_sets(api: &GarminApiClient, activity_id: &str) -> String {
    let endpoint = format!("/activity-service/activity/{}/exerciseSets", activity_id);
    match api.api_json(&endpoint, None).await {
        Ok(data) => serde_json::to_string_pretty(&data).unwrap_or_else(|e| format!("Error: {e}")),
        Err(e) => format!("Error retrieving exercise sets for activity {activity_id}: {e}"),
    }
}

pub async fn activity_gear(api: &GarminApiClient, activity_id: &str) -> String {
    let params = HashMap::from([("activityId".to_string(), activity_id.to_string())]);
    match api
        .api_json("/gear-service/gear/filterGear", Some(params))
        .await
    {
        Ok(data) => serde_json::to_string_pretty(&data).unwrap_or_else(|e| format!("Error: {e}")),
        Err(e) => format!("Error retrieving gear for activity {activity_id}: {e}"),
    }
}

pub async fn activity_training_effect(api: &GarminApiClient, activity_id: &str) -> String {
    let endpoint = format!("/activity-service/activity/{}/trainingEffect", activity_id);
    match api.api_json(&endpoint, None).await {
        Ok(data) => serde_json::to_string_pretty(&data).unwrap_or_else(|e| format!("Error: {e}")),
        Err(e) => format!("Error retrieving training effect for activity {activity_id}: {e}"),
    }
}

pub async fn count_activities(api: &GarminApiClient) -> String {
    match api
        .api_json("/activitylist-service/activities/count", None)
        .await
    {
        Ok(data) => serde_json::to_string_pretty(&data).unwrap_or_else(|e| format!("Error: {e}")),
        Err(e) => format!("Error retrieving activity count: {e}"),
    }
}

pub async fn activity_types(api: &GarminApiClient) -> String {
    match api
        .api_json("/activity-service/activity/activityTypes", None)
        .await
    {
        Ok(data) => serde_json::to_string_pretty(&data).unwrap_or_else(|e| format!("Error: {e}")),
        Err(e) => format!("Error retrieving activity types: {e}"),
    }
}

fn curate_activity(a: &Value) -> Value {
    let mut m = serde_json::Map::new();
    let pairs: &[(&[&str], &str)] = &[
        (&["activityId"], "id"),
        (&["activityName"], "name"),
        (&["activityType", "typeKey"], "type"),
        (&["startTimeLocal"], "start_time"),
        (&["distance"], "distance_meters"),
        (&["duration"], "duration_seconds"),
        (&["calories"], "calories"),
        (&["averageHR"], "avg_hr_bpm"),
        (&["maxHR"], "max_hr_bpm"),
        (&["steps"], "steps"),
    ];
    for (path, dst) in pairs {
        let mut cur = a;
        let mut ok = true;
        for key in path.iter() {
            match cur.get(*key) {
                Some(v) => cur = v,
                None => {
                    ok = false;
                    break;
                }
            }
        }
        if ok && !cur.is_null() {
            m.insert(dst.to_string(), cur.clone());
        }
    }
    Value::Object(m)
}
