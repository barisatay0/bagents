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
    &self,
    config: &Config,
    prompt: &str,
    model: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    let max_retries = config.llm_max_retries;
    let mut attempt = 0u32;
    let mut last_error = None;

    loop {
        attempt += 1;
        let response = self
            .client
            .chat()
            .with_model(model)
            .with_message(prompt)
            .await;

        match response {
            Ok(res) => {
                return Ok(res);
            }
            Err(e) => {
                if attempt >= max_retries {
                    return Err(e.into());
                }
                last_error = Some(e);
                tokio::time::sleep(Duration::from_secs(2u64.pow(attempt))).await;
            }
        }
    }
}