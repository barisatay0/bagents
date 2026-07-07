use crate::models::issue::Issue;
use crate::services::traits::{IssueTracker, RepoService};
use async_trait::async_trait;
use reqwest::{header, Client};
use serde_json::{json, Value};
use tracing::{debug, info};

pub struct GitlabService {
    client: Client,
    base_url: String,
    project_id: String,
}

impl GitlabService {
    pub fn new(
        token: String,
        base_url: String,
        project: String,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let mut headers = header::HeaderMap::new();
        headers.insert("PRIVATE-TOKEN", header::HeaderValue::from_str(&token)?);

        let client = Client::builder().default_headers(headers).build()?;

        let base_url = base_url.trim_end_matches('/').to_string();
        // GitLab API requires URL-encoded project path (e.g. "owner%2Frepo")
        let project_id = project.replace("/", "%2F");

        Ok(Self {
            client,
            base_url,
            project_id,
        })
    }
}

#[async_trait]
impl IssueTracker for GitlabService {
    async fn fetch_open_issues(&self) -> Result<Vec<Issue>, Box<dyn std::error::Error>> {
        let url = format!(
            "{}/api/v4/projects/{}/issues?state=opened",
            self.base_url, self.project_id
        );

        let res = self.client.get(&url).send().await?.error_for_status()?;
        let json: Value = res.json().await?;

        let mut issues = Vec::new();
        if let Value::Array(arr) = json {
            for item in arr {
                if let (Some(iid), Some(title)) = (
                    item.get("iid").and_then(|v| v.as_u64()),
                    item.get("title").and_then(|v| v.as_str()),
                ) {
                    let desc = item.get("description")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    issues.push(Issue {
                        id: iid.to_string(),
                        title: title.to_string(),
                        body: desc.to_string(),
                    });
                }
            }
        }

        debug!(count = issues.len(), "Fetched open issues from GitLab");
        Ok(issues)
    }

    async fn create_issue_comment(
        &self,
        issue_id: &str,
        body: &str,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let url = format!(
            "{}/api/v4/projects/{}/issues/{}/notes",
            self.base_url, self.project_id, issue_id
        );

        self.client
            .post(&url)
            .json(&json!({ "body": body }))
            .send()
            .await?
            .error_for_status()?;

        debug!(issue = issue_id, "Comment posted to GitLab");
        Ok(())
    }

    async fn fetch_issue_comments(
        &self,
        issue_id: &str,
    ) -> Result<String, Box<dyn std::error::Error>> {
        let url = format!(
            "{}/api/v4/projects/{}/issues/{}/notes",
            self.base_url, self.project_id, issue_id
        );

        let res = self.client.get(&url).send().await?.error_for_status()?;
        let json: Value = res.json().await?;

        let mut combined = String::new();
        if let Value::Array(arr) = json {
            // GitLab returns notes newest first by default in v4, let's just append them.
            // A production app might sort them by created_at.
            for item in arr.iter().rev() {
                if item.get("system").and_then(|v| v.as_bool()).unwrap_or(false) {
                    continue; // Skip system notes (e.g. "User changed status to closed")
                }
                if let (Some(body), Some(author)) = (
                    item.get("body").and_then(|v| v.as_str()),
                    item.get("author").and_then(|a| a.get("username")).and_then(|u| u.as_str()),
                ) {
                    combined.push_str(&format!("Comment by {}:\n{}\n\n", author, body));
                }
            }
        }

        Ok(combined)
    }
}

#[async_trait]
impl RepoService for GitlabService {
    async fn create_pull_request(
        &self,
        title: &str,
        body: &str,
        head_branch: &str,
        base_branch: &str,
    ) -> Result<String, Box<dyn std::error::Error>> {
        let url = format!(
            "{}/api/v4/projects/{}/merge_requests",
            self.base_url, self.project_id
        );

        let payload = json!({
            "source_branch": head_branch,
            "target_branch": base_branch,
            "title": title,
            "description": body
        });

        let res = self.client
            .post(&url)
            .json(&payload)
            .send()
            .await?
            .error_for_status()?;

        let json: Value = res.json().await?;
        let web_url = json.get("web_url").and_then(|v| v.as_str()).unwrap_or_default().to_string();

        info!(url = %web_url, "Merge request created on GitLab");
        Ok(web_url)
    }
}
