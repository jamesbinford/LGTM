pub mod json;

pub use json::JsonLedger;

use anyhow::Result;
use crate::models::Review;

/// Trait for review persistence backends
pub trait Ledger: Send + Sync {
    /// Save a review to the ledger
    fn save(&self, review: &Review) -> Result<()>;

    /// Load a review by ID
    fn load(&self, id: &uuid::Uuid) -> Result<Option<Review>>;

    /// Load a review by PR number
    fn load_by_pr(&self, repo: &str, pr_number: u64) -> Result<Option<Review>>;

    /// List all pending reviews
    fn list_pending(&self) -> Result<Vec<Review>>;

    /// List all reviews for a repository
    fn list_by_repo(&self, repo: &str) -> Result<Vec<Review>>;
}
