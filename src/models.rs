use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Severity level for a code review suggestion
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Critical,
    High,
    Medium,
    Low,
}

/// Type of code review suggestion
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SuggestionType {
    Security,
    Performance,
    Style,
    Logic,
    Documentation,
}

/// Location in source code
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Location {
    pub file: String,
    pub line_start: u32,
    pub line_end: u32,
}

/// A suggestion from the Codex reviewer
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Suggestion {
    pub id: String,
    #[serde(rename = "type")]
    pub suggestion_type: SuggestionType,
    pub severity: Severity,
    pub location: Location,
    pub description: String,
    pub proposed_fix: Option<String>,
}

/// Claude's recommended action for a suggestion
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RecommendedAction {
    Accept,
    Reject,
    Modify,
}

/// Claude's recommendation on a Codex suggestion
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Recommendation {
    pub suggestion_id: String,
    pub action: RecommendedAction,
    pub confidence: f64,
    pub rationale: String,
    pub modified_fix: Option<String>,
}

/// Human decision on a suggestion
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HumanDecision {
    Accepted,
    Rejected,
    Deferred,
}

/// Record of a human decision
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecisionRecord {
    pub suggestion_id: String,
    pub decision: HumanDecision,
    pub reason: Option<String>,
    pub decided_by: String,
    pub decided_at: DateTime<Utc>,
}

/// Status of a review
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ReviewStatus {
    #[default]
    Pending,
    Decided,
    Applied,
    Stale,
}

/// A complete review record combining all stages
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Review {
    pub id: Uuid,
    pub pr_number: u64,
    pub repo: String,
    pub branch: Option<String>,
    pub commit_sha: String,
    pub created_at: DateTime<Utc>,
    pub status: ReviewStatus,
    pub suggestions: Vec<SuggestionWithRecommendation>,
}

/// A suggestion paired with its recommendation and decision
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SuggestionWithRecommendation {
    pub suggestion: Suggestion,
    pub recommendation: Option<Recommendation>,
    pub decision: Option<DecisionRecord>,
}

/// Context for a review request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewContext {
    pub pr_number: u64,
    pub repo: String,
    pub branch: Option<String>,
    pub commit_sha: String,
    pub base_sha: Option<String>,
}

impl Review {
    pub fn new(context: ReviewContext) -> Self {
        Self {
            id: Uuid::new_v4(),
            pr_number: context.pr_number,
            repo: context.repo,
            branch: context.branch,
            commit_sha: context.commit_sha,
            created_at: Utc::now(),
            status: ReviewStatus::Pending,
            suggestions: Vec::new(),
        }
    }

    /// Check if all suggestions have been decided
    pub fn is_fully_decided(&self) -> bool {
        self.suggestions
            .iter()
            .all(|s| s.decision.is_some())
    }

    /// Get pending suggestions (no human decision yet)
    pub fn pending_suggestions(&self) -> Vec<&SuggestionWithRecommendation> {
        self.suggestions
            .iter()
            .filter(|s| s.decision.is_none())
            .collect()
    }

    /// Get suggestions by severity
    pub fn suggestions_by_severity(&self, severity: Severity) -> Vec<&SuggestionWithRecommendation> {
        self.suggestions
            .iter()
            .filter(|s| s.suggestion.severity == severity)
            .collect()
    }
}
