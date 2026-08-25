use crate::client::{self, GarminApiClient};

// `/userprofile-service/userprofile` and `/userprofile-service/userprofile/v2/information`
// both 404 today — confirmed live, and by auth.rs's own resolve_display_name() probe, which
// already falls back past v2/information to socialProfile at startup. socialProfile carries
// the same identity data (fullName, displayName, email as userName) plus privacy/visibility
// settings, so the four tools below use it — and userprofile/settings for locale/unit-format
// preferences — instead of the dead paths. `pub` because auth.rs probes the same path: one
// place to edit when Garmin next moves it.
pub const SOCIAL_PROFILE: &str = "/userprofile-service/socialProfile";
pub const PROFILE_SETTINGS: &str = "/userprofile-service/userprofile/settings";

pub async fn get_user_profile(api: &GarminApiClient) -> String {
    match api.api_json(SOCIAL_PROFILE, None).await {
        Ok(data) => client::render_or_friendly(
            &data,
            "No profile returned by socialProfile — Garmin may have moved this endpoint.",
        ),
        Err(e) => format!("Error retrieving user profile: {e}"),
    }
}

pub async fn get_userprofile_settings(api: &GarminApiClient) -> String {
    match api.api_json(PROFILE_SETTINGS, None).await {
        Ok(data) => client::render_or_friendly(
            &data,
            "No profile settings returned — Garmin may have moved this endpoint.",
        ),
        Err(e) => format!("Error retrieving profile settings: {e}"),
    }
}

pub async fn get_full_name(api: &GarminApiClient) -> String {
    let data = match api.api_json(SOCIAL_PROFILE, None).await {
        Ok(d) => d,
        Err(e) => return format!("Error retrieving full name: {e}"),
    };

    let full_name = data.get("fullName").and_then(|v| v.as_str());
    let display_name = data.get("displayName").and_then(|v| v.as_str());

    // Neither field present means an empty body, a Garmin error envelope, or a
    // payload reshape. Fall through rather than reporting `""` for both, which
    // reads as "this account has no name" to anything parsing the output.
    if full_name.is_none() && display_name.is_none() {
        return client::render_or_friendly(
            &data,
            "No name fields on socialProfile — Garmin may have changed the payload shape.",
        );
    }

    serde_json::to_string_pretty(&serde_json::json!({
        "full_name": full_name.unwrap_or(""),
        "display_name": display_name.unwrap_or(""),
    }))
    .unwrap_or_else(|e| format!("Error: {e}"))
}

pub async fn get_unit_system(api: &GarminApiClient) -> String {
    let data = match api.api_json(PROFILE_SETTINGS, None).await {
        Ok(d) => d,
        Err(e) => return format!("Error retrieving unit system: {e}"),
    };

    // Same reasoning as get_full_name: a missing field is reported, not rendered
    // as `{"measurement_system": null}`.
    let Some(unit) = data.get("measurementSystem").filter(|v| !v.is_null()) else {
        return client::render_or_friendly(
            &data,
            "No measurementSystem field on the profile settings — Garmin may have changed the payload shape.",
        );
    };

    serde_json::to_string_pretty(&serde_json::json!({ "measurement_system": unit }))
        .unwrap_or_else(|e| format!("Error: {e}"))
}
