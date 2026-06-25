use octocrab::Octocrab;
use tracing::{debug, info};
use async_trait::async_trait;
use crate::models::issue::Issue;
use crate::services::traits::{IssueTracker, RepoService};

pub struct GithubService {
    client: Octocrab,
    owner: String,
    repo: String,
}

impl GithubService {
    pub fn new(token: String, base_url: String, project: String) -> Result<Self, Box<dyn std::error::Error>> {
        let mut builder = Octocrab::builder().personal_token(token);
        
        if !base_url.is_empty() && base_url != "https://api.github.com" {
            builder = builder.base_uri(base_url)?;
        }
        
        let client = builder.build()?;
        
        let (owner, repo) = project.split_once('/').ok_or("project must be in the format owner/repo")?;

        Ok(Self {
            client,
            owner: owner.to_string(),
            repo: repo.to_string(),
        })
    }
}

#[async_trait]
impl IssueTracker for GithubService {
    async fn fetch_open_issues(&self) -> Result<Vec<Issue>, Box<dyn std::error::Error>> {
        let page = self
            .client
            .issues(&self.owner, &self.repo)
            .list()
            .state(octocrab::params::State::Open)
            .send()
            .await?;

        let issues: Vec<Issue> = page
            .items
            .into_iter()
            .filter(|i| i.pull_request.is_none())
            .map(|i| Issue {
                id: i.number.to_string(),
                title: i.title,
                body: i.body.unwrap_or_default(),
            })
            .collect();

        debug!(count = issues.len(), "Fetched open issues from GitHub");
        Ok(issues)
    }

    async fn create_issue_comment(
        &self,
        issue_id: &str,
        body: &str,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let issue_number: u64 = issue_id.parse()?;
        self.client
            .issues(&self.owner, &self.repo)
            .create_comment(issue_number, body)
            .await?;

        debug!(issue = issue_id, "Comment posted to GitHub");
        Ok(())
    }

    async fn fetch_issue_comments(
        &self,
        issue_id: &str,
    ) -> Result<String, Box<dyn std::error::Error>> {
        let issue_number: u64 = issue_id.parse()?;
        let comments = self
            .client
            .issues(&self.owner, &self.repo)
            .list_comments(issue_number)
            .send()
            .await?;

        let combined = comments
            .items
            .iter()
            .filter_map(|c| {
                c.body
                    .as_deref()
                    .map(|b| format!("Comment by {}:\n{}\n\n", c.user.login, b))
            })
            .collect::<String>();

        Ok(combined)
    }
}

#[async_trait]
impl RepoService for GithubService {
    async fn create_pull_request(
        &self,
        title: &str,
        body: &str,
        head_branch: &str,
        base_branch: &str,
    ) -> Result<String, Box<dyn std::error::Error>> {
        let pr = self
            .client
            .pulls(&self.owner, &self.repo)
            .create(title, head_branch, base_branch)
            .body(body)
            .send()
            .await?;

        let url = pr.html_url.map(|u| u.to_string()).unwrap_or_default();
        info!(url = %url, "Pull request created on GitHub");
        Ok(url)
    }
}
