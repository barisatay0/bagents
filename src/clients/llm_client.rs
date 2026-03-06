use reqwest::Client;
use serde_json::{Value, json};
use std::env;

use crate::helpers::helper_output::output;

pub async fn ask(
    system_prompt: &str,
    user_prompt: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    let api_key = env::var("LLM_API_KEY").unwrap_or_default();
    let url = env::var("LLM_API_URL").expect("LLM_API_URL does not exist in .env file!");
    let model = env::var("LLM_MODEL").expect("LLM_MODEL does not exist in .env file!");

    let temp_str = env::var("LLM_TEMPERATURE").unwrap_or_else(|_| "0.2".to_string());
    let temperature: f32 = temp_str.parse().unwrap_or(0.2);

    let client = Client::new();

    let body = json!({
        "model": model,
        "messages": [
            {"role": "system", "content": system_prompt},
            {"role": "user", "content": user_prompt}
        ],
        "temperature": temperature
    });

    let payload = serde_json::to_string(&body)?;

    let res = client
        .post(&url)
        .bearer_auth(api_key)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .body(payload)
        .send()
        .await?;

    let res_text = res.text().await?;
    let res_json: Value = serde_json::from_str(&res_text)?;

    if res_json["error"].is_object() {
        let err_msg = res_json["error"]["message"]
            .as_str()
            .unwrap_or("Unknown API Error");
        return Err(format!("LLM API Error: {}", err_msg).into());
    }

    let raw_content = res_json["choices"][0]["message"]["content"]
        .as_str()
        .unwrap_or("")
        .to_string();

    Ok(output(&raw_content))
}
