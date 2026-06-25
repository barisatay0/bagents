use crate::models::issue::Issue;
use async_trait::async_trait;

#[async_trait]
pub trait IssueTracker: Send + Sync {
    async fn fetch_open_issues(&self) -> Result<Vec<Issue>, Box<dyn std::error::Error>>;
    async fn create_issue_comment(&self, issue_id: &str, body: &str) -> Result<(), Box<dyn std::error::Error>>;
    async fn fetch_issue_comments(&self, issue_id: &str) -> Result<String, Box<dyn std::error::Error>>;
}

#[async_trait]
pub trait RepoService: Send + Sync {
    async fn create_pull_request(
        &self,
        title: &str,
        body: &str,
        head_branch: &str,
        base_branch: &str,
    ) -> Result<String, Box<dyn std::error::Error>>;
}
