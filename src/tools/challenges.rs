use std::collections::HashMap;

use crate::client::GarminApiClient;

pub async fn earned_badges(api: &GarminApiClient) -> String {
    let name = match api.require_display_name() {
        Ok(n) => n.to_string(),
        Err(e) => return e,
    };
    let endpoint = format!("/badge-service/badge/earned/{}", name);
    match api.api_json(&endpoint, None).await {
        Ok(data) => serde_json::to_string_pretty(&data).unwrap_or_else(|e| format!("Error: {e}")),
        Err(e) => format!("Error retrieving earned badges: {e}"),
    }
}

pub async fn available_badge_challenges(api: &GarminApiClient) -> String {
    match api.api_json("/badge-service/badge/available", None).await {
        Ok(data) => serde_json::to_string_pretty(&data).unwrap_or_else(|e| format!("Error: {e}")),
        Err(e) => format!("Error retrieving available badge challenges: {e}"),
    }
}

pub async fn badge_challenges(api: &GarminApiClient) -> String {
    let name = match api.require_display_name() {
        Ok(n) => n.to_string(),
        Err(e) => return e,
    };
    let endpoint = format!("/badge-service/badge/challenges/{}", name);
    match api.api_json(&endpoint, None).await {
        Ok(data) => serde_json::to_string_pretty(&data).unwrap_or_else(|e| format!("Error: {e}")),
        Err(e) => format!("Error retrieving badge challenges: {e}"),
    }
}

pub async fn non_completed_badge_challenges(api: &GarminApiClient) -> String {
    let name = match api.require_display_name() {
        Ok(n) => n.to_string(),
        Err(e) => return e,
    };
    let endpoint = format!("/badge-service/badge/challenges/non-completed/{}", name);
    match api.api_json(&endpoint, None).await {
        Ok(data) => serde_json::to_string_pretty(&data).unwrap_or_else(|e| format!("Error: {e}")),
        Err(e) => format!("Error retrieving non-completed badge challenges: {e}"),
    }
}

pub async fn adhoc_challenges(api: &GarminApiClient, start: u32, limit: u32) -> String {
    let params = HashMap::from([
        ("start".to_string(), start.to_string()),
        ("limit".to_string(), limit.clamp(1, 100).to_string()),
    ]);
    match api
        .api_json("/fitnesschallenge-service/challenge/adhoc", Some(params))
        .await
    {
        Ok(data) => serde_json::to_string_pretty(&data).unwrap_or_else(|e| format!("Error: {e}")),
        Err(e) => format!("Error retrieving adhoc challenges: {e}"),
    }
}

pub async fn inprogress_virtual_challenges(api: &GarminApiClient) -> String {
    match api
        .api_json(
            "/fitnesschallenge-service/challenge/virtual/in-progress",
            None,
        )
        .await
    {
        Ok(data) => serde_json::to_string_pretty(&data).unwrap_or_else(|e| format!("Error: {e}")),
        Err(e) => format!("Error retrieving in-progress virtual challenges: {e}"),
    }
}

pub async fn goals(api: &GarminApiClient, goal_type: Option<&str>) -> String {
    let mut params = HashMap::from([
        ("status".to_string(), "active".to_string()),
        ("start".to_string(), "1".to_string()),
        ("limit".to_string(), "30".to_string()),
        ("sortOrder".to_string(), "asc".to_string()),
    ]);
    if let Some(gt) = goal_type {
        if !gt.is_empty() {
            params.insert("goalType".to_string(), gt.to_string());
        }
    }
    match api.api_json("/goal-service/goal/list", Some(params)).await {
        Ok(data) => crate::client::render_or_friendly(&data, "No active goals."),
        Err(e) => format!("Error retrieving goals: {e}"),
    }
}

pub async fn personal_records(api: &GarminApiClient) -> String {
    let name = match api.require_display_name() {
        Ok(n) => n.to_string(),
        Err(e) => return e,
    };
    let endpoint = format!("/personalrecord-service/personalrecord/prs/{}", name);
    match api.api_json(&endpoint, None).await {
        Ok(data) => serde_json::to_string_pretty(&data).unwrap_or_else(|e| format!("Error: {e}")),
        Err(e) => format!("Error retrieving personal records: {e}"),
    }
}
