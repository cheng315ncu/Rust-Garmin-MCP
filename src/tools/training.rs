use std::collections::HashMap;

use crate::client::GarminApiClient;

pub async fn training_status(api: &GarminApiClient, date: &str) -> String {
    let endpoint = format!(
        "/metrics-service/metrics/trainingstatus/aggregated/{}",
        date
    );
    match api.api_json(&endpoint, None).await {
        Ok(data) => serde_json::to_string_pretty(&data).unwrap_or_else(|e| format!("Error: {e}")),
        Err(e) => format!("Error retrieving training status for {date}: {e}"),
    }
}

pub async fn progress_summary(
    api: &GarminApiClient,
    start_date: &str,
    end_date: &str,
    activity_type: Option<&str>,
) -> String {
    let mut params = HashMap::from([
        ("startDate".to_string(), start_date.to_string()),
        ("endDate".to_string(), end_date.to_string()),
        ("aggregation".to_string(), "weekly".to_string()),
        ("groupByParentActivityType".to_string(), "true".to_string()),
    ]);
    if let Some(at) = activity_type {
        if !at.is_empty() {
            params.insert("activityType".to_string(), at.to_string());
        }
    }
    match api
        .api_json("/fitnessstats-service/statistics/activities", Some(params))
        .await
    {
        Ok(data) => serde_json::to_string_pretty(&data).unwrap_or_else(|e| format!("Error: {e}")),
        Err(e) => format!("Error retrieving progress summary from {start_date} to {end_date}: {e}"),
    }
}

pub async fn race_predictions(api: &GarminApiClient) -> String {
    let name = match api.require_display_name() {
        Ok(n) => n.to_string(),
        Err(e) => return e,
    };
    let endpoint = format!("/fitnessstats-service/statistics/racePredictions/{}", name);
    match api.api_json(&endpoint, None).await {
        Ok(data) => serde_json::to_string_pretty(&data).unwrap_or_else(|e| format!("Error: {e}")),
        Err(e) => format!("Error retrieving race predictions: {e}"),
    }
}
