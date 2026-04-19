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
    prompt: &str,
    max_retries: u32,
) -> Result<String, Box<dyn std::error::Error>> {
    let mut attempt = 0u32;
    loop {
        match self.client.ask(prompt).await {
            Ok(response) => return Ok(response),
            Err(e) => {
                if attempt >= max_retries {
                    return Err(e);
                }
                attempt += 1;
                tokio::time::sleep(std::time::Duration::from_secs(2u64.pow(attempt))).await;
            }
        }
    }
}