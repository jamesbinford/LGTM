use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use tracing::{debug, info};
use uuid::Uuid;

use super::Ledger;
use crate::models::{Review, ReviewStatus};

/// JSON file-based ledger for MVP persistence
pub struct JsonLedger {
    base_path: PathBuf,
}

impl JsonLedger {
    pub fn new(base_path: impl AsRef<Path>) -> Result<Self> {
        let base_path = base_path.as_ref().to_path_buf();
        fs::create_dir_all(&base_path)
            .with_context(|| format!("Failed to create ledger directory: {}", base_path.display()))?;

        info!(path = %base_path.display(), "Initialized JSON ledger");

        Ok(Self { base_path })
    }

    fn review_path(&self, id: &Uuid) -> PathBuf {
        self.base_path.join(format!("{}.json", id))
    }

    fn index_path(&self) -> PathBuf {
        self.base_path.join("index.json")
    }

    fn load_index(&self) -> Result<ReviewIndex> {
        let path = self.index_path();
        if !path.exists() {
            return Ok(ReviewIndex::default());
        }

        let content = fs::read_to_string(&path)
            .with_context(|| format!("Failed to read index: {}", path.display()))?;

        serde_json::from_str(&content).context("Failed to parse index")
    }

    fn save_index(&self, index: &ReviewIndex) -> Result<()> {
        let path = self.index_path();
        let content = serde_json::to_string_pretty(index)?;
        fs::write(&path, content)
            .with_context(|| format!("Failed to write index: {}", path.display()))?;
        Ok(())
    }
}

#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
struct ReviewIndex {
    reviews: Vec<ReviewIndexEntry>,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct ReviewIndexEntry {
    id: Uuid,
    repo: String,
    pr_number: Option<u64>,
    commit_sha: String,
    status: ReviewStatus,
}

impl Ledger for JsonLedger {
    fn save(&self, review: &Review) -> Result<()> {
        let path = self.review_path(&review.id);
        let content = serde_json::to_string_pretty(review)?;

        fs::write(&path, content)
            .with_context(|| format!("Failed to write review: {}", path.display()))?;

        // Update index
        let mut index = self.load_index()?;

        // Remove existing entry if present
        index.reviews.retain(|r| r.id != review.id);

        // Add new entry
        index.reviews.push(ReviewIndexEntry {
            id: review.id,
            repo: review.repo.clone(),
            pr_number: review.pr_number,
            commit_sha: review.commit_sha.clone(),
            status: review.status,
        });

        self.save_index(&index)?;

        debug!(id = %review.id, "Saved review to ledger");

        Ok(())
    }

    fn load(&self, id: &Uuid) -> Result<Option<Review>> {
        let path = self.review_path(id);
        if !path.exists() {
            return Ok(None);
        }

        let content = fs::read_to_string(&path)
            .with_context(|| format!("Failed to read review: {}", path.display()))?;

        let review = serde_json::from_str(&content).context("Failed to parse review")?;

        Ok(Some(review))
    }

    fn load_by_pr(&self, repo: &str, pr_number: u64) -> Result<Option<Review>> {
        let index = self.load_index()?;

        let entry = index
            .reviews
            .iter()
            .find(|r| r.repo == repo && r.pr_number == Some(pr_number));

        match entry {
            Some(e) => self.load(&e.id),
            None => Ok(None),
        }
    }

    fn load_by_commit(&self, repo: &str, commit_sha: &str) -> Result<Option<Review>> {
        let index = self.load_index()?;

        let entry = index
            .reviews
            .iter()
            .find(|r| r.repo == repo && r.commit_sha == commit_sha);

        match entry {
            Some(e) => self.load(&e.id),
            None => Ok(None),
        }
    }

    fn list_pending(&self) -> Result<Vec<Review>> {
        let index = self.load_index()?;

        let mut reviews = Vec::new();
        for entry in index.reviews.iter().filter(|r| r.status == ReviewStatus::Pending) {
            if let Some(review) = self.load(&entry.id)? {
                reviews.push(review);
            }
        }

        Ok(reviews)
    }

    fn list_by_repo(&self, repo: &str) -> Result<Vec<Review>> {
        let index = self.load_index()?;

        let mut reviews = Vec::new();
        for entry in index.reviews.iter().filter(|r| r.repo == repo) {
            if let Some(review) = self.load(&entry.id)? {
                reviews.push(review);
            }
        }

        Ok(reviews)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::ReviewContext;
    use tempfile::tempdir;

    #[test]
    fn test_save_and_load() {
        let dir = tempdir().unwrap();
        let ledger = JsonLedger::new(dir.path()).unwrap();

        let review = Review::new(ReviewContext {
            pr_number: Some(123),
            repo: "owner/repo".to_string(),
            branch: Some("feature".to_string()),
            commit_sha: "abc123".to_string(),
            base_sha: None,
        });

        ledger.save(&review).unwrap();

        let loaded = ledger.load(&review.id).unwrap().unwrap();
        assert_eq!(loaded.pr_number, Some(123));
        assert_eq!(loaded.repo, "owner/repo");
    }

    #[test]
    fn test_load_by_pr() {
        let dir = tempdir().unwrap();
        let ledger = JsonLedger::new(dir.path()).unwrap();

        let review = Review::new(ReviewContext {
            pr_number: Some(456),
            repo: "owner/repo".to_string(),
            branch: None,
            commit_sha: "def456".to_string(),
            base_sha: None,
        });

        ledger.save(&review).unwrap();

        let loaded = ledger.load_by_pr("owner/repo", 456).unwrap().unwrap();
        assert_eq!(loaded.id, review.id);
    }

    #[test]
    fn test_load_by_commit() {
        let dir = tempdir().unwrap();
        let ledger = JsonLedger::new(dir.path()).unwrap();

        let review = Review::new(ReviewContext {
            pr_number: None,
            repo: "owner/repo".to_string(),
            branch: Some("main".to_string()),
            commit_sha: "abc789".to_string(),
            base_sha: None,
        });

        ledger.save(&review).unwrap();

        let loaded = ledger.load_by_commit("owner/repo", "abc789").unwrap().unwrap();
        assert_eq!(loaded.id, review.id);
        assert_eq!(loaded.pr_number, None);
    }
}
