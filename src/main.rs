use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use tracing::info;
use tracing_subscriber::EnvFilter;

use ai_review::{
    generate_summary, ClaudeAdapter, CodexAdapter, GitHubClient, JsonLedger, Ledger,
    Orchestrator, ReviewContext,
};

#[derive(Parser)]
#[command(name = "ai-review")]
#[command(about = "Multi-agent AI code review pipeline")]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    /// Path to ledger directory
    #[arg(long, default_value = ".ai-review/ledger")]
    ledger_path: PathBuf,
}

#[derive(Subcommand)]
enum Commands {
    /// Run a review on a PR or commit
    Review {
        /// PR number (optional for push events)
        #[arg(long)]
        pr: Option<u64>,

        /// Repository (owner/repo)
        #[arg(long)]
        repo: String,

        /// Commit SHA
        #[arg(long)]
        sha: String,

        /// Branch name
        #[arg(long)]
        branch: Option<String>,

        /// Path to diff file (or read from stdin if not provided)
        #[arg(long)]
        diff_file: Option<PathBuf>,

        /// Output summary to file
        #[arg(long, default_value = "review_summary.md")]
        output: PathBuf,

        /// Post review summary as PR comment
        #[arg(long)]
        post_comment: bool,

        /// Fetch diff from GitHub API instead of file
        #[arg(long)]
        fetch_diff: bool,
    },

    /// List pending reviews
    Pending,

    /// Show review details
    Show {
        /// PR number
        pr: u64,

        /// Repository (owner/repo)
        #[arg(long)]
        repo: String,
    },

    /// Make a decision on a suggestion
    Decide {
        /// PR number
        pr: u64,

        /// Repository (owner/repo)
        #[arg(long)]
        repo: String,

        /// Suggestion ID (e.g., S001)
        #[arg(long)]
        suggestion: String,

        /// Accept the suggestion
        #[arg(long, conflicts_with = "reject")]
        accept: bool,

        /// Reject the suggestion
        #[arg(long, conflicts_with = "accept")]
        reject: bool,

        /// Reason for decision
        #[arg(long)]
        reason: Option<String>,

        /// Your username
        #[arg(long, env = "USER")]
        user: String,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive("ai_review=info".parse()?))
        .init();

    let cli = Cli::parse();

    match cli.command {
        Commands::Review {
            pr,
            repo,
            sha,
            branch,
            diff_file,
            output,
            post_comment,
            fetch_diff,
        } => {
            run_review(
                cli.ledger_path,
                pr,
                repo,
                sha,
                branch,
                diff_file,
                output,
                post_comment,
                fetch_diff,
            )
            .await?;
        }
        Commands::Pending => {
            list_pending(cli.ledger_path)?;
        }
        Commands::Show { pr, repo } => {
            show_review(cli.ledger_path, &repo, pr)?;
        }
        Commands::Decide {
            pr,
            repo,
            suggestion,
            accept,
            reject,
            reason,
            user,
        } => {
            make_decision(cli.ledger_path, &repo, pr, &suggestion, accept, reject, reason, &user)?;
        }
    }

    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn run_review(
    ledger_path: PathBuf,
    pr: Option<u64>,
    repo: String,
    sha: String,
    branch: Option<String>,
    diff_file: Option<PathBuf>,
    output: PathBuf,
    post_comment: bool,
    fetch_diff: bool,
) -> Result<()> {
    // Validate that PR is provided if post_comment or fetch_diff is requested
    if post_comment && pr.is_none() {
        anyhow::bail!("--post-comment requires --pr to be specified");
    }
    if fetch_diff && pr.is_none() {
        anyhow::bail!("--fetch-diff requires --pr to be specified");
    }

    let openai_key = std::env::var("OPENAI_API_KEY").context("OPENAI_API_KEY not set")?;
    let anthropic_key = std::env::var("ANTHROPIC_API_KEY").context("ANTHROPIC_API_KEY not set")?;

    let codex = CodexAdapter::new(openai_key);
    let claude = ClaudeAdapter::new(anthropic_key);
    let ledger = JsonLedger::new(&ledger_path)?;

    let orchestrator = Orchestrator::new(codex, claude, ledger);

    // Parse owner/repo
    let (owner, repo_name) = parse_repo(&repo)?;

    // Get GitHub client if needed
    let github = if post_comment || fetch_diff {
        let token = std::env::var("GITHUB_TOKEN").context("GITHUB_TOKEN not set")?;
        Some(GitHubClient::new(&token)?)
    } else {
        None
    };

    // Read diff
    let diff = if fetch_diff {
        let gh = github.as_ref().unwrap();
        let pr_num = pr.unwrap(); // Safe due to validation above
        info!("Fetching diff from GitHub API");
        gh.get_pr_diff(owner, repo_name, pr_num).await?
    } else {
        match diff_file {
            Some(path) => fs::read_to_string(&path)
                .with_context(|| format!("Failed to read diff file: {}", path.display()))?,
            None => {
                use std::io::Read;
                let mut buffer = String::new();
                std::io::stdin()
                    .read_to_string(&mut buffer)
                    .context("Failed to read diff from stdin")?;
                buffer
            }
        }
    };

    let context = ReviewContext {
        pr_number: pr,
        repo: repo.clone(),
        branch,
        commit_sha: sha,
        base_sha: None,
    };

    let review = orchestrator.review(&diff, context).await?;

    // Generate and write summary
    let summary = generate_summary(&review);
    fs::write(&output, &summary)
        .with_context(|| format!("Failed to write summary: {}", output.display()))?;

    info!(output = %output.display(), "Review summary written");

    // Post comment to PR if requested
    if post_comment {
        let gh = github.as_ref().unwrap();
        let pr_num = pr.unwrap(); // Safe due to validation above
        let comment_id = gh.post_comment(owner, repo_name, pr_num, &summary).await?;
        info!(comment_id, "Posted review comment to PR");
        println!("Posted review comment (ID: {})", comment_id);
    }

    // Print to stdout as well
    println!("{}", summary);

    Ok(())
}

/// Parse owner and repo from "owner/repo" format
fn parse_repo(repo: &str) -> Result<(&str, &str)> {
    let parts: Vec<&str> = repo.split('/').collect();
    if parts.len() != 2 {
        anyhow::bail!("Invalid repo format. Expected 'owner/repo', got: {}", repo);
    }
    Ok((parts[0], parts[1]))
}

fn list_pending(ledger_path: PathBuf) -> Result<()> {
    let ledger = JsonLedger::new(&ledger_path)?;
    let reviews = ledger.list_pending()?;

    if reviews.is_empty() {
        println!("No pending reviews.");
        return Ok(());
    }

    println!("Pending Reviews:\n");
    for review in reviews {
        let pending_count = review.pending_suggestions().len();
        let target = match review.pr_number {
            Some(pr) => format!("PR #{}", pr),
            None => format!("commit {}", &review.commit_sha[..7.min(review.commit_sha.len())]),
        };
        println!(
            "  {} in {} - {} pending suggestions",
            target, review.repo, pending_count
        );
        println!("    ID: {}", review.id);
        println!("    Commit: {}", review.commit_sha);
        println!();
    }

    Ok(())
}

fn show_review(ledger_path: PathBuf, repo: &str, pr: u64) -> Result<()> {
    let ledger = JsonLedger::new(&ledger_path)?;
    let review = ledger
        .load_by_pr(repo, pr)?
        .with_context(|| format!("No review found for PR #{} in {}", pr, repo))?;

    let summary = generate_summary(&review);
    println!("{}", summary);

    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn make_decision(
    ledger_path: PathBuf,
    repo: &str,
    pr: u64,
    suggestion_id: &str,
    accept: bool,
    reject: bool,
    reason: Option<String>,
    user: &str,
) -> Result<()> {
    use ai_review::{DecisionRecord, HumanDecision};

    let ledger = JsonLedger::new(&ledger_path)?;
    let mut review = ledger
        .load_by_pr(repo, pr)?
        .with_context(|| format!("No review found for PR #{} in {}", pr, repo))?;

    let decision = if accept {
        HumanDecision::Accepted
    } else if reject {
        HumanDecision::Rejected
    } else {
        anyhow::bail!("Must specify --accept or --reject");
    };

    // Find and update the suggestion
    let item = review
        .suggestions
        .iter_mut()
        .find(|s| s.suggestion.id == suggestion_id)
        .with_context(|| format!("Suggestion {} not found", suggestion_id))?;

    item.decision = Some(DecisionRecord {
        suggestion_id: suggestion_id.to_string(),
        decision,
        reason,
        decided_by: user.to_string(),
        decided_at: chrono::Utc::now(),
    });

    // Update status if all decided
    if review.is_fully_decided() {
        review.status = ai_review::ReviewStatus::Decided;
    }

    ledger.save(&review)?;

    println!(
        "Recorded {:?} for suggestion {} by {}",
        decision, suggestion_id, user
    );

    Ok(())
}
