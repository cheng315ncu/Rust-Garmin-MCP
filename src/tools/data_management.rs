use crate::client::GarminApiClient;

pub async fn add_hydration_data(api: &GarminApiClient, date: &str, value_in_ml: f64) -> String {
    let body = serde_json::json!({
        "calendarDate": date,
        "valueInML": value_in_ml,
    });
    match api
        .api_post_json("/usersummary-service/usersummary/hydration/daily", body)
        .await
    {
        Ok(data) => serde_json::to_string_pretty(&data).unwrap_or_else(|e| format!("Error: {e}")),
        Err(e) => format!("Error adding hydration data for {date}: {e}"),
    }
}

pub async fn set_blood_pressure(
    api: &GarminApiClient,
    date: &str,
    systolic: u32,
    diastolic: u32,
    pulse: u32,
    notes: Option<&str>,
) -> String {
    let mut body = serde_json::json!({
        "measurementTimestampLocal": date,
        "systolic": systolic,
        "diastolic": diastolic,
        "pulse": pulse,
    });
    if let Some(n) = notes {
        body["notes"] = serde_json::Value::String(n.to_string());
    }
    match api
        .api_post_json("/biometric-service/blood-pressure", body)
        .await
    {
        Ok(data) => serde_json::to_string_pretty(&data).unwrap_or_else(|e| format!("Error: {e}")),
        Err(e) => format!("Error recording blood pressure for {date}: {e}"),
    }
}

pub async fn add_body_composition(
    api: &GarminApiClient,
    date: &str,
    weight_kg: f64,
    percent_fat: Option<f64>,
    muscle_mass_grams: Option<u32>,
    bone_mass_grams: Option<u32>,
) -> String {
    // Garmin expects weight in grams
    let weight_grams = (weight_kg * 1000.0) as u32;
    let mut body = serde_json::json!({
        "date": date,
        "weight": weight_grams,
    });
    if let Some(pf) = percent_fat {
        body["percentFat"] = serde_json::json!(pf);
    }
    if let Some(mm) = muscle_mass_grams {
        body["muscleMass"] = serde_json::json!(mm);
    }
    if let Some(bm) = bone_mass_grams {
        body["boneMass"] = serde_json::json!(bm);
    }
    match api.api_post_json("/weight-service/weight", body).await {
        Ok(data) => serde_json::to_string_pretty(&data).unwrap_or_else(|e| format!("Error: {e}")),
        Err(e) => format!("Error adding body composition for {date}: {e}"),
    }
}
