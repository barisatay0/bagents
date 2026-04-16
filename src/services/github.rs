use log::{info, debug, warn, error};
use reqwest::Client;
use std::env;

pub struct GitHubClient {
    client: Client,
    token: String,
}

impl GitHubClient {
    pub fn build_client() -> Self {
        let token = env::var("GITHUB_TOKEN").expect("GITHUB_TOKEN must be set");
        let client = Client::new();
        info!("GitHub client initialized with token: {}", token);

        GitHubClient { client, token }
    }

    pub async fn fetch_open_issues(&self, owner: &str, repo: &str) -> Result<Vec<Issue>, reqwest::Error> {
        let url = format!("https://api.github.com/repos/{}/{}/issues?state=open", owner, repo);
        debug!("Fetching open issues from: {}", url);

        let response = self.client.get(&url)
            .header("Authorization", format!("token {}", self.token))
            .header("User-Agent", "bagents")
            .send()
            .await?
            .json::<Vec<Issue>>()
            .await;

        match response {
            Ok(issues) => {
                info!("Fetched {} open issues from {}/{}/{}", issues.len(), owner, repo);
                Ok(issues)
            }
            Err(e) => {
                error!("Failed to fetch open issues: {}", e);
                Err(e)
            }
        }
    }

    pub async fn create_pull_request(&self, owner: &str, repo: &str, title: &str, head: &str, base: &str) -> Result<PullRequest, reqwest::Error> {
        let url = format!("https://api.github.com/repos/{}/{}/pulls", owner, repo);
        debug!("Creating pull request from {} to {} in {}/{}/{}", head, base, owner, repo);

        let body = json!({
            "title": title,
            "head": head,
            "base": base,
        });

        let response = self.client.post(&url)
            .header("Authorization", format!("token {}", self.token))
            .header("User-Agent", "bagents")
            .json(&body)
            .send()
            .await?
            .json::<PullRequest>()
            .await;

        match response {
            Ok(pr) => {
                info!("Created pull request #{}: {}", pr.number, pr.title);
                Ok(pr)
            }
            Err(e) => {
                error!("Failed to create pull request: {}", e);
                Err(e)
            }
        }
    }

    pub async fn create_issue_comment(&self, owner: &str, repo: &str, issue_number: u64, body: &str) -> Result<Comment, reqwest::Error> {
        let url = format!("https://api.github.com/repos/{}/{}/issues/{}/comments", owner, repo, issue_number);
        debug!("Creating comment on issue #{} in {}/{}/{}", issue_number, owner, repo);

        let body = json!({
            "body": body,
        });

        let response = self.client.post(&url)
            .header("Authorization", format!("token {}", self.token))
            .header("User-Agent", "bagents")
            .json(&body)
            .send()
            .await?
            .json::<Comment>()
            .await;

        match response {
            Ok(comment) => {
                info!("Created comment #{} on issue #{}", comment.id, issue_number);
                Ok(comment)
            }
            Err(e) => {
                error!("Failed to create issue comment: {}", e);
                Err(e)
            }
        }
    }

    pub async fn fetch_issue_comments(&self, owner: &str, repo: &str, issue_number: u64) -> Result<Vec<Comment>, reqwest::Error> {
        let url = format!("https://api.github.com/repos/{}/{}/issues/{}/comments", owner, repo, issue_number);
        debug!("Fetching comments for issue #{} in {}/{}/{}", issue_number, owner, repo);

        let response = self.client.get(&url)
            .header("Authorization", format!("token {}", self.token))
            .header("User-Agent", "bagents")
            .send()
            .await?
            .json::<Vec<Comment>>()
            .await;

        match response {
            Ok(comments) => {
                info!("Fetched {} comments for issue #{}", comments.len(), issue_number);
                Ok(comments)
            }
            Err(e) => {
                error!("Failed to fetch issue comments: {}", e);
                Err(e)
            }
        }
    }
}

#[derive(Debug, Deserialize)]
struct Issue {
    number: u64,
    title: String,
}

#[derive(Debug, Deserialize)]
struct PullRequest {
    number: u64,
    title: String,
}

#[derive(Debug, Deserialize)]
struct Comment {
    id: u64,
    body: String,
}