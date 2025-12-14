# LGTM - Multi-Agent AI Code Review Pipeline

LGTM is an automated code review system written in Rust that coordinates multiple AI models in a pipeline:

1. **OpenAI GPT-4o** performs initial code review on PR diffs
2. **Claude** evaluates and recommends actions on those suggestions
3. **Humans** make final decisions via CLI or PR comments
4. All decisions are persisted to a **decision ledger**

## Features

- Multi-stage AI review pipeline with configurable models
- GitHub Actions integration for automatic PR reviews
- PostgreSQL or JSON file persistence
- Auto-rules engine for automatic decisions
- Slack notifications for critical issues
- Configurable file patterns and severity thresholds

## Requirements

- Rust 1.70+
- OpenAI API key
- Anthropic API key
- GitHub token (for PR integration)
- PostgreSQL (optional, for production)

## Installation

```bash
# Clone the repository
git clone https://github.com/your-org/lgtm.git
cd lgtm

# Build
cargo build --release
```

The binary will be at `target/release/ai-review`.

## Configuration

### Environment Variables

```bash
export OPENAI_API_KEY="sk-..."
export ANTHROPIC_API_KEY="sk-ant-..."
export GITHUB_TOKEN="ghp_..."           # Optional: for PR comments
export DATABASE_URL="postgres://..."     # Optional: for PostgreSQL backend
```

### Configuration File

Create `.ai-review/config.yml` in your repository:

```yaml
review:
  # File patterns to include in review
  include_patterns:
    - "**/*.rs"
    - "**/*.py"
    - "**/*.ts"
    - "**/*.go"

  # File patterns to exclude
  exclude_patterns:
    - "**/test_*.py"
    - "**/*_test.go"
    - "**/migrations/**"
    - "**/node_modules/**"

# Severity handling
severity_thresholds:
  blocking:
    - critical
  warning:
    - high
    - medium

# Auto-rules for automatic decisions
auto_rules:
  - condition: "severity == 'low' AND type == 'style' AND age_days > 14"
    action: auto_dismiss
    reason: "Aged out - low severity style issue"

  - condition: "claude_action == 'accept' AND claude_confidence > 0.95"
    action: auto_accept
    reason: "High confidence Claude recommendation"

# Staleness handling
staleness:
  warn_after_days: 3
  escalate_after_days: 7

# Slack notifications
notifications:
  slack:
    enabled: false
    channel: "#code-review"
    on_critical: true
    on_new_review: false

# Model configuration
models:
  codex:
    model: "gpt-4o"
    temperature: 0.1
  claude:
    model: "claude-sonnet-4-20250514"
    temperature: 0.1
```

## Usage

### Run a Review

```bash
# Review a PR with diff from file
ai-review review \
  --pr 123 \
  --repo owner/repo \
  --sha abc123 \
  --branch feature-branch \
  --diff-file pr.diff \
  --output review_summary.md

# Fetch diff from GitHub and post comment
ai-review review \
  --pr 123 \
  --repo owner/repo \
  --sha abc123 \
  --fetch-diff \
  --post-comment
```

### List Pending Reviews

```bash
ai-review pending
```

### Show Review Details

```bash
ai-review show 123 --repo owner/repo
```

### Make a Decision

```bash
# Accept a suggestion
ai-review decide 123 \
  --repo owner/repo \
  --suggestion S001 \
  --accept \
  --reason "Good catch" \
  --user "developer@example.com"

# Reject a suggestion
ai-review decide 123 \
  --repo owner/repo \
  --suggestion S002 \
  --reject \
  --reason "False positive"
```

### Custom Ledger Path

```bash
ai-review --ledger-path ./custom-ledger pending
```

## GitHub Actions Integration

Add this workflow to `.github/workflows/ai-review.yml`:

```yaml
name: AI Code Review

on:
  pull_request:
    types: [opened, synchronize]

concurrency:
  group: ai-review-${{ github.event.pull_request.number }}
  cancel-in-progress: true

jobs:
  ai-review:
    runs-on: ubuntu-latest
    permissions:
      contents: read
      pull-requests: write

    steps:
      - uses: actions/checkout@v4
        with:
          fetch-depth: 0

      - uses: dtolnay/rust-toolchain@stable

      - uses: Swatinem/rust-cache@v2

      - name: Build ai-review
        run: cargo build --release

      - name: Generate diff
        run: |
          git diff origin/${{ github.base_ref }}...HEAD > pr_diff.txt

      - name: Run AI Review
        env:
          OPENAI_API_KEY: ${{ secrets.OPENAI_API_KEY }}
          ANTHROPIC_API_KEY: ${{ secrets.ANTHROPIC_API_KEY }}
          GITHUB_TOKEN: ${{ secrets.GITHUB_TOKEN }}
        run: |
          ./target/release/ai-review review \
            --pr ${{ github.event.pull_request.number }} \
            --repo ${{ github.repository }} \
            --sha ${{ github.event.pull_request.head.sha }} \
            --branch ${{ github.head_ref }} \
            --diff-file pr_diff.txt \
            --output review_summary.md \
            --post-comment

      - uses: actions/upload-artifact@v4
        if: always()
        with:
          name: ai-review-results
          path: |
            review_summary.md
            .ai-review/ledger/
          retention-days: 30
```

## Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                     Review Pipeline                          │
├─────────────────────────────────────────────────────────────┤
│                                                              │
│  PR Diff ──► Codex (GPT-4o) ──► Claude ──► Human Decision   │
│              │                  │          │                 │
│              ▼                  ▼          ▼                 │
│         Suggestions      Recommendations  Decisions          │
│                                                              │
│              └────────────────┴────────────┘                 │
│                              │                               │
│                              ▼                               │
│                         Ledger (JSON/PostgreSQL)             │
│                              │                               │
│                              ▼                               │
│                      PR Comment / Summary                    │
└─────────────────────────────────────────────────────────────┘
```

### Components

| Component | Description |
|-----------|-------------|
| `orchestrator.rs` | Coordinates the multi-stage review pipeline |
| `adapters/codex.rs` | OpenAI GPT-4o integration for initial review |
| `adapters/claude.rs` | Anthropic Claude integration for evaluation |
| `github/client.rs` | GitHub API client for PR comments and diffs |
| `github/diff.rs` | Unified diff parsing utilities |
| `ledger/json.rs` | File-based persistence (development) |
| `ledger/postgres.rs` | PostgreSQL persistence (production) |
| `config.rs` | YAML configuration system |
| `rules.rs` | Auto-rules engine for automatic decisions |
| `notifications.rs` | Slack notification service |

## Data Models

### Suggestion Types

- `Security` - Security vulnerabilities
- `Performance` - Performance issues
- `Logic` - Logic errors or bugs
- `Style` - Code style issues
- `Documentation` - Missing or incorrect docs

### Severity Levels

- `Critical` - Must fix before merge
- `High` - Should fix before merge
- `Medium` - Should consider fixing
- `Low` - Nice to have

### Recommended Actions (from Claude)

- `Accept` - Agree with the suggestion
- `Reject` - Disagree with the suggestion
- `Modify` - Agree but with modifications

### Human Decisions

- `Accepted` - Apply the fix
- `Rejected` - Ignore the suggestion
- `Deferred` - Review later

## Auto-Rules

Auto-rules automatically make decisions based on conditions:

```yaml
auto_rules:
  # Condition syntax: field operator value [AND/OR field operator value]
  - condition: "severity == 'low' AND type == 'style'"
    action: auto_dismiss
    reason: "Auto-dismissed low severity style issue"

  - condition: "claude_confidence > 0.95"
    action: auto_accept
    reason: "High confidence recommendation"
```

**Supported fields:** `severity`, `type`, `age_days`, `claude_action`, `claude_confidence`

**Supported operators:** `==`, `>`, `>=`, `<`, `<=`

**Supported actions:** `auto_accept`, `auto_dismiss`, `auto_defer`

## Database Setup (PostgreSQL)

For production use with PostgreSQL:

```bash
# Set connection string
export DATABASE_URL="postgres://user:pass@localhost/lgtm"

# Migrations run automatically on first use
```

The schema creates two tables:
- `reviews` - PR review records
- `suggestions` - Individual suggestions with recommendations and decisions

## Development

```bash
# Run tests
cargo test

# Run with logging
RUST_LOG=debug cargo run -- review --pr 1 --repo test/repo --sha abc123

# Format code
cargo fmt

# Lint
cargo clippy
```

## License

MIT
