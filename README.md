# BAGENTS

[![Rust](https://img.shields.io/badge/Rust-1.75+-orange.svg)](https://www.rust-lang.org/)
[![AI Powered](https://img.shields.io/badge/AI-Powered-purple.svg)]()

**BAGENTS** is an autonomous, multi-agent AI software factory built in Rust. It monitors your GitHub repositories for open issues, assigns them to specialized AI agents (Backend, Frontend, DevOps), writes the code, reviews it, and automatically opens a Pull Request—all without human intervention.

## ✨ Key Features

* **🕵️‍♂️ Autonomous Issue Processing:** Automatically fetches open issues from your GitHub repository using the GitHub API (`octocrab`).
* **🧠 Multi-Agent Orchestration:**
    * **Team Leader:** Analyzes the issue and generates an architectural plan.
    * **Developer Agents:** Specialized agents (`backend_dev`, `frontend_dev`, `devops_dev`) write the code based on the architectural plan.
    * **Code Reviewer:** A strict, detail-oriented agent that reviews the generated code.
* **🔄 Self-Correcting Feedback Loop:** If the Code Reviewer rejects the code, it leaves a comment on the GitHub issue, and the Developer agent automatically attempts to fix the code (up to 3 attempts).
* **🌐 Cross-Repository Support:** BAGENTS can run in its own directory while targeting and modifying code in a completely different workspace (`WORKSPACE_DIR`).
* **📦 Automated Git Operations:** Automatically creates branches, commits changes, and pushes to the remote repository.
* **🚀 Automated Pull Requests:** Opens a fully formatted Pull Request once the code passes the AI review.
* **LLM Agnostic:** Works with OpenAI-compatible APIs (tested perfectly with Groq's `llama-3.3-70b-versatile` and local Ollama models).

## 🛠️ System Architecture

The factory operates in a continuous, multi-stage workflow:

1.  **Ingestion:** Reads the latest open issue from the target GitHub repo.
2.  **Planning:** The `team_lead` LLM outputs a JSON plan and assigns a worker.
3.  **Execution:** The assigned worker (e.g., `backend_dev`) writes the code and modifies the file system in the target `WORKSPACE_DIR`.
4.  **Review:** The `reviewer` LLM checks the `git diff`.
5.  **Iteration (Optional):** If rejected, the reviewer posts feedback to the GitHub issue, and the workflow loops back to Execution.
6.  **Delivery:** If approved, the system pushes the branch via SSH and opens a PR via the GitHub API.

## 🚀 Getting Started

### 1. Prerequisites
* [Rust](https://www.rust-lang.org/tools/install) installed on your machine.
* Git configured with SSH access to your GitHub account (`git push` must work without a password prompt).
* A GitHub Personal Access Token (Classic) with `repo` permissions.
* An LLM API Key (e.g., Groq API key or a running local Ollama instance).

### 2. Installation
Clone the repository:
```bash
git clone git@github.com:[YOUR_USERNAME]/bagents.git
cd bagents
```

### 3. Configuration
Create a .env file in the root directory of the project. You must configure both your LLM provider and your GitHub details.

#### --- LLM CONFIGURATION (Example using Groq) ---
LLM_API_KEY="gsk_your_groq_api_key_here"
LLM_API_URL="[https://api.groq.com/openai/v1/chat/completions](https://api.groq.com/openai/v1/chat/completions)"
LLM_MODEL="llama-3.3-70b-versatile"
LLM_TEMPERATURE="0.2"

#### --- GITHUB CONFIGURATION ---
GITHUB_TOKEN="ghp_your_github_personal_access_token_here"
GITHUB_OWNER="your_github_username"
GITHUB_REPO="target_repository_name"

#### --- WORKSPACE CONFIGURATION ---
#### The absolute path where the target repository is located on your local machine.
#### Bagents will perform git operations and file modifications inside this directory.
WORKSPACE_DIR="/home/your_username/projects/target_repository_name"

# Usage
Go to your target GitHub repository and create a new Issue. Be descriptive (e.g., "Create a fast Fibonacci function in Rust in src/math.rs").

Run the BAGENTS orchestrator from your bagents directory:

```bash
cargo run
```

Watch the terminal as the agents analyze the issue, write the code, review each other, and finally provide you with a completed Pull Request!

## Project Structure
src/orchestrator.rs: The core engine managing the agent workflow and feedback loops.

src/clients/llm_client.rs: Handles communication with OpenAI-compatible APIs.

src/services/: Contains integrations for github, git_local, and file_system.

config/: Contains the Markdown prompt files that define the persona and rules for each AI agent (team_lead.md, backend_dev.md, reviewer.md, etc.).

## Important Notes
JSON Enforcement: The prompts in the config/ directory heavily enforce valid JSON output. Modifying these prompts requires strict attention to JSON syntax instructions to prevent parsing panics.

Rate Limits: Be mindful of your LLM API rate limits, especially when the feedback loop is triggered multiple times.

## Contributing
Contributions are welcome! If you want to add new agent types (e.g., qa_engineer or security_auditor) or implement RAG to allow agents to read the entire codebase, feel free to open a PR.

