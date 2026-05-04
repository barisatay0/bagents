# BAGENTS: AI Software Team

BAGENTS is an autonomous, multi-agent AI coding framework built in Rust. It functions as an automated software engineer that monitors your GitHub repositories for open issues, writes the necessary code to resolve them, verifies the changes, and automatically opens a Pull Request — without human intervention.

Instead of operating as a simple chat assistant, BAGENTS uses a multi-agent architecture and semantic code understanding to safely modify large codebases.

---

## How It Works

The system operates in a continuous, multi-stage workflow:

### 1. Ingestion
Polls your target GitHub repository for the latest open issues.

### 2. Planning
A Team Lead agent analyzes the issue, reads the relevant repository structure, and generates an architectural plan — assigning the task to a specialized developer agent (Backend, Frontend, or DevOps).

### 3. Execution
The Developer agent writes the code. The system uses Tree-sitter for semantic chunking, allowing the agent to target and replace specific functions or structs without breaking the rest of the file.

### 4. Verification
The system runs your project's local test or build command (e.g. `cargo check`, `npm test`). If it fails, the error output is fed back to the agent for self-correction.

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
- An LLM API Key (e.g. Groq, OpenAI)
- Docker and Docker Compose (Recommended) **OR** Rust installed locally

---

## Installation

Clone the repository to your local machine:

```bash
git clone git@github.com:[YOUR_USERNAME]/bagents.git
cd bagents
```

## Configuration

Create a `.env` file in the root directory of the project.

### Example `.env` configuration

```env
# ── LLM Provider ─────────────────────────────────────────────────────────────
LLM_API_KEY="your_llm_api_key_here"
LLM_API_URL="https://api.groq.com/openai/v1/chat/completions"
LLM_MODEL="llama-3.3-70b-versatile"
LLM_TEMPERATURE="0.2"

# JSON mode: "openai" (default), "groq", "ollama", or "none"
LLM_JSON_MODE="openai"

# Max output tokens for planning/review requests (default: 4096)
LLM_MAX_TOKENS="4096"

# Max output tokens for developer requests — these produce full file content
# and need more room. Increase if agents are truncating. (default: 8192)
LLM_MAX_TOKENS_LARGE="8192"

# ── GitHub ────────────────────────────────────────────────────────────────────
GITHUB_TOKEN="your_github_personal_access_token_here"
GITHUB_OWNER="target_github_username"
GITHUB_REPO="target_repository_name"

# ── Workspace ─────────────────────────────────────────────────────────────────
# Absolute path where the target repository is located
WORKSPACE_DIR="/workspace"

# Command to verify the code before review (leave blank to skip)
VERIFY_COMMAND="cargo check"
```

### Tuning for truncation issues

If developer agents are cutting responses short or producing incomplete code, increase the token limits:

```env
LLM_MAX_TOKENS_LARGE="16384"
```

Note: your provider must support the requested output token count. Check your plan limits.

---

## Execution

### Recommended: Using Docker

Running BAGENTS via Docker ensures that the AI has a safe, isolated environment with the correct language dependencies.

1. Open the `docker-compose.yml` file
2. Ensure `PROJECT_LANG` matches your repo (`rust`, `node`, `python`)
3. Ensure the volume mapping points to your target repository

Start the agent:

```bash
docker compose up --build
```

The agent will immediately begin polling GitHub for issues, checking out branches, and writing code in the mapped volume.

---

### Alternative: Run Locally

1. Update `WORKSPACE_DIR` in your `.env` file to the absolute path of your repository
2. Ensure the required language toolchains are installed (Rust, Node.js, etc.)
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

```
Implement a user authentication middleware in src/middleware.ts

The middleware should:
- Read the Authorization header
- Validate the JWT token
- Attach the decoded user object to the request context
- Return 401 if the token is missing or invalid
```

The more specific your issue description, the better the output. Include file paths, function names, and expected behaviour where possible.

BAGENTS will automatically detect the issue and begin processing within a minute.

---

## Architecture

```
orchestrator
├── plan_issue          → Team Lead agent: reads codebase, assigns agent, writes plan
├── apply_token_budget  → Enforces per-cycle file limit; defers extras to next cycle
├── execute_dev_loop    → Developer agent: writes code, applies patches, runs verification
│   ├── llm_client.ask_large  → Uses higher token limit for code generation
│   ├── file_system.apply_modifications  → Semantic chunk or search/replace patching
│   ├── git_local.run_verification       → Runs VERIFY_COMMAND
│   └── review_code     → Reviewer agent: approves or rejects the diff
└── deliver_pr          → Pushes branch and opens GitHub PR
```

### Agent Prompts (`config/`)

| File | Agent | Role |
|------|-------|------|
| `team_lead.md` | Team Lead | Analyses issue, picks agent, writes plan |
| `backend_dev.md` | Backend Dev | Writes server-side code (Rust, Python, Go, JS) |
| `frontend_dev.md` | Frontend Dev | Writes UI code (React, Vue, HTML/CSS) |
| `devops_dev.md` | DevOps Dev | Edits Cargo.toml, package.json, Dockerfiles, CI YAML |
| `reviewer.md` | Reviewer | Validates the git diff, approves or rejects |

All prompts can be edited without recompiling. The system re-reads them at startup.

# Building a Full Project with BAGENTS

BAGENTS is a highly capable coding engine, but it is not a "one-click project generator." It operates exactly like a team of Senior Developers waiting for well-defined tasks. To build an entire application from scratch, you must step into the role of the Product Manager and Architect.
Here is the recommended workflow for building a complete, complex application:

1. Bootstrap the Repository (Human)

Do not ask BAGENTS to initialize a project from nothing. Run your framework's initialization commands (e.g., cargo new, npx create-next-app, npm init) yourself, set up your base directory structure, and push the initial commit to main.

2. Write Atomic, Focused Issues (Human)

To ensure high-quality code, prevent hallucinations, and respect token budgets, BAGENTS limits the number of files it modifies per cycle. Break your project down into small, logical features.

- Bad : "Build a complete e-commerce backend with payment integration."

- Good : "Create the Prisma schema for the User model and implement the JWT authentication middleware in src/auth.ts."

3. Provide Clear Technical Direction (Human)

BAGENTS is smart, but it cannot read your mind. The more specific your issue, the better the result.

- Mention exact file paths you want modified
- Specify which libraries or design patterns to use.
- If you have specific business logic, outline it in bullet points.

4. Sequential Execution (BAGENTS)

Build your project step-by-step. Let BAGENTS process an issue, review its Pull Request, and merge it into main before creating the next dependent issue. This ensures BAGENTS always reads the most up-to-date, working codebase.

### Example Progression: 

1. Issue #1: "Set up the database connection pool in src/db.rs" ➔ Merge PR
2. Issue #2: "Create the User struct and implement CRUD operations" ➔ Merge PR
3. Issue #3: "Add a REST endpoint for user registration using axum" ➔ Merge PR

By treating BAGENTS as a collaborative engineering team rather than a magic wand, you can incrementally build massive, production-ready systems without writing the boilerplate yourself.
