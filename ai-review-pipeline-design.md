# AI Code Review Pipeline - Design Document

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

### 1. Orchestrator (`review_orchestrator.py`)

Entry point for CI pipeline. Responsibilities:

- Extract diff from PR/commit
- Call Codex adapter for review
- Call Claude adapter for recommendations
- Persist results to decision ledger
- Post summary to PR

### 2. Agent Adapters

Thin wrappers normalizing LLM interactions:

```python
# adapters/codex_adapter.py
class CodexAdapter:
    async def review(self, diff: str, context: dict) -> list[Suggestion]

# adapters/claude_adapter.py  
class ClaudeAdapter:
    async def recommend(self, suggestions: list[Suggestion], diff: str) -> list[Recommendation]
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

      - name: Set up Python
        uses: actions/setup-python@v5
        with:
          python-version: '3.12'

      - name: Install dependencies
        run: pip install -r requirements.txt

      - name: Run AI Review
        env:
          OPENAI_API_KEY: ${{ secrets.OPENAI_API_KEY }}
          ANTHROPIC_API_KEY: ${{ secrets.ANTHROPIC_API_KEY }}
          DATABASE_URL: ${{ secrets.DATABASE_URL }}
          GITHUB_TOKEN: ${{ secrets.GITHUB_TOKEN }}
        run: |
          python -m review_orchestrator \
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
│   ├── __init__.py
│   ├── orchestrator.py        # Main entry point
│   ├── adapters/
│   │   ├── __init__.py
│   │   ├── base.py            # Abstract adapter interface
│   │   ├── codex.py           # OpenAI Codex integration
│   │   └── claude.py          # Claude/Anthropic integration
│   ├── ledger/
│   │   ├── __init__.py
│   │   ├── db.py              # Database operations
│   │   ├── models.py          # SQLAlchemy/Pydantic models
│   │   └── migrations/        # Alembic migrations
│   ├── cli/
│   │   ├── __init__.py
│   │   └── main.py            # Click/Typer CLI
│   ├── github/
│   │   ├── __init__.py
│   │   ├── pr.py              # PR comment generation
│   │   └── diff.py            # Diff extraction
│   └── config.py              # Configuration loading
├── tests/
├── .ai-review/
│   └── config.yml
├── requirements.txt
├── pyproject.toml
└── README.md
```

## Implementation Priority

1. **Phase 1: Core Pipeline**
- [ ] Codex adapter with structured output
- [ ] Claude adapter for recommendations
- [ ] Basic orchestrator tying them together
- [ ] File-based persistence (JSON) for MVP
1. **Phase 2: Persistence & CLI**
- [ ] Database schema and migrations
- [ ] Ledger operations (create, query, update)
- [ ] CLI for viewing and deciding
1. **Phase 3: CI Integration**
- [ ] GitHub Actions workflow
- [ ] PR comment generation
- [ ] Diff extraction utilities
1. **Phase 4: Polish**
- [ ] Configuration system
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
