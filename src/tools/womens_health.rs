use std::collections::HashMap;

use crate::client::GarminApiClient;

pub async fn get_menstrual_data_for_date(api: &GarminApiClient, date: &str) -> String {
    let endpoint = format!("/menstrual-service/menstrual/dayview/{}", date);
    match api.api_json(&endpoint, None).await {
        Ok(data) => crate::client::render_or_friendly(
            &data,
            &format!("No menstrual data for {date} — menstrual tracking may not be enabled on this account."),
        ),
        Err(e) => format!("Error retrieving menstrual data for {date}: {e}"),
    }
}

pub async fn get_menstrual_calendar_data(
    api: &GarminApiClient,
    start_date: &str,
    end_date: &str,
) -> String {
    let params = HashMap::from([
        ("from".to_string(), start_date.to_string()),
        ("to".to_string(), end_date.to_string()),
    ]);
    match api
        .api_json("/menstrual-service/menstrual/calendar", Some(params))
        .await
    {
        Ok(data) => crate::client::render_or_friendly(
            &data,
            &format!("No menstrual calendar data between {start_date} and {end_date}."),
        ),
        Err(e) => {
            format!("Error retrieving menstrual calendar from {start_date} to {end_date}: {e}")
        }
    }
}

pub async fn get_pregnancy_summary(api: &GarminApiClient) -> String {
    match api
        .api_json("/pregnancy-service/pregnancy/snapshot", None)
        .await
    {
        Ok(data) => crate::client::render_or_friendly(
            &data,
            "No pregnancy data — pregnancy tracking is not active on this account.",
        ),
        Err(e) => format!("Error retrieving pregnancy summary: {e}"),
    }
}
