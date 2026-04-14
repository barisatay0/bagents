use reqwest::{Client, StatusCode};
use serde_json::{Value, json};
use tokio::time::{Duration, sleep};
use tracing::{debug, error, warn};

use crate::config::Config;
use crate::helpers::helper_output::output;

/// Send a prompt to the configured LLM and return the cleaned response string.
///
/// Retries automatically on rate-limit responses (HTTP 429 or body containing
/// "rate limit"/"try again in") with a 15-second base sleep plus random jitter
/// to avoid thundering-herd against the API. Hard cap of 5 retries.
pub async fn ask(
    config: &Config,
    system_prompt: &str,
    user_prompt: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    let client = Client::new();

    let mut body = json!({
        "model": config.llm_model,
        "messages": [
            {"role": "system", "content": system_prompt},
            {"role": "user",   "content": user_prompt}
        ],
        "temperature": config.llm_temperature
    });

    match config.json_mode.to_lowercase().as_str() {
        "openai" | "groq" | "true" => {
            body["response_format"] = json!({"type": "json_object"});
        }
        "ollama" => {
            body["format"] = json!("json");
        }
        _ => {}
    }

    let payload = serde_json::to_string(&body)?;
    debug!(model = %config.llm_model, "Sending LLM request");

    let mut retries = 0u32;
    let max_retries = 5u32;

    loop {
        let res = client
            .post(&config.llm_api_url)
            .bearer_auth(&config.llm_api_key)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(payload.clone())
            .send()
            .await?;

        let status = res.status();
        let res_text = res.text().await?;

        if status == StatusCode::TOO_MANY_REQUESTS
            || (!status.is_success() && res_text.to_lowercase().contains("rate limit"))
            || (!status.is_success() && res_text.to_lowercase().contains("try again in"))
        {
            if retries >= max_retries {
                return Err(format!(
                    "LLM API rate limit hit {} times — giving up: {}",
                    max_retries, res_text
                )
                .into());
            }

            // Jitter: 15s base + up to 5s random to spread retries
            let jitter = (std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .subsec_millis()
                % 5000) as u64;
            let wait = 15 + jitter / 1000;

            warn!(
                attempt = retries + 1,
                max_retries,
                wait_secs = wait,
                "Rate limit hit — backing off"
            );
            sleep(Duration::from_secs(wait)).await;
            retries += 1;
            continue;
        }

        let res_json: Value = serde_json::from_str(&res_text)?;

        if res_json["error"].is_object() {
            let err_msg = res_json["error"]["message"]
                .as_str()
                .unwrap_or("Unknown API error");
            error!(err = err_msg, "LLM API returned an error object");
            return Err(format!("LLM API Error: {}", err_msg).into());
        }

        let raw_content = res_json["choices"][0]["message"]["content"]
            .as_str()
            .unwrap_or("")
            .to_string();

        debug!("LLM response received ({} chars)", raw_content.len());
        return Ok(output(&raw_content));
    }
}
