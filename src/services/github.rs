use octocrab::{Octocrab, models::issues::Issue};
use std::env;

pub async fn fetch_open_issues() -> Result<Vec<Issue>, Box<dyn std::error::Error>> {
    let token = env::var("GITHUB_TOKEN").expect("GITHUB_TOKEN missing in .env");
    let owner = env::var("GITHUB_OWNER").expect("GITHUB_OWNER missing in .env");
    let repo = env::var("GITHUB_REPO").expect("GITHUB_REPO missing in .env");

    let octocrab = Octocrab::builder().personal_token(token).build()?;

    let issues_page = octocrab
        .issues(owner, repo)
        .list()
        .state(octocrab::params::State::Open)
        .send()
        .await?;

    let actual_issues: Vec<Issue> = issues_page
        .items
        .into_iter()
        .filter(|issue| issue.pull_request.is_none())
        .collect();

    Ok(actual_issues)
}

pub async fn create_pull_request(
    title: &str,
    body: &str,
    head_branch: &str,
    base_branch: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    let token = env::var("GITHUB_TOKEN").expect("GITHUB_TOKEN missing in .env");
    let owner = env::var("GITHUB_OWNER").expect("GITHUB_OWNER missing in .env");
    let repo = env::var("GITHUB_REPO").expect("GITHUB_REPO missing in .env");

    let octocrab = Octocrab::builder().personal_token(token).build()?;

    let pr = octocrab
        .pulls(owner, repo)
        .create(title, head_branch, base_branch)
        .body(body)
        .send()
        .await?;

    Ok(pr.html_url.map(|url| url.to_string()).unwrap_or_default())
}

pub async fn create_issue_comment(
    issue_number: u64,
    body: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let token = env::var("GITHUB_TOKEN").expect("GITHUB_TOKEN missing in .env");
    let owner = env::var("GITHUB_OWNER").expect("GITHUB_OWNER missing in .env");
    let repo = env::var("GITHUB_REPO").expect("GITHUB_REPO missing in .env");

    let octocrab = Octocrab::builder().personal_token(token).build()?;

    octocrab
        .issues(owner, repo)
        .create_comment(issue_number, body)
        .await?;

    Ok(())
}

pub async fn fetch_issue_comments(issue_number: u64) -> Result<String, Box<dyn std::error::Error>> {
    let token = env::var("GITHUB_TOKEN").expect("GITHUB_TOKEN missing in .env");
    let owner = env::var("GITHUB_OWNER").expect("GITHUB_OWNER missing in .env");
    let repo = env::var("GITHUB_REPO").expect("GITHUB_REPO missing in .env");

    let octocrab = Octocrab::builder().personal_token(token).build()?;

    let comments = octocrab
        .issues(owner, repo)
        .list_comments(issue_number)
        .send()
        .await?;

    let mut all_comments = String::new();
    for c in comments {
        if let Some(body) = c.body {
            all_comments.push_str(&format!("Comment by {}:\n{}\n\n", c.user.login, body));
        }
    }

    Ok(all_comments)
}
