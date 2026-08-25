use crate::client::GarminApiClient;

pub async fn get_devices(api: &GarminApiClient) -> String {
    match api
        .api_json("/device-service/deviceregistration/devices", None)
        .await
    {
        Ok(data) => serde_json::to_string_pretty(&data).unwrap_or_else(|e| format!("Error: {e}")),
        Err(e) => format!("Error retrieving devices: {e}"),
    }
}

pub async fn get_device_last_used(api: &GarminApiClient) -> String {
    match api
        .api_json("/device-service/deviceservice/user-device/last-used", None)
        .await
    {
        Ok(data) => serde_json::to_string_pretty(&data).unwrap_or_else(|e| format!("Error: {e}")),
        Err(e) => format!("Error retrieving last used device: {e}"),
    }
}

pub async fn get_device_settings(api: &GarminApiClient, device_id: &str) -> String {
    let endpoint = format!(
        "/device-service/deviceservice/device-info/all/{}",
        device_id
    );
    match api.api_json(&endpoint, None).await {
        Ok(data) => serde_json::to_string_pretty(&data).unwrap_or_else(|e| format!("Error: {e}")),
        Err(e) => format!("Error retrieving settings for device {device_id}: {e}"),
    }
}

pub async fn get_primary_training_device(api: &GarminApiClient) -> String {
    match api
        .api_json(
            "/device-service/deviceservice/primary-training-device",
            None,
        )
        .await
    {
        Ok(data) => {
            crate::client::render_or_friendly(&data, "No primary training device on this account.")
        }
        Err(e) => format!("Error retrieving primary training device: {e}"),
    }
}

pub async fn get_device_solar_data(api: &GarminApiClient, device_id: &str) -> String {
    let endpoint = format!("/solar-service/solar/{}", device_id);
    match api.api_json(&endpoint, None).await {
        Ok(data) => serde_json::to_string_pretty(&data).unwrap_or_else(|e| format!("Error: {e}")),
        Err(e) => format!("Error retrieving solar data for device {device_id}: {e}"),
    }
}

pub async fn get_device_alarms(api: &GarminApiClient, device_id: &str) -> String {
    let endpoint = format!("/device-service/deviceservice/alarms/{}", device_id);
    match api.api_json(&endpoint, None).await {
        Ok(data) => serde_json::to_string_pretty(&data).unwrap_or_else(|e| format!("Error: {e}")),
        Err(e) => format!("Error retrieving alarms for device {device_id}: {e}"),
    }
}
