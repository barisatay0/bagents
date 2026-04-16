use log::{debug, error, info, warn};

// ... existing code ...

fn fetch_open_issues(...) {
    info!("Fetching open issues for repository: {}", repo_name);
    // ... original logic with error!(...) for failures ...
}

fn create_pull_request(...) {
    debug!("Creating PR from branch {} to {}", source, target);
    // ... enhanced error messages with status codes ...
}

// All println!/eprintln! replaced with log macros and contextual variables