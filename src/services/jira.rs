use crate::models::issue::Issue;
use crate::services::traits::IssueTracker;
use async_trait::async_trait;
use base64::Engine;
use reqwest::{header, Client};
use serde_json::{json, Value};
use tracing::debug;

pub struct JiraService {
    client: Client,
    base_url: String,
    project_key: String,
}

impl JiraService {
    pub fn new(
        token: String,
        username: Option<String>,
        base_url: String,
        project: String,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let mut headers = header::HeaderMap::new();

        if let Some(user) = username {
            let auth_str = format!("{}:{}", user, token);
            let b64 = base64::engine::general_purpose::STANDARD.encode(auth_str.as_bytes());
            headers.insert(
                "Authorization",
                header::HeaderValue::from_str(&format!("Basic {}", b64))?,
            );
        } else {
            headers.insert(
                "Authorization",
                header::HeaderValue::from_str(&format!("Bearer {}", token))?,
            );
        }

        headers.insert("Accept", header::HeaderValue::from_static("application/json"));

        let client = Client::builder().default_headers(headers).build()?;
        let base_url = base_url.trim_end_matches('/').to_string();

        Ok(Self {
            client,
            base_url,
            project_key: project,
        })
    }
}

#[async_trait]
impl IssueTracker for JiraService {
    async fn fetch_open_issues(&self) -> Result<Vec<Issue>, Box<dyn std::error::Error>> {
        let jql = format!("project=\"{}\" AND statusCategory!=\"Done\"", self.project_key);
        let url = format!("{}/rest/api/2/search", self.base_url);

        let res = self
            .client
            .get(&url)
            .query(&[("jql", &jql)])
            .send()
            .await?
            .error_for_status()?;

        let json: Value = res.json().await?;

        let mut issues = Vec::new();
        if let Some(arr) = json.get("issues").and_then(|i| i.as_array()) {
            for item in arr {
                if let (Some(key), Some(fields)) = (
                    item.get("key").and_then(|v| v.as_str()),
                    item.get("fields").and_then(|v| v.as_object()),
                ) {
                    let title = fields
                        .get("summary")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let body = fields
                        .get("description")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();

                    issues.push(Issue {
                        id: key.to_string(),
                        title,
                        body,
                    });
                }
            }
        }

        debug!(count = issues.len(), "Fetched open issues from Jira");
        Ok(issues)
    }

    async fn create_issue_comment(
        &self,
        issue_id: &str,
        body: &str,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let url = format!("{}/rest/api/2/issue/{}/comment", self.base_url, issue_id);

        self.client
            .post(&url)
            .json(&json!({ "body": body }))
            .send()
            .await?
            .error_for_status()?;

        debug!(issue = issue_id, "Comment posted to Jira");
        Ok(())
    }

    async fn fetch_issue_comments(
        &self,
        issue_id: &str,
    ) -> Result<String, Box<dyn std::error::Error>> {
        let url = format!("{}/rest/api/2/issue/{}/comment", self.base_url, issue_id);

        let res = self.client.get(&url).send().await?.error_for_status()?;
        let json: Value = res.json().await?;

        let mut combined = String::new();
        if let Some(arr) = json.get("comments").and_then(|c| c.as_array()) {
            for item in arr {
                if let (Some(body), Some(author)) = (
                    item.get("body").and_then(|v| v.as_str()),
                    item.get("author")
                        .and_then(|a| a.get("displayName"))
                        .and_then(|n| n.as_str()),
                ) {
                    combined.push_str(&format!("Comment by {}:\n{}\n\n", author, body));
                }
            }
        }

        Ok(combined)
    }
}
