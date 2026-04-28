use crate::client::GarminApiClient;

pub async fn get_user_profile(api: &GarminApiClient) -> String {
    match api.api_json("/userprofile-service/userprofile", None).await {
        Ok(data) => serde_json::to_string_pretty(&data).unwrap_or_else(|e| format!("Error: {e}")),
        Err(e) => format!("Error retrieving user profile: {e}"),
    }
}

pub async fn get_userprofile_settings(api: &GarminApiClient) -> String {
    match api
        .api_json("/userprofile-service/social-profile/settings", None)
        .await
    {
        Ok(data) => serde_json::to_string_pretty(&data).unwrap_or_else(|e| format!("Error: {e}")),
        Err(e) => format!("Error retrieving profile settings: {e}"),
    }
}

pub async fn get_full_name(api: &GarminApiClient) -> String {
    match api
        .api_json("/userprofile-service/userprofile/v2/information", None)
        .await
    {
        Ok(data) => {
            let first = data
                .get("firstName")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let last = data
                .get("lastName")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let display = data
                .get("displayName")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            serde_json::to_string_pretty(&serde_json::json!({
                "full_name": format!("{} {}", first, last).trim().to_string(),
                "display_name": display,
            }))
            .unwrap_or_else(|e| format!("Error: {e}"))
        }
        Err(e) => format!("Error retrieving full name: {e}"),
    }
}

pub async fn get_unit_system(api: &GarminApiClient) -> String {
    match api
        .api_json("/userprofile-service/userprofile/v2/information", None)
        .await
    {
        Ok(data) => {
            let unit = data
                .get("measurementSystem")
                .cloned()
                .unwrap_or(serde_json::Value::Null);
            serde_json::to_string_pretty(&serde_json::json!({ "measurement_system": unit }))
                .unwrap_or_else(|e| format!("Error: {e}"))
        }
        Err(e) => format!("Error retrieving unit system: {e}"),
    }
}
