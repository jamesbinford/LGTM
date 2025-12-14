# AI Code Review Pipeline - Design Document

> **Implementation Language:** Rust 🦀

## Overview

A multi-agent code review system where:

- **Claude Code** develops and iterates locally with the developer
- **OpenAI Codex** reviews code on commit/PR (CI-triggered only)
- **Claude Code** evaluates Codex suggestions and makes recommendations
- **Human** makes final decisions via CLI or dashboard

## Architecture

```
LOCAL DEVELOPMENT (no Codex)
┌─────────────────────────────────────────────────────────────────────┐
│  Developer ◀──▶ Claude Code (rapid iteration)                       │
└─────────────────────────────────────────────────────────────────────┘
                              │
                              │ git push / PR opened
                              ▼
CI/CD PIPELINE ───────────────────────────────────────────────────────
│                                                                     │
│   ┌─────────────┐     ┌─────────────┐     ┌──────────────────┐     │
│   │ OpenAI Codex│────▶│ Claude Code │────▶│ Decision Ledger  │     │
│   │  (Review)   │     │ (Recommend) │     │ + PR Comment     │     │
│   └─────────────┘     └─────────────┘     └──────────────────┘     │
│                                                                     │
──────────────────────────────────────────────────────────────────────
                              │
                              ▼
HUMAN DECISION GATE ──────────────────────────────────────────────────
│   • CLI: `review-cli pending` / `review-cli decide ...`            │
│   • PR interface for visibility                                     │
│   • Decisions persisted to ledger (DB), not just PR comments        │
──────────────────────────────────────────────────────────────────────
```

## Components to Build

### 1. Orchestrator (`orchestrator.rs`)

Entry point for CI pipeline. Responsibilities:

- Extract diff from PR/commit
- Call Codex adapter for review
- Call Claude adapter for recommendations
- Persist results to decision ledger
- Post summary to PR

### 2. Agent Adapters

Thin wrappers normalizing LLM interactions:

```rust
// adapters/codex.rs
pub struct CodexAdapter {
    client: reqwest::Client,
    api_key: String,
}

impl CodexAdapter {
    pub async fn review(&self, diff: &str, context: &ReviewContext) -> Result<Vec<Suggestion>>;
}

// adapters/claude.rs
pub struct ClaudeAdapter {
    client: reqwest::Client,
    api_key: String,
}

impl ClaudeAdapter {
    pub async fn recommend(&self, suggestions: &[Suggestion], diff: &str) -> Result<Vec<Recommendation>>;
}
```

### 3. Decision Ledger (Database)

Source of truth for all reviews and decisions.

**Schema:**

```sql
CREATE TABLE reviews (
    id UUID PRIMARY KEY,
    pr_number INTEGER NOT NULL,
    repo VARCHAR(255) NOT NULL,
    branch VARCHAR(255),
    commit_sha VARCHAR(40),
    created_at TIMESTAMP DEFAULT NOW(),
    status VARCHAR(20) DEFAULT 'pending'  -- pending, decided, applied, stale
);

CREATE TABLE suggestions (
    id UUID PRIMARY KEY,
    review_id UUID REFERENCES reviews(id),
    external_id VARCHAR(50),  -- S001, S002, etc.
    type VARCHAR(50),         -- security, performance, style, logic
    severity VARCHAR(20),     -- critical, high, medium, low
    file_path VARCHAR(500),
    line_start INTEGER,
    line_end INTEGER,
    codex_description TEXT,
    codex_proposed_fix TEXT,
    claude_action VARCHAR(20),      -- accept, reject, modify
    claude_rationale TEXT,
    claude_modified_fix TEXT,
    human_decision VARCHAR(20),     -- accepted, rejected, deferred
    human_reason TEXT,
    decided_by VARCHAR(100),
    decided_at TIMESTAMP
);

CREATE INDEX idx_pending ON suggestions(review_id) 
    WHERE human_decision IS NULL;
```

### 4. CLI (`review-cli`)

Human interface for managing decisions:

```bash
review-cli pending                     # List all pending reviews
review-cli show <pr_number>            # Show details for a PR
review-cli decide <pr> <suggestion_id> --accept|--reject [--reason "..."]
review-cli decide <pr> --accept-all-recommended
review-cli stale                       # Show aging reviews
review-cli export <pr> --format json|md
```

### 5. GitHub Actions Workflow

```yaml
# .github/workflows/ai-review.yml
name: AI Code Review

on:
  pull_request:
    types: [opened, synchronize]

jobs:
  ai-review:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
        with:
          fetch-depth: 0

      - name: Install Rust
        uses: dtolnay/rust-toolchain@stable

      - name: Cache cargo
        uses: Swatinem/rust-cache@v2

      - name: Build
        run: cargo build --release

      - name: Run AI Review
        env:
          OPENAI_API_KEY: ${{ secrets.OPENAI_API_KEY }}
          ANTHROPIC_API_KEY: ${{ secrets.ANTHROPIC_API_KEY }}
          DATABASE_URL: ${{ secrets.DATABASE_URL }}
          GITHUB_TOKEN: ${{ secrets.GITHUB_TOKEN }}
        run: |
          ./target/release/ai-review \
            --pr ${{ github.event.pull_request.number }} \
            --repo ${{ github.repository }} \
            --sha ${{ github.event.pull_request.head.sha }}

      - name: Post PR Comment
        uses: actions/github-script@v7
        with:
          script: |
            const fs = require('fs');
            const summary = fs.readFileSync('review_summary.md', 'utf8');
            github.rest.issues.createComment({
              issue_number: context.issue.number,
              owner: context.repo.owner,
              repo: context.repo.repo,
              body: summary
            });
```

## Data Structures

### Codex Output Schema

```json
{
  "suggestions": [
    {
      "id": "S001",
      "type": "security|performance|style|logic",
      "severity": "critical|high|medium|low",
      "location": {
        "file": "src/auth/login.py",
        "line_start": 42,
        "line_end": 48
      },
      "description": "User input passed directly to SQL query without sanitization",
      "proposed_fix": "Use parameterized queries: cursor.execute('SELECT * FROM users WHERE id = ?', (user_id,))"
    }
  ]
}
```

### Claude Recommendation Schema

```json
{
  "recommendations": [
    {
      "suggestion_id": "S001",
      "action": "accept|reject|modify",
      "confidence": 0.95,
      "rationale": "Legitimate SQL injection vulnerability. The proposed fix correctly uses parameterized queries.",
      "modified_fix": null
    }
  ]
}
```

### Decision Record Schema

```json
{
  "suggestion_id": "S001",
  "decision": "accepted|rejected|deferred",
  "reason": "Optional human-provided reason",
  "decided_by": "username",
  "decided_at": "2024-12-14T10:30:00Z"
}
```

## Configuration

```yaml
# .ai-review/config.yml
review:
  # Only review these file types
  include_patterns:
    - "**/*.py"
    - "**/*.rs"
  exclude_patterns:
    - "**/test_*.py"
    - "**/migrations/**"

severity_thresholds:
  # Block merge if any of these exist undecided
  blocking: ["critical"]
  # Warn but allow merge
  warning: ["high", "medium"]

auto_rules:
  # Auto-dismiss low severity style issues after 14 days
  - condition: "severity == 'low' AND type == 'style' AND age_days > 14"
    action: "auto_dismiss"
    reason: "Aged out - low severity style issue"

staleness:
  warn_after_days: 3
  escalate_after_days: 7

notifications:
  slack:
    channel: "#code-review"
    on_critical: true
```

## Project Structure

```
ai-review-pipeline/
├── .github/
│   └── workflows/
│       └── ai-review.yml
├── src/
│   ├── main.rs                # Entry point, CLI dispatch
│   ├── lib.rs                 # Library root
│   ├── orchestrator.rs        # Pipeline coordination
│   ├── adapters/
│   │   ├── mod.rs
│   │   ├── codex.rs           # OpenAI Codex integration
│   │   └── claude.rs          # Claude/Anthropic integration
│   ├── models.rs              # Suggestion, Recommendation, Review structs
│   ├── ledger/
│   │   ├── mod.rs
│   │   ├── json.rs            # JSON file persistence (MVP)
│   │   └── db.rs              # Database operations (Phase 2)
│   ├── cli/
│   │   ├── mod.rs
│   │   └── commands.rs        # Clap CLI commands
│   ├── github/
│   │   ├── mod.rs
│   │   ├── pr.rs              # PR comment generation
│   │   └── diff.rs            # Diff extraction
│   └── config.rs              # Configuration loading
├── tests/
│   ├── adapters_test.rs
│   └── orchestrator_test.rs
├── .ai-review/
│   └── config.yml
├── Cargo.toml
└── README.md
```

## Dependencies (Cargo.toml)

```toml
[package]
name = "ai-review"
version = "0.1.0"
edition = "2021"

[dependencies]
tokio = { version = "1", features = ["full"] }
reqwest = { version = "0.12", features = ["json"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
clap = { version = "4", features = ["derive"] }
octocrab = "0.41"
anyhow = "1"
thiserror = "2"
tracing = "0.1"
tracing-subscriber = "0.3"
uuid = { version = "1", features = ["v4", "serde"] }
chrono = { version = "0.4", features = ["serde"] }

# Phase 2: Database
# sqlx = { version = "0.8", features = ["runtime-tokio", "postgres", "uuid", "chrono"] }

[dev-dependencies]
tokio-test = "0.4"
wiremock = "0.6"
```

## Implementation Priority

1. **Phase 1: Core Pipeline** (Rust)
- [ ] Project scaffolding (Cargo.toml, module structure)
- [ ] Data models with serde (Suggestion, Recommendation, Review)
- [ ] Codex adapter with structured JSON output
- [ ] Claude adapter for recommendations
- [ ] Basic orchestrator tying them together
- [ ] File-based persistence (JSON) for MVP

2. **Phase 2: Persistence & CLI**
- [ ] Database schema and sqlx migrations
- [ ] Ledger operations (create, query, update)
- [ ] Clap CLI for viewing and deciding

3. **Phase 3: CI Integration**
- [ ] GitHub Actions workflow
- [ ] PR comment generation with octocrab
- [ ] Diff extraction utilities

4. **Phase 4: Polish**
- [ ] Configuration system (config.yml parsing)
- [ ] Staleness handling
- [ ] Auto-rules engine
- [ ] Notifications (Slack, etc.)

## Security Considerations

- **Secrets**: API keys via environment variables only, never in config files
- **Code sanitization**: Strip potential secrets from diffs before sending to external APIs
- **Audit trail**: Log all LLM interactions for compliance
- **Data residency**: Consider where code transits (OpenAI vs Anthropic endpoints)
- **Access control**: CLI should respect existing Git/GitHub permissions

## Open Questions

- [ ] Should Codex review full files or just diffs?
- [ ] How to handle very large PRs (token limits)?
- [ ] Integration with existing code scanning tools (CodeQL, Semgrep)?
- [ ] Multi-repo support?
