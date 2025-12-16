use anyhow::{Context, Result};
use tracing::info;

use crate::adapters::CodexAdapter;
use crate::ledger::Ledger;
use crate::models::{Review, ReviewContext, ReviewStatus, SuggestionItem};

/// Orchestrates the AI review pipeline
pub struct Orchestrator<L: Ledger> {
    codex: CodexAdapter,
    ledger: L,
}

impl<L: Ledger> Orchestrator<L> {
    pub fn new(codex: CodexAdapter, ledger: L) -> Self {
        Self {
            codex,
            ledger,
        }
    }

    /// Run the review pipeline for a PR or commit
    pub async fn review(&self, diff: &str, context: ReviewContext) -> Result<Review> {
        info!(
            pr = ?context.pr_number,
            repo = %context.repo,
            commit = %context.commit_sha,
            "Starting review pipeline"
        );

        // Check for existing review by PR number or commit SHA
        let existing = if let Some(pr) = context.pr_number {
            self.ledger.load_by_pr(&context.repo, pr)?
        } else {
            self.ledger.load_by_commit(&context.repo, &context.commit_sha)?
        };

        if let Some(existing) = existing {
            if existing.commit_sha == context.commit_sha {
                info!("Review already exists for this commit");
                return Ok(existing);
            }
            info!("New commit detected, creating new review");
        }

        // Create new review
        let mut review = Review::new(context.clone());

        // Run Codex review
        info!("Running Codex review");
        let suggestions = self
            .codex
            .review(diff, &context)
            .await
            .context("Codex review failed")?;

        if suggestions.is_empty() {
            info!("No issues found by Codex");
            review.status = ReviewStatus::Decided;
            self.ledger.save(&review)?;
            return Ok(review);
        }

        info!(count = suggestions.len(), "Codex found issues");

        // Store suggestions
        for suggestion in suggestions {
            review.suggestions.push(SuggestionItem {
                suggestion,
                decision: None,
            });
        }

        // Save review
        self.ledger.save(&review)?;

        info!(
            id = %review.id,
            suggestions = review.suggestions.len(),
            "Review pipeline complete"
        );

        Ok(review)
    }

    /// Get the ledger for direct access
    pub fn ledger(&self) -> &L {
        &self.ledger
    }
}

/// Generate a markdown summary for PR comment
pub fn generate_summary(review: &Review) -> String {
    let mut md = String::new();

    md.push_str("## AI Code Review Summary\n\n");

    if review.suggestions.is_empty() {
        md.push_str("No issues found.\n");
        return md;
    }

    // Group by severity
    let critical: Vec<_> = review
        .suggestions
        .iter()
        .filter(|s| s.suggestion.severity == crate::models::Severity::Critical)
        .collect();

    let high: Vec<_> = review
        .suggestions
        .iter()
        .filter(|s| s.suggestion.severity == crate::models::Severity::High)
        .collect();

    let medium: Vec<_> = review
        .suggestions
        .iter()
        .filter(|s| s.suggestion.severity == crate::models::Severity::Medium)
        .collect();

    let low: Vec<_> = review
        .suggestions
        .iter()
        .filter(|s| s.suggestion.severity == crate::models::Severity::Low)
        .collect();

    // Summary counts
    md.push_str(&format!(
        "| Severity | Count |\n|----------|-------|\n| Critical | {} |\n| High | {} |\n| Medium | {} |\n| Low | {} |\n\n",
        critical.len(),
        high.len(),
        medium.len(),
        low.len()
    ));

    // Details for each suggestion
    md.push_str("### Suggestions\n\n");

    for item in &review.suggestions {
        let s = &item.suggestion;
        let severity_emoji = match s.severity {
            crate::models::Severity::Critical => "🔴",
            crate::models::Severity::High => "🟠",
            crate::models::Severity::Medium => "🟡",
            crate::models::Severity::Low => "🟢",
        };

        let severity_str = format!("{:?}", s.severity).to_uppercase();
        let type_str = format!("{:?}", s.suggestion_type);
        md.push_str(&format!(
            "#### {} {} `{}` - {}\n",
            severity_emoji, severity_str, s.id, type_str
        ));

        md.push_str(&format!(
            "**File:** `{}` (lines {}-{})\n\n",
            s.location.file, s.location.line_start, s.location.line_end
        ));

        md.push_str(&format!("{}\n\n", s.description));

        if let Some(fix) = &s.proposed_fix {
            md.push_str(&format!("**Proposed fix:**\n```\n{}\n```\n\n", fix));
        }

        md.push_str("---\n\n");
    }

    // Decision instructions
    md.push_str(&format!(
        "\n**Review ID:** `{}`\n\n",
        review.id
    ));

    md.push_str("Use `review-cli decide` to accept or reject suggestions.\n");

    md
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::*;

    #[test]
    fn test_generate_summary_empty() {
        let review = Review::new(ReviewContext {
            pr_number: Some(1),
            repo: "test/repo".to_string(),
            branch: None,
            commit_sha: "abc".to_string(),
            base_sha: None,
        });

        let summary = generate_summary(&review);
        assert!(summary.contains("No issues found"));
    }

    #[test]
    fn test_generate_summary_empty_no_pr() {
        let review = Review::new(ReviewContext {
            pr_number: None,
            repo: "test/repo".to_string(),
            branch: Some("main".to_string()),
            commit_sha: "abc".to_string(),
            base_sha: None,
        });

        let summary = generate_summary(&review);
        assert!(summary.contains("No issues found"));
    }
}
