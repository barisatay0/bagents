use octocrab::{Octocrab, models::issues::Issue};
use std::env;

pub async fn fetch_open_issues() -> Result<Vec<Issue>, Box<dyn std::error::Error>> {
    let token = env::var("GITHUB_TOKEN").expect("GITHUB_TOKEN missing in .env");
    let owner = env::var("GITHUB_OWNER").expect("GITHUB_OWNER missing in .env");
    let repo = env::var("GITHUB_REPO").expect("GITHUB_REPO missing in .env");

    let octocrab = Octocrab::builder().personal_token(token).build()?;

    // Fetch open issues from the repository
    let issues = octocrab
        .issues(owner, repo)
        .list()
        .state(octocrab::params::State::Open)
        .send()
        .await?;

    Ok(issues.items)
}
