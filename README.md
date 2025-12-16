# LGTM - AI Code Review Pipeline

LGTM is an automated code review system written in Rust that uses OpenAI GPT-4o to review code changes in CI, with results committed back to the repository for human review.

## How It Works

1. **OpenAI GPT-4o** reviews diffs on PRs and commits to main
2. **Review results** are committed as markdown files to `lgtm-reviews/`
3. **Developers** pull and review findings with their local tools (e.g., Claude Code)
4. **Rejected findings** are suppressed to prevent re-flagging

## Features

- Automatic code review on PRs and pushes to main
- Reviews persisted as markdown in the repository
- Suppression system with code-change detection (suppressions auto-expire when code changes)
- JSON or PostgreSQL persistence for review metadata
- Auto-rules engine for automatic decisions
- Configurable file patterns and severity thresholds

## Requirements

- Rust 1.70+
- OpenAI API key
- GitHub token (for PR integration)
- PostgreSQL (optional, for production metadata)

## Installation

```bash
# Clone the repository
git clone https://github.com/jamesbinford/LGTM.git
cd LGTM

# Build
cargo build --release
```

The binary will be at `target/release/ai-review`.

## Configuration

### Environment Variables

```bash
export OPENAI_API_KEY="sk-..."
export GITHUB_TOKEN="ghp_..."           # For PR comments and diff fetching
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

# Staleness handling
staleness:
  warn_after_days: 3
  escalate_after_days: 7

# Model configuration
models:
  codex:
    model: "gpt-4o"
    temperature: 0.1
```

### Suppressions

Suppress findings that have been reviewed and rejected by adding entries to `.ai-review/suppressions.yml`:

```yaml
items:
  - id: "S001-example"
    file: "src/example.rs"
    line_start: 10
    line_end: 15
    finding_type: "logic"  # Optional: security, performance, logic, style, documentation
    pattern: "some text"   # Optional: match description containing this text
    reason: "Intentional design decision"
    suppressed_by: "developer"
    suppressed_at: "2024-01-15T12:00:00Z"
    content_hash: "abc123def456"  # Optional: auto-expires if code changes
    expires: "2024-06-15T12:00:00Z"  # Optional: explicit expiry date
```

Suppressions automatically expire when:
- The explicit `expires` date passes
- The code at the specified lines changes (detected via content hash)

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

# Review a commit (no PR)
ai-review review \
  --repo owner/repo \
  --sha abc123 \
  --branch main \
  --diff-file commit.diff \
  --output review_summary.md

# Fetch diff from GitHub and post comment to PR
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
  --user "developer"

# Reject a suggestion
ai-review decide 123 \
  --repo owner/repo \
  --suggestion S002 \
  --reject \
  --reason "False positive - intentional behavior"
```

## GitHub Actions Integration

Add this workflow to `.github/workflows/ai-reviews.yml`:

```yaml
name: AI Code Review

on:
  pull_request:
    types: [opened, synchronize]
  push:
    branches:
      - main

concurrency:
  group: ai-review-${{ github.event.pull_request.number || github.sha }}
  cancel-in-progress: true

jobs:
  ai-review:
    # Skip reviews on bot commits to prevent infinite loops
    if: github.event_name == 'pull_request' || github.event.head_commit.author.name != 'github-actions[bot]'
    runs-on: ubuntu-latest
    permissions:
      contents: write
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
          if [ "${{ github.event_name }}" = "pull_request" ]; then
            git diff origin/${{ github.base_ref }}...HEAD > pr_diff.txt
          else
            git diff ${{ github.event.before }}..${{ github.sha }} > pr_diff.txt
          fi

      - name: Run AI Review (PR)
        if: github.event_name == 'pull_request'
        env:
          OPENAI_API_KEY: ${{ secrets.OPENAI_API_KEY }}
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

      - name: Run AI Review (Push)
        if: github.event_name == 'push'
        env:
          OPENAI_API_KEY: ${{ secrets.OPENAI_API_KEY }}
        run: |
          ./target/release/ai-review review \
            --repo ${{ github.repository }} \
            --sha ${{ github.sha }} \
            --branch ${{ github.ref_name }} \
            --diff-file pr_diff.txt \
            --output lgtm-reviews/${{ github.sha }}.md

      - name: Commit review results
        if: github.event_name == 'push'
        run: |
          git config user.name "github-actions[bot]"
          git config user.email "github-actions[bot]@users.noreply.github.com"
          git add lgtm-reviews/
          git diff --staged --quiet || git commit -m "Add AI review for ${{ github.sha }}"
          git push
```

## Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                     Review Pipeline                          │
├─────────────────────────────────────────────────────────────┤
│                                                              │
│  PR/Commit Diff ──► Codex (GPT-4o) ──► Review Summary        │
│                          │                   │               │
│                          ▼                   ▼               │
│                    Suggestions         lgtm-reviews/*.md     │
│                          │                                   │
│                          ▼                                   │
│                  Ledger (JSON/PostgreSQL)                    │
│                                                              │
│  Developer pulls ──► Reviews with Claude Code ──► Decides    │
│                                                  │           │
│                                                  ▼           │
│                                        suppressions.yml      │
└─────────────────────────────────────────────────────────────┘
```

### Components

| Component | Description |
|-----------|-------------|
| `orchestrator.rs` | Coordinates the review pipeline |
| `adapters/codex.rs` | OpenAI GPT-4o integration for code review |
| `github/client.rs` | GitHub API client for PR comments and diffs |
| `github/diff.rs` | Unified diff parsing utilities |
| `ledger/json.rs` | File-based persistence (development) |
| `ledger/postgres.rs` | PostgreSQL persistence (production) |
| `suppressions.rs` | Suppression system with change detection |
| `config.rs` | YAML configuration system |
| `rules.rs` | Auto-rules engine for automatic decisions |

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

### Human Decisions

- `Accepted` - Apply the fix
- `Rejected` - Ignore the suggestion (add to suppressions)
- `Deferred` - Review later

## Auto-Rules

Auto-rules automatically make decisions based on conditions:

```yaml
auto_rules:
  # Condition syntax: field operator value [AND field operator value]
  - condition: "severity == 'low' AND type == 'style'"
    action: auto_dismiss
    reason: "Auto-dismissed low severity style issue"

  - condition: "age_days > 14"
    action: auto_defer
    reason: "Stale finding"
```

**Supported fields:** `severity`, `type`, `age_days`, `file_path`

**Supported operators:** `==`, `>`, `>=`, `<`, `<=`

**Supported actions:** `auto_accept`, `auto_dismiss`, `auto_defer`

## Database Setup (PostgreSQL)

For production use with PostgreSQL:

```bash
# Set connection string
export DATABASE_URL="postgres://user:pass@localhost/lgtm"

# Migrations run automatically on first use
```

## Development

```bash
# Run tests
cargo test

# Run with logging
RUST_LOG=debug cargo run -- review --repo test/repo --sha abc123 --diff-file test.diff

# Format code
cargo fmt

# Lint
cargo clippy
```

## License

MIT
