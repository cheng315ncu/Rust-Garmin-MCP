use crate::client::GarminApiClient;

// `/userprofile-service/userprofile` and `/userprofile-service/userprofile/v2/information`
// both 404 today — confirmed live, and by auth.rs's own resolve_display_name() probe, which
// already falls back past v2/information to socialProfile at startup. socialProfile carries
// the same identity data (fullName, displayName, email as userName) plus privacy/visibility
// settings, so the four tools below use it — and userprofile/settings for locale/unit-format
// preferences — instead of the dead paths.
const SOCIAL_PROFILE: &str = "/userprofile-service/socialProfile";
const PROFILE_SETTINGS: &str = "/userprofile-service/userprofile/settings";

pub async fn get_user_profile(api: &GarminApiClient) -> String {
    match api.api_json(SOCIAL_PROFILE, None).await {
        Ok(data) => serde_json::to_string_pretty(&data).unwrap_or_else(|e| format!("Error: {e}")),
        Err(e) => format!("Error retrieving user profile: {e}"),
    }
}

pub async fn get_userprofile_settings(api: &GarminApiClient) -> String {
    match api.api_json(PROFILE_SETTINGS, None).await {
        Ok(data) => serde_json::to_string_pretty(&data).unwrap_or_else(|e| format!("Error: {e}")),
        Err(e) => format!("Error retrieving profile settings: {e}"),
    }
}

pub async fn get_full_name(api: &GarminApiClient) -> String {
    match api.api_json(SOCIAL_PROFILE, None).await {
        Ok(data) => {
            let full_name = data.get("fullName").and_then(|v| v.as_str()).unwrap_or("");
            let display = data
                .get("displayName")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            serde_json::to_string_pretty(&serde_json::json!({
                "full_name": full_name,
                "display_name": display,
            }))
            .unwrap_or_else(|e| format!("Error: {e}"))
        }
        Err(e) => format!("Error retrieving full name: {e}"),
    }
}

pub async fn get_unit_system(api: &GarminApiClient) -> String {
    match api.api_json(PROFILE_SETTINGS, None).await {
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
