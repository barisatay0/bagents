use reqwest::{Client, StatusCode};
use serde_json::{json, Value};
use std::env;
use tokio::time::{sleep, Duration};

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

    let mut retries = 0;
    let max_retries = 5;

    loop {
        let res = client
            .post(&url)
            .bearer_auth(&api_key)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(payload.clone())
            .send()
            .await?;

        let status = res.status();
        let res_text = res.text().await?;

        if status == StatusCode::TOO_MANY_REQUESTS
            || res_text.to_lowercase().contains("rate limit")
            || res_text.to_lowercase().contains("try again in")
        {
            if retries >= max_retries {
                return Err(format!(
                    "LLM API Error (Rate Limit hit {} times): {}",
                    max_retries, res_text
                )
                .into());
            }

            println!(
                "API Rate Limit hit! Factory is taking a 15-second coffee break... ({}/{})",
                retries + 1,
                max_retries
            );
            sleep(Duration::from_secs(15)).await;
            retries += 1;
            continue;
        }

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

        return Ok(output(&raw_content));
    }
}
