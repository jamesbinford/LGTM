use anyhow::{Context, Result};
use sqlx::postgres::PgPoolOptions;
use sqlx::{PgPool, Row};
use tracing::{debug, info};
use uuid::Uuid;

use crate::models::{
    DecisionRecord, HumanDecision, Location, Review, ReviewStatus, Severity, Suggestion,
    SuggestionItem, SuggestionType,
};

/// PostgreSQL-backed ledger for production persistence
pub struct PostgresLedger {
    pool: PgPool,
}

impl PostgresLedger {
    /// Create a new PostgreSQL ledger with the given connection string
    pub async fn new(database_url: &str) -> Result<Self> {
        let pool = PgPoolOptions::new()
            .max_connections(5)
            .connect(database_url)
            .await
            .context("Failed to connect to PostgreSQL")?;

        info!("Connected to PostgreSQL");

        Ok(Self { pool })
    }

    /// Run database migrations
    pub async fn migrate(&self) -> Result<()> {
        sqlx::migrate!("./migrations")
            .run(&self.pool)
            .await
            .context("Failed to run database migrations")?;

        info!("Database migrations complete");

        Ok(())
    }

    /// Save a review to the database
    pub async fn save(&self, review: &Review) -> Result<()> {
        let mut tx = self.pool.begin().await?;

        // Upsert the review
        sqlx::query(
            r#"
            INSERT INTO reviews (id, pr_number, repo, branch, commit_sha, created_at, status)
            VALUES ($1, $2, $3, $4, $5, $6, $7)
            ON CONFLICT (id) DO UPDATE SET
                status = EXCLUDED.status
            "#,
        )
        .bind(review.id)
        .bind(review.pr_number.map(|n| n as i64))
        .bind(&review.repo)
        .bind(&review.branch)
        .bind(&review.commit_sha)
        .bind(review.created_at)
        .bind(status_to_str(review.status))
        .execute(&mut *tx)
        .await
        .context("Failed to save review")?;

        // Delete existing suggestions (will be re-inserted)
        sqlx::query("DELETE FROM suggestions WHERE review_id = $1")
            .bind(review.id)
            .execute(&mut *tx)
            .await?;

        // Insert suggestions
        for item in &review.suggestions {
            let s = &item.suggestion;
            let d = &item.decision;

            sqlx::query(
                r#"
                INSERT INTO suggestions (
                    review_id, external_id, suggestion_type, severity,
                    file_path, line_start, line_end, description, proposed_fix,
                    human_decision, human_reason, decided_by, decided_at
                ) VALUES (
                    $1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13
                )
                "#,
            )
            .bind(review.id)
            .bind(&s.id)
            .bind(suggestion_type_to_str(s.suggestion_type))
            .bind(severity_to_str(s.severity))
            .bind(&s.location.file)
            .bind(s.location.line_start as i32)
            .bind(s.location.line_end as i32)
            .bind(&s.description)
            .bind(&s.proposed_fix)
            .bind(d.as_ref().map(|d| decision_to_str(d.decision)))
            .bind(d.as_ref().and_then(|d| d.reason.as_ref()))
            .bind(d.as_ref().map(|d| &d.decided_by))
            .bind(d.as_ref().map(|d| d.decided_at))
            .execute(&mut *tx)
            .await
            .context("Failed to save suggestion")?;
        }

        tx.commit().await?;

        debug!(id = %review.id, "Saved review to database");

        Ok(())
    }

    /// Load a review by ID
    pub async fn load(&self, id: &Uuid) -> Result<Option<Review>> {
        let row = sqlx::query(
            r#"
            SELECT id, pr_number, repo, branch, commit_sha, created_at, status
            FROM reviews WHERE id = $1
            "#,
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;

        match row {
            Some(row) => {
                let review = self.build_review_from_row(&row).await?;
                Ok(Some(review))
            }
            None => Ok(None),
        }
    }

    /// Load a review by PR number
    pub async fn load_by_pr(&self, repo: &str, pr_number: u64) -> Result<Option<Review>> {
        let row = sqlx::query(
            r#"
            SELECT id, pr_number, repo, branch, commit_sha, created_at, status
            FROM reviews
            WHERE repo = $1 AND pr_number = $2
            ORDER BY created_at DESC
            LIMIT 1
            "#,
        )
        .bind(repo)
        .bind(pr_number as i64)
        .fetch_optional(&self.pool)
        .await?;

        match row {
            Some(row) => {
                let review = self.build_review_from_row(&row).await?;
                Ok(Some(review))
            }
            None => Ok(None),
        }
    }

    /// Load a review by commit SHA
    pub async fn load_by_commit(&self, repo: &str, commit_sha: &str) -> Result<Option<Review>> {
        let row = sqlx::query(
            r#"
            SELECT id, pr_number, repo, branch, commit_sha, created_at, status
            FROM reviews
            WHERE repo = $1 AND commit_sha = $2
            ORDER BY created_at DESC
            LIMIT 1
            "#,
        )
        .bind(repo)
        .bind(commit_sha)
        .fetch_optional(&self.pool)
        .await?;

        match row {
            Some(row) => {
                let review = self.build_review_from_row(&row).await?;
                Ok(Some(review))
            }
            None => Ok(None),
        }
    }

    /// List all pending reviews
    pub async fn list_pending(&self) -> Result<Vec<Review>> {
        let rows = sqlx::query(
            r#"
            SELECT id, pr_number, repo, branch, commit_sha, created_at, status
            FROM reviews
            WHERE status = 'pending'
            ORDER BY created_at DESC
            "#,
        )
        .fetch_all(&self.pool)
        .await?;

        let mut reviews = Vec::new();
        for row in rows {
            let review = self.build_review_from_row(&row).await?;
            reviews.push(review);
        }

        Ok(reviews)
    }

    /// List all reviews for a repository
    pub async fn list_by_repo(&self, repo: &str) -> Result<Vec<Review>> {
        let rows = sqlx::query(
            r#"
            SELECT id, pr_number, repo, branch, commit_sha, created_at, status
            FROM reviews
            WHERE repo = $1
            ORDER BY created_at DESC
            "#,
        )
        .bind(repo)
        .fetch_all(&self.pool)
        .await?;

        let mut reviews = Vec::new();
        for row in rows {
            let review = self.build_review_from_row(&row).await?;
            reviews.push(review);
        }

        Ok(reviews)
    }

    /// Mark old reviews as stale
    pub async fn mark_stale(&self, repo: &str, pr_number: u64, except_id: &Uuid) -> Result<u64> {
        let result = sqlx::query(
            r#"
            UPDATE reviews
            SET status = 'stale'
            WHERE repo = $1 AND pr_number = $2 AND id != $3 AND status = 'pending'
            "#,
        )
        .bind(repo)
        .bind(pr_number as i64)
        .bind(except_id)
        .execute(&self.pool)
        .await?;

        Ok(result.rows_affected())
    }

    /// Get statistics for a repository
    pub async fn get_stats(&self, repo: &str) -> Result<RepoStats> {
        let row = sqlx::query(
            r#"
            SELECT
                COUNT(*) FILTER (WHERE status = 'pending') as pending_reviews,
                COUNT(*) FILTER (WHERE status = 'decided') as decided_reviews,
                COUNT(*) as total_reviews
            FROM reviews
            WHERE repo = $1
            "#,
        )
        .bind(repo)
        .fetch_one(&self.pool)
        .await?;

        let suggestion_row = sqlx::query(
            r#"
            SELECT
                COUNT(*) FILTER (WHERE s.human_decision IS NULL) as pending_suggestions,
                COUNT(*) FILTER (WHERE s.severity = 'critical' AND s.human_decision IS NULL) as critical_pending
            FROM suggestions s
            JOIN reviews r ON s.review_id = r.id
            WHERE r.repo = $1 AND r.status = 'pending'
            "#,
        )
        .bind(repo)
        .fetch_one(&self.pool)
        .await?;

        Ok(RepoStats {
            pending_reviews: row.get::<i64, _>("pending_reviews") as u64,
            decided_reviews: row.get::<i64, _>("decided_reviews") as u64,
            total_reviews: row.get::<i64, _>("total_reviews") as u64,
            pending_suggestions: suggestion_row.get::<i64, _>("pending_suggestions") as u64,
            critical_pending: suggestion_row.get::<i64, _>("critical_pending") as u64,
        })
    }

    async fn build_review_from_row(&self, row: &sqlx::postgres::PgRow) -> Result<Review> {
        let id: Uuid = row.get("id");
        let pr_number: Option<i64> = row.get("pr_number");
        let repo: String = row.get("repo");
        let branch: Option<String> = row.get("branch");
        let commit_sha: String = row.get("commit_sha");
        let created_at: chrono::DateTime<chrono::Utc> = row.get("created_at");
        let status: String = row.get("status");

        // Load suggestions for this review
        let suggestion_rows = sqlx::query(
            r#"
            SELECT
                external_id, suggestion_type, severity, file_path, line_start, line_end,
                description, proposed_fix, human_decision, human_reason, decided_by, decided_at
            FROM suggestions
            WHERE review_id = $1
            ORDER BY external_id
            "#,
        )
        .bind(id)
        .fetch_all(&self.pool)
        .await?;

        let mut suggestions = Vec::new();
        for srow in suggestion_rows {
            let suggestion = Suggestion {
                id: srow.get("external_id"),
                suggestion_type: str_to_suggestion_type(srow.get("suggestion_type")),
                severity: str_to_severity(srow.get("severity")),
                location: Location {
                    file: srow.get("file_path"),
                    line_start: srow.get::<i32, _>("line_start") as u32,
                    line_end: srow.get::<i32, _>("line_end") as u32,
                },
                description: srow.get("description"),
                proposed_fix: srow.get("proposed_fix"),
            };

            let decision = match srow.get::<Option<String>, _>("human_decision") {
                Some(dec) => Some(DecisionRecord {
                    suggestion_id: suggestion.id.clone(),
                    decision: str_to_decision(&dec),
                    reason: srow.get("human_reason"),
                    decided_by: srow.get::<Option<String>, _>("decided_by").unwrap_or_default(),
                    decided_at: srow
                        .get::<Option<chrono::DateTime<chrono::Utc>>, _>("decided_at")
                        .unwrap_or_else(chrono::Utc::now),
                }),
                None => None,
            };

            suggestions.push(SuggestionItem {
                suggestion,
                decision,
            });
        }

        Ok(Review {
            id,
            pr_number: pr_number.map(|n| n as u64),
            repo,
            branch,
            commit_sha,
            created_at,
            status: str_to_status(&status),
            suggestions,
        })
    }
}

/// Repository statistics
#[derive(Debug, Clone)]
pub struct RepoStats {
    pub pending_reviews: u64,
    pub decided_reviews: u64,
    pub total_reviews: u64,
    pub pending_suggestions: u64,
    pub critical_pending: u64,
}

// Conversion helpers
fn status_to_str(status: ReviewStatus) -> &'static str {
    match status {
        ReviewStatus::Pending => "pending",
        ReviewStatus::Decided => "decided",
        ReviewStatus::Applied => "applied",
        ReviewStatus::Stale => "stale",
    }
}

fn str_to_status(s: &str) -> ReviewStatus {
    match s {
        "pending" => ReviewStatus::Pending,
        "decided" => ReviewStatus::Decided,
        "applied" => ReviewStatus::Applied,
        "stale" => ReviewStatus::Stale,
        _ => ReviewStatus::Pending,
    }
}

fn suggestion_type_to_str(t: SuggestionType) -> &'static str {
    match t {
        SuggestionType::Security => "security",
        SuggestionType::Performance => "performance",
        SuggestionType::Style => "style",
        SuggestionType::Logic => "logic",
        SuggestionType::Documentation => "documentation",
    }
}

fn str_to_suggestion_type(s: &str) -> SuggestionType {
    match s {
        "security" => SuggestionType::Security,
        "performance" => SuggestionType::Performance,
        "style" => SuggestionType::Style,
        "logic" => SuggestionType::Logic,
        "documentation" => SuggestionType::Documentation,
        _ => SuggestionType::Logic,
    }
}

fn severity_to_str(s: Severity) -> &'static str {
    match s {
        Severity::Critical => "critical",
        Severity::High => "high",
        Severity::Medium => "medium",
        Severity::Low => "low",
    }
}

fn str_to_severity(s: &str) -> Severity {
    match s {
        "critical" => Severity::Critical,
        "high" => Severity::High,
        "medium" => Severity::Medium,
        "low" => Severity::Low,
        _ => Severity::Low,
    }
}

fn decision_to_str(d: HumanDecision) -> &'static str {
    match d {
        HumanDecision::Accepted => "accepted",
        HumanDecision::Rejected => "rejected",
        HumanDecision::Deferred => "deferred",
    }
}

fn str_to_decision(s: &str) -> HumanDecision {
    match s {
        "accepted" => HumanDecision::Accepted,
        "rejected" => HumanDecision::Rejected,
        "deferred" => HumanDecision::Deferred,
        _ => HumanDecision::Deferred,
    }
}
