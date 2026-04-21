# BAGENTS: Autonomous AI Software Engineer

BAGENTS is an autonomous, multi-agent AI coding framework built in Rust. It functions as an automated software engineer that monitors your GitHub repositories for open issues, writes the necessary code to resolve them, verifies the changes, and automatically opens a Pull Request without human intervention.

Instead of operating as a simple chat assistant, BAGENTS uses a multi-agent architecture and semantic code understanding to safely modify large codebases.

---

## How It Works

The system operates in a continuous, multi-stage workflow:

### 1. Ingestion
Polls your target GitHub repository for the latest open issues.

### 2. Planning
A Team Lead agent analyzes the issue, reads the relevant repository structure, and generates an architectural plan, assigning the task to a specialized developer agent (Backend, Frontend, or DevOps).

### 3. Execution
The Developer agent writes the code. The system uses Tree-sitter for semantic chunking, allowing the agent to target and replace specific functions or structs without breaking the rest of the file.

### 4. Verification
The system runs your project's local test or build command (e.g., `cargo check`, `npm test`). If it fails, the error output is fed back to the agent for self-correction.

### 5. Review
A Code Reviewer agent analyzes the git diff. If the code is rejected, the workflow loops back to the developer with feedback.

### 6. Delivery
Once approved, the system commits the changes, pushes the branch via SSH, and opens a Pull Request on GitHub.

---

BAGENTS is LLM-agnostic and works with any OpenAI-compatible API, including Groq, local Ollama instances, and standard OpenAI models.

---

## How to Run

### Prerequisites

- Git configured with SSH access to your GitHub account (pushing must work without a password prompt)
- A GitHub Personal Access Token (Classic) with repository permissions
- An LLM API Key (e.g., Groq, OpenAI)
- Docker and Docker Compose (Recommended) **OR** Rust installed locally

---

## Installation

Clone the repository to your local machine:

```bash
git clone git@github.com:[YOUR_USERNAME]/bagents.git
cd bagents
```

## Configuration

Create a `.env` file in the root directory of the project. This file requires your LLM provider details, GitHub credentials, and the path to the target repository you want the AI to modify.

### Example `.env` configuration:

```env
LLM_API_KEY="your_llm_api_key_here"
LLM_API_URL="https://api.groq.com/openai/v1/chat/completions"
LLM_MODEL="llama-3.3-70b-versatile"
LLM_TEMPERATURE="0.2"

GITHUB_TOKEN="your_github_personal_access_token_here"
GITHUB_OWNER="target_github_username"
GITHUB_REPO="target_repository_name"

# The absolute path where the target repository is located
WORKSPACE_DIR="/workspace"

# The command used to verify the code before review
VERIFY_COMMAND="cargo check"
```

---

## Execution

### Recommended: Using Docker

Running BAGENTS via Docker ensures that the AI has a safe, isolated environment with the correct language dependencies.

1. Open the `docker-compose.yml` file  
2. Ensure `PROJECT_LANG` matches your repo (`rust`, `node`, `python`)  
3. Ensure volume mapping points to your target repository  

Start the agent:

```bash
docker compose up --build
```

The agent will immediately begin polling GitHub for issues, checking out branches, and writing code in the mapped volume.

---

### Alternative: Run Locally

1. Update `WORKSPACE_DIR` in your `.env` file to the absolute path of your repository (e.g., `/home/user/projects/target_repo`)  
2. Ensure required language toolchains (Rust, Node.js, etc.) are installed  
3. Start the system:

```bash
cargo run
```

## Usage

To trigger BAGENTS:

1. Go to your target repository on GitHub  
2. Create a new Issue  
3. Describe the task clearly and in detail  

**Example:**

```text
Implement a user authentication middleware in src/middleware.ts
```

BAGENTS will automatically detect the issue and begin processing it within a minute.
