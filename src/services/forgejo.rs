use crate::services::traits::RepoService;
use async_trait::async_trait;
use reqwest::{header, Client};
use serde_json::{json, Value};
use tracing::info;

pub struct ForgejoService {
    client: Client,
    base_url: String,
    owner: String,
    repo: String,
}

impl ForgejoService {
    pub fn new(
        token: String,
        base_url: String,
        project: String,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let mut headers = header::HeaderMap::new();
        headers.insert(
            "Authorization",
            header::HeaderValue::from_str(&format!("token {}", token))?,
        );

        let client = Client::builder().default_headers(headers).build()?;

        let base_url = base_url.trim_end_matches('/').to_string();
        let (owner, repo) = project
            .split_once('/')
            .ok_or("project must be in the format owner/repo")?;

        Ok(Self {
            client,
            base_url,
            owner: owner.to_string(),
            repo: repo.to_string(),
        })
    }
}

#[async_trait]
impl RepoService for ForgejoService {
    async fn create_pull_request(
        &self,
        title: &str,
        body: &str,
        head_branch: &str,
        base_branch: &str,
    ) -> Result<String, Box<dyn std::error::Error>> {
        let url = format!(
            "{}/api/v1/repos/{}/{}/pulls",
            self.base_url, self.owner, self.repo
        );

        let payload = json!({
            "title": title,
            "body": body,
            "head": head_branch,
            "base": base_branch
        });

        let res = self
            .client
            .post(&url)
            .json(&payload)
            .send()
            .await?
            .error_for_status()?;

        let json: Value = res.json().await?;
        let html_url = json
            .get("html_url")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();

        info!(url = %html_url, "Pull request created on Forgejo");
        Ok(html_url)
    }
}
