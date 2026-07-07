<p align="center">
  <img src="docs/images/logo.png" alt="Bagents Logo" width="200" />
</p>

# BAGENTS

BAGENTS is an autonomous, end-to-end software engineering orchestration engine built in Rust. Operating as a specialized multi-agent system, it continuously monitors issue trackers, formulates architectural implementation plans, executes targeted codebase mutations, enforces deterministic verification, and orchestrates peer reviews before delivering production-ready Pull Requests.

Engineered with systems design principles, BAGENTS mitigates common LLM integration pitfalls—such as context window pollution and output hallucination—through rigorous Abstract Syntax Tree (AST) parsing, state-driven feedback loops, and robust fault-tolerance mechanisms.

---

## Example Output

```
2026-06-02T07:27:10.100Z  INFO bagents: Configuration loaded owner="acme-corp" repo="core-api" workspace="/tmp/core-api" model="claude-3-5-sonnet-20241022"
2026-06-02T07:27:10.102Z  INFO bagents::orchestrator: Factory started — polling for issues continuously
2026-06-02T07:27:10.105Z  INFO bagents::orchestrator: Checking tracker for open issues...
2026-06-02T07:27:11.450Z  INFO bagents::orchestrator: Processing issue issue=42 title="Fix out of bounds panic in config parser"
2026-06-02T07:27:11.500Z  INFO bagents::git_local: Resetting workspace to main branch
2026-06-02T07:27:12.100Z  INFO bagents::git_local: Creating new branch branch="feature/issue-42"
2026-06-02T07:27:12.305Z  INFO bagents::orchestrator: Team leader is planning...
2026-06-02T07:27:18.900Z  INFO bagents::orchestrator: Plan ready agent="backend_dev" plan="The issue indicates a panic on line 84 when splitting the configuration string. We need to check if the split iterator yields a second item before accessing it. I will assign backend_dev to modify src/parser/config.rs." files=["src/parser/config.rs"]
2026-06-02T07:27:18.905Z  INFO bagents::orchestrator: Developer writing code attempt=1 max_attempts=6 agent="backend_dev"
2026-06-02T07:27:23.100Z  INFO bagents::clients::llm_client: Agent is reading a file (Read-Before-Write) attempt=1 read_turns=1 file="src/parser/config.rs" start=70 end=90
2026-06-02T07:27:28.450Z  INFO bagents::clients::llm_client: Agent called apply_patch attempt=1
2026-06-02T07:27:28.452Z  INFO bagents::orchestrator: Developer response parsed thought="I have added a length check before accessing index 1 of the parts array."
2026-06-02T07:27:28.460Z  INFO bagents::services::file_system: Patch applied (search_replace_blocks) path="src/parser/config.rs" count=1
2026-06-02T07:27:28.465Z  INFO bagents::git_local: Running verification command: cargo check
2026-06-02T07:27:31.200Z  WARN bagents::orchestrator: Verification failed on modified files attempt=1
2026-06-02T07:27:31.205Z  WARN bagents::orchestrator: Feedback applied: CRITICAL VERIFICATION ERROR: Your code failed the build/linter.
Diagnostic output:
Verification failed — 1 error(s) found:

  src/parser/config.rs:85 — cannot move out of index of `Vec<&str>`

Fix ONLY the errors in files you modified. Do not change unrelated code.
2026-06-02T07:27:31.210Z  INFO bagents::orchestrator: Developer writing code attempt=2 max_attempts=6 agent="backend_dev"
2026-06-02T07:27:40.500Z  INFO bagents::clients::llm_client: Agent called apply_patch attempt=2
2026-06-02T07:27:40.505Z  INFO bagents::orchestrator: Developer response parsed thought="Ah, I attempted to move the string slice out of the Vec instead of copying/referencing it. I will fix the syntax to use .get(1) and handle the Option."
2026-06-02T07:27:40.510Z  INFO bagents::services::file_system: Patch applied (search_replace_blocks) path="src/parser/config.rs" count=1
2026-06-02T07:27:40.515Z  INFO bagents::git_local: Running verification command: cargo check
2026-06-02T07:27:42.800Z  INFO bagents::orchestrator: Reviewer analysing code...
2026-06-02T07:27:47.300Z  INFO bagents::orchestrator: Review approved on attempt 2
2026-06-02T07:27:47.350Z  INFO bagents::git_local: Pushing branch to remote branch="feature/issue-42"
2026-06-02T07:27:49.100Z  INFO bagents::repo_service: Pull request created url="https://github.com/acme-corp/core-api/pull/43"
2026-06-02T07:27:49.105Z  INFO bagents::orchestrator: Issue completed successfully issue=42
2026-06-02T07:27:59.110Z  INFO bagents::orchestrator: Checking tracker for open issues...
2026-06-02T07:28:00.120Z  INFO bagents::orchestrator: No new issues — resting for 30s
```
## Core Architecture

### Multi-Agent State Machine
BAGENTS decouples the engineering lifecycle into discrete, specialized agent roles, reducing cognitive load on the LLM and ensuring clear separation of concerns:
*   **Team Lead (Planner):** Ingests the issue and the repository map. Responsible for architectural scoping, identifying dependency chains, and selecting the exact files and semantic chunks required for the mutation.
*   **Developer (Executor):** Operates strictly on the scoped files. Utilizes specific tool calls (`read_file`, `apply_patch`) to perform surgical edits, circumventing the need to rewrite entire files.
*   **Reviewer (Gatekeeper):** Performs a static analysis of the generated `git diff` against the initial architectural plan. Rejects substandard implementations with actionable feedback, forcing a remediation cycle.

### Semantic AST Parsing (Tree-sitter)
To optimize token utilization and prevent context degradation, BAGENTS employs Tree-sitter for semantic code chunking across multiple languages (Rust, JS, TS, TSX, Python).
*   **Context-Aware Repo Mapping:** Generates a highly compressed repository map containing only symbol signatures (functions, structs, classes) rather than raw file contents.
*   **Surgical Chunking:** The executor interacts with specific AST nodes (e.g., `function_item:from_env`) rather than arbitrary line numbers, drastically reducing the margin for patch application errors.

### Deterministic Verification & Feedback Loop
LLMs are inherently probabilistic; engineering requires determinism. BAGENTS bridges this gap via an active feedback loop:
*   Post-mutation, the engine triggers local build systems or linters (via `VERIFY_COMMAND`).
*   Standard error and standard output streams (e.g., `cargo` compiler diagnostics) are parsed, correlated with the modified files, and fed back into the Developer agent's context. The agent is forced to remediate its own syntax or logical errors before proceeding to the peer review stage.

### Resilient Output Processing & Fault Tolerance
Modern reasoning models (such as DeepSeek-R1) often inject reasoning tokens or produce malformed structural outputs. The `helper_output` pipeline provides defensive parsing:
*   **Heuristic JSON Repair:** Automatically recovers from truncated payloads by intelligently closing strings, arrays, and object braces, preventing pipeline crashes.
*   **Chain-of-Thought Stripping:** Safely extracts actionable JSON from intermediate `<think>` blocks.
*   **Fuzzy Patch Application:** Normalizes search-and-replace blocks to handle CRLF mismatches, trailing whitespace drift, and indentation variance.

### Context & Cost Optimization
*   **Ephemeral Prompt Caching:** Natively supports Anthropic's prompt caching protocols (`anthropic-beta: prompt-caching-2024-10-22`), achieving significant latency reduction and cost efficiency when providing the repository map to the context window.
*   **Token Budgeting:** Implements pagination for file reading. If an architectural plan exceeds the optimal context threshold, the system defers secondary modifications to a subsequent execution cycle.

---

## Execution Pipeline

1.  **Poll & Checkout:** Polls the configured Issue Tracker API for unresolved issues. Isolates state by creating a targeted feature branch (`feature/issue-<id>`).
2.  **Analyze & Plan:** Generates the AST-based repository map. The Team Lead outputs a deterministic execution scope.
3.  **Read-Before-Write (RBW):** The Developer agent inspects the exact current state of the requested files or semantic chunks.
4.  **Mutate:** The Developer agent issues patches via targeted block replacements or AST node overrides.
5.  **Verify:** The system executes local toolchains (e.g., `cargo check`, `npm run lint`). Failures trigger an immediate remediation loop.
6.  **Review:** The Reviewer agent asserts the diff against the issue criteria. Rejections trigger a remediation loop.
7.  **Deliver:** Upon consensus, the system commits the validated state, pushes to the remote, and opens a comprehensive Pull Request.

---

## Deployment & Configuration

### Prerequisites
*   Rust 1.80+ (Edition 2024)
*   Git CLI installed and configured in the system PATH.
*   An API token for your chosen Tracker/Repository Provider (e.g., GitHub PAT, GitLab Access Token).

### Initialization

1. Clone the repository:
```bash
   git clone git@github.com:barisatay0/bagents.git
   cd bagents
```

2. Establish the environment configuration (`.env`):

```env
   # Architecture Configuration
   TRACKER_TYPE=github # "github", "gitlab", or "jira". "jira" is issue tracker ONLY.
   REPO_TYPE=github    # "github", "gitlab", or "forgejo". "forgejo" is repo service ONLY.
                       # Mixed configurations (e.g. tracker "jira" + repo "forgejo") are fully supported.

   # Tracker Configuration
   TRACKER_URL=https://api.github.com
   TRACKER_TOKEN=your_tracker_token
   TRACKER_PROJECT=owner/repo # "owner/repo" for github, project ID for gitlab, KEY for jira
   # TRACKER_USERNAME=email@example.com # Required for Jira

   # Repository Configuration
   REPO_URL=https://api.github.com
   REPO_TOKEN=your_repo_token
   REPO_PROJECT=owner/repo

   # Workspace Isolation
   # Absolute path to the local clone of the target repository. 
   # BAGENTS will perform destructive operations (git reset, branch, checkout) here.
   WORKSPACE_DIR=/path/to/local/target/repo
   
   # LLM Provider Configuration
   LLM_API_URL=https://api.anthropic.com/v1/messages
   LLM_API_KEY=your_api_key
   LLM_MODEL=claude-3-5-sonnet-20241022
   LLM_TEMPERATURE=0.2
   LLM_JSON_MODE=none # Specify "openai" to enforce JSON schema compliance on compatible endpoints
   
   # Token Thresholds
   LLM_MAX_TOKENS=4096
   LLM_MAX_TOKENS_LARGE=8192
   
   # CI/CD Emulation
   VERIFY_COMMAND=cargo check

   # Branch and Polling Customization (Optional)
   # BASE_BRANCH=main # Base/target branch name (default is "main")
   # POLL_INTERVAL_SECS=30 # Polling rest interval in seconds (default is 30)
   # ERROR_RETRY_SECS=60 # Cooldown retry interval on errors in seconds (default is 60)

```

3. Provision the prompt definitions:
Ensure the `config/` directory is populated with the requisite system prompts: `team_lead.md`, `backend_dev.md`, `frontend_dev.md`, `devops_dev.md`, and `reviewer.md`.
4. Execute the orchestration engine:

```bash
   RUST_LOG=info ./target/release/bagents

```

## License

This architecture is distributed under the [MIT License](https://www.google.com/search?q=LICENSE).

