use std::fs;

/// All agent system prompts loaded from `config/` at startup.
/// Avoids repeated disk reads inside the hot processing loop.
#[derive(Debug, Clone)]
pub struct Prompts {
    pub team_lead: String,
    pub backend_dev: String,
    pub frontend_dev: String,
    pub devops_dev: String,
    pub reviewer: String,
}

impl Prompts {
    /// Read all prompt files from the `config/` directory.
    /// Returns an error if any required file is missing or unreadable.
    pub fn load() -> Result<Self, String> {
        let read = |name: &str| -> Result<String, String> {
            fs::read_to_string(format!("config/{}.md", name))
                .map_err(|e| format!("Could not read config/{}.md: {}", name, e))
        };

        Ok(Self {
            team_lead: read("team_lead")?,
            backend_dev: read("backend_dev")?,
            frontend_dev: read("frontend_dev")?,
            devops_dev: read("devops_dev")?,
            reviewer: read("reviewer")?,
        })
    }

    /// Return the correct developer prompt for the agent name returned by the team lead.
    /// Falls back to `backend_dev` for unknown agent names.
    pub fn for_agent(&self, agent: &str) -> &str {
        match agent {
            "frontend_dev" => &self.frontend_dev,
            "devops_dev" => &self.devops_dev,
            _ => &self.backend_dev,
        }
    }
}
