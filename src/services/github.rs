use octocrab::{Octocrab, models::issues::Issue};
use tracing::{debug, info};

use crate::config::Config;

/// Build an authenticated Octocrab client from config.
fn build_client(config: &Config) -> Result<Octocrab, Box<dyn std::error::Error>> {
    Ok(Octocrab::builder()
        .personal_token(config.github_token.clone())
        .build()?)
}

/// Fetch all open issues for the configured repository, excluding pull requests.
pub async fn fetch_open_issues(config: &Config) -> Result<Vec<Issue>, Box<dyn std::error::Error>> {
    let client = build_client(config)?;

    let page = client
        .issues(&config.github_owner, &config.github_repo)
        .list()
        .state(octocrab::params::State::Open)
        .send()
        .await?;

    let issues: Vec<Issue> = page
        .items
        .into_iter()
        .filter(|i| i.pull_request.is_none())
        .collect();

    debug!(count = issues.len(), "Fetched open issues");
    Ok(issues)
}

/// Open a pull request from `head_branch` into `base_branch`.
/// Returns the HTML URL of the created PR.
pub async fn create_pull_request(
    config: &Config,
    title: &str,
    body: &str,
    head_branch: &str,
    base_branch: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    let client = build_client(config)?;

    let pr = client
        .pulls(&config.github_owner, &config.github_repo)
        .create(title, head_branch, base_branch)
        .body(body)
        .send()
        .await?;

    let url = pr.html_url.map(|u| u.to_string()).unwrap_or_default();
    info!(url = %url, "Pull request created");
    Ok(url)
}

/// Post a comment on the given issue number.
pub async fn create_issue_comment(
    config: &Config,
    issue_number: u64,
    body: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let client = build_client(config)?;

    client
        .issues(&config.github_owner, &config.github_repo)
        .create_comment(issue_number, body)
        .await?;

    debug!(issue = issue_number, "Comment posted");
    Ok(())
}

/// Fetch all comments on an issue and concatenate them into a single string.
pub async fn fetch_issue_comments(
    config: &Config,
    issue_number: u64,
) -> Result<String, Box<dyn std::error::Error>> {
    let client = build_client(config)?;

    let comments = client
        .issues(&config.github_owner, &config.github_repo)
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
