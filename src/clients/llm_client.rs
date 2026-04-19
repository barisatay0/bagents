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
pub async fn ask(&self, prompt: &str) -> Result<String, Box<dyn std::error::Error>> {
    let max_retries = self.config.llm_max_retries;
    let mut retries = 0u32;
    let mut last_error = None;

    while retries < max_retries {
        match self.client.ask(prompt).await {
            Ok(response) => return Ok(response),
            Err(e) => {
                retries += 1;
                last_error = Some(e);
                if retries >= max_retries {
                    break;
                }
                // Exponential backoff
                let delay = std::time::Duration::from_secs(2u64.pow(retries));
                tokio::time::sleep(delay).await;
            }
        }
    }

    Err(last_error.unwrap_or_else(|| "Max retries exceeded".into()))
}