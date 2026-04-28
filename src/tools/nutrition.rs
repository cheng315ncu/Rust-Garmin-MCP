use std::collections::HashMap;

use crate::client::GarminApiClient;

pub async fn get_nutrition_daily_food_log(api: &GarminApiClient, date: &str) -> String {
    let params = HashMap::from([("date".to_string(), date.to_string())]);
    match api
        .api_json(
            "/nutrition-service/nutrition/foodLog",
            Some(params),
        )
        .await
    {
        Ok(data) => crate::client::render_or_friendly(
            &data,
            &format!("No nutrition log for {date} — nutrition tracking may not be enabled on this account."),
        ),
        Err(e) => format!("Error retrieving food log for {date}: {e}"),
    }
}

pub async fn get_nutrition_daily_settings(api: &GarminApiClient) -> String {
    match api
        .api_json("/nutrition-service/nutrition/settings", None)
        .await
    {
        Ok(data) => crate::client::render_or_friendly(
            &data,
            "No nutrition settings — nutrition tracking may not be enabled on this account.",
        ),
        Err(e) => format!("Error retrieving nutrition settings: {e}"),
    }
}

pub async fn get_custom_foods(api: &GarminApiClient) -> String {
    match api
        .api_json("/nutrition-service/nutrition/foods/user", None)
        .await
    {
        Ok(data) => crate::client::render_or_friendly(
            &data,
            "No custom foods saved.",
        ),
        Err(e) => format!("Error retrieving custom foods: {e}"),
    }
}
