<p align="center">
  <img src="docs/images/logo.png" alt="Bagents Logo" width="200" />
</p>

# BAGENTS

BAGENTS is an autonomous, end-to-end software engineering orchestration engine built in Rust. Operating as a specialized multi-agent system, it continuously monitors issue trackers, formulates architectural implementation plans, executes targeted codebase mutations, enforces deterministic verification, and orchestrates peer reviews before delivering production-ready Pull Requests.

Engineered with systems design principles, BAGENTS mitigates common LLM integration pitfalls—such as context window pollution and output hallucination—through rigorous Abstract Syntax Tree (AST) parsing, state-driven feedback loops, and robust fault-tolerance mechanisms.

---

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

1.  **Poll & Checkout:** Polls the GitHub API for unresolved issues. Isolates state by creating a targeted feature branch (`feature/issue-<id>`).
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
*   A GitHub Personal Access Token (PAT) with `repo` scopes.

### Initialization

1. Clone the repository and compile the binary:
```bash
   git clone [https://github.com/yourusername/bagents.git](https://github.com/yourusername/bagents.git)
   cd bagents
   cargo build --release

```

2. Establish the environment configuration (`.env`):

```env
   # Version Control Integration
   GITHUB_TOKEN=your_github_pat
   GITHUB_OWNER=target_organization_or_user
   GITHUB_REPO=target_repository_name
   
   # Workspace Isolation
   # Absolute path to the local clone of the target repository. 
   # BAGENTS will perform destructive operations (git reset, branch, checkout) here.
   WORKSPACE_DIR=/path/to/local/target/repo
   
   # LLM Provider Configuration
   LLM_API_URL=[https://api.anthropic.com/v1/messages](https://api.anthropic.com/v1/messages)
   LLM_API_KEY=your_api_key
   LLM_MODEL=claude-3-5-sonnet-20241022
   LLM_TEMPERATURE=0.2
   LLM_JSON_MODE=none # Specify "openai" to enforce JSON schema compliance on compatible endpoints
   
   # Token Thresholds
   LLM_MAX_TOKENS=4096
   LLM_MAX_TOKENS_LARGE=8192
   
   # CI/CD Emulation
   VERIFY_COMMAND=cargo check

```

3. Provision the prompt definitions:
Ensure the `config/` directory is populated with the requisite system prompts: `team_lead.md`, `backend_dev.md`, `frontend_dev.md`, `devops_dev.md`, and `reviewer.md`.
4. Execute the orchestration engine:

```bash
   RUST_LOG=info ./target/release/bagents

```

## License

This architecture is distributed under the [MIT License](https://www.google.com/search?q=LICENSE).

