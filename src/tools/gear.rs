use crate::client::GarminApiClient;

pub async fn get_gear(api: &GarminApiClient) -> String {
    let name = match api.require_display_name() {
        Ok(n) => n.to_string(),
        Err(e) => return e,
    };
    let endpoint = format!("/gear-service/gear/user/{}", name);
    match api.api_json(&endpoint, None).await {
        Ok(data) => serde_json::to_string_pretty(&data).unwrap_or_else(|e| format!("Error: {e}")),
        Err(e) => format!("Error retrieving gear: {e}"),
    }
}

pub async fn add_gear_to_activity(
    api: &GarminApiClient,
    gear_uuid: &str,
    activity_id: &str,
) -> String {
    let endpoint = format!("/gear-service/gear/{}/activity/{}", gear_uuid, activity_id);
    match api.api_post_json(&endpoint, serde_json::Value::Null).await {
        Ok(data) => serde_json::to_string_pretty(&data).unwrap_or_else(|e| format!("Error: {e}")),
        Err(e) => format!("Error adding gear {gear_uuid} to activity {activity_id}: {e}"),
    }
}

pub async fn remove_gear_from_activity(
    api: &GarminApiClient,
    gear_uuid: &str,
    activity_id: &str,
) -> String {
    let endpoint = format!("/gear-service/gear/{}/activity/{}", gear_uuid, activity_id);
    match api.api_delete(&endpoint).await {
        Ok(_) => format!("Gear {gear_uuid} removed from activity {activity_id}"),
        Err(e) => format!("Error removing gear {gear_uuid} from activity {activity_id}: {e}"),
    }
}
