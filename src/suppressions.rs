use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tracing::{debug, info};

/// A suppressed finding that should not be re-reported
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Suppression {
    /// Unique identifier for this suppression
    pub id: String,
    /// File path where the finding was suppressed
    pub file: String,
    /// Starting line number
    pub line_start: u32,
    /// Ending line number
    pub line_end: u32,
    /// Type of finding (security, performance, logic, style, documentation)
    pub finding_type: Option<String>,
    /// Pattern to match in the description (optional)
    pub pattern: Option<String>,
    /// Reason for suppression
    pub reason: String,
    /// Who suppressed it
    pub suppressed_by: String,
    /// When it was suppressed
    pub suppressed_at: DateTime<Utc>,
    /// Git blob hash of the suppressed lines (for change detection)
    pub content_hash: Option<String>,
    /// Explicit expiry date (optional)
    pub expires: Option<DateTime<Utc>>,
}

/// Collection of suppressions
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Suppressions {
    #[serde(default)]
    pub items: Vec<Suppression>,
}

impl Suppressions {
    /// Load suppressions from a YAML file
    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();

        if !path.exists() {
            debug!(path = %path.display(), "Suppressions file not found, using empty list");
            return Ok(Self::default());
        }

        let content = fs::read_to_string(path)
            .with_context(|| format!("Failed to read suppressions file: {}", path.display()))?;

        let suppressions: Suppressions = serde_yaml::from_str(&content)
            .with_context(|| format!("Failed to parse suppressions file: {}", path.display()))?;

        info!(
            path = %path.display(),
            count = suppressions.items.len(),
            "Loaded suppressions"
        );

        Ok(suppressions)
    }

    /// Load from the default location
    pub fn load_default() -> Result<Self> {
        Self::load(".ai-review/suppressions.yml")
    }

    /// Save suppressions to a YAML file
    pub fn save(&self, path: impl AsRef<Path>) -> Result<()> {
        let path = path.as_ref();

        // Ensure parent directory exists
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        let content = serde_yaml::to_string(self)
            .context("Failed to serialize suppressions")?;

        fs::write(path, content)
            .with_context(|| format!("Failed to write suppressions file: {}", path.display()))?;

        info!(path = %path.display(), "Saved suppressions");

        Ok(())
    }

    /// Save to the default location
    pub fn save_default(&self) -> Result<()> {
        self.save(".ai-review/suppressions.yml")
    }

    /// Add a new suppression
    pub fn add(&mut self, suppression: Suppression) {
        // Remove any existing suppression with the same ID
        self.items.retain(|s| s.id != suppression.id);
        self.items.push(suppression);
    }

    /// Get active (non-expired, non-stale) suppressions
    pub fn active(&self, repo_root: Option<&Path>) -> Vec<&Suppression> {
        let now = Utc::now();

        self.items
            .iter()
            .filter(|s| {
                // Check explicit expiry
                if let Some(expires) = s.expires {
                    if now > expires {
                        debug!(id = %s.id, "Suppression expired by date");
                        return false;
                    }
                }

                // Check if code has changed (content hash mismatch)
                if let Some(ref expected_hash) = s.content_hash {
                    if let Some(root) = repo_root {
                        if let Some(current_hash) = get_content_hash(root, &s.file, s.line_start, s.line_end) {
                            if &current_hash != expected_hash {
                                debug!(
                                    id = %s.id,
                                    expected = %expected_hash,
                                    current = %current_hash,
                                    "Suppression invalidated by code change"
                                );
                                return false;
                            }
                        }
                    }
                }

                true
            })
            .collect()
    }

    /// Check if a finding should be suppressed
    pub fn is_suppressed(
        &self,
        file: &str,
        line_start: u32,
        line_end: u32,
        finding_type: &str,
        description: &str,
        repo_root: Option<&Path>,
    ) -> Option<&Suppression> {
        for suppression in self.active(repo_root) {
            // Check file match
            if suppression.file != file {
                continue;
            }

            // Check line overlap
            let overlaps = line_start <= suppression.line_end && line_end >= suppression.line_start;
            if !overlaps {
                continue;
            }

            // Check finding type match (if specified)
            if let Some(ref stype) = suppression.finding_type {
                if stype != finding_type {
                    continue;
                }
            }

            // Check pattern match (if specified)
            if let Some(ref pattern) = suppression.pattern {
                if !description.to_lowercase().contains(&pattern.to_lowercase()) {
                    continue;
                }
            }

            // All checks passed - this finding is suppressed
            return Some(suppression);
        }

        None
    }

    /// Generate a prompt snippet listing active suppressions for OpenAI
    pub fn to_prompt(&self, repo_root: Option<&Path>) -> String {
        let active = self.active(repo_root);

        if active.is_empty() {
            return String::new();
        }

        let mut prompt = String::from("\n\nPreviously reviewed and suppressed findings (DO NOT report these again):\n");

        for s in active {
            prompt.push_str(&format!(
                "- {} (lines {}-{}): {} [Reason: {}]\n",
                s.file, s.line_start, s.line_end,
                s.pattern.as_deref().unwrap_or("any issue"),
                s.reason
            ));
        }

        prompt
    }

    /// Remove expired and invalidated suppressions
    pub fn cleanup(&mut self, repo_root: Option<&Path>) {
        let now = Utc::now();
        let initial_count = self.items.len();

        self.items.retain(|s| {
            // Remove expired
            if let Some(expires) = s.expires {
                if now > expires {
                    return false;
                }
            }

            // Remove if code changed
            if let Some(ref expected_hash) = s.content_hash {
                if let Some(root) = repo_root {
                    if let Some(current_hash) = get_content_hash(root, &s.file, s.line_start, s.line_end) {
                        if &current_hash != expected_hash {
                            return false;
                        }
                    }
                }
            }

            true
        });

        let removed = initial_count - self.items.len();
        if removed > 0 {
            info!(removed, "Cleaned up expired/invalidated suppressions");
        }
    }
}

/// Get a hash of specific lines in a file for change detection
fn get_content_hash(repo_root: &Path, file: &str, line_start: u32, line_end: u32) -> Option<String> {
    let file_path = repo_root.join(file);

    let content = fs::read_to_string(&file_path).ok()?;
    let lines: Vec<&str> = content.lines().collect();

    // Extract the relevant lines (1-indexed to 0-indexed)
    let start = (line_start.saturating_sub(1)) as usize;
    let end = (line_end as usize).min(lines.len());

    if start >= lines.len() {
        return None;
    }

    let relevant_lines = &lines[start..end];
    let combined = relevant_lines.join("\n");

    // Simple hash using the content
    Some(format!("{:x}", md5::compute(combined.as_bytes())))
}

/// Create a suppression from a rejected review finding
#[allow(clippy::too_many_arguments)]
pub fn create_suppression(
    id: &str,
    file: &str,
    line_start: u32,
    line_end: u32,
    finding_type: Option<&str>,
    reason: &str,
    suppressed_by: &str,
    expires_days: Option<u32>,
    repo_root: Option<&Path>,
) -> Suppression {
    let content_hash = repo_root.and_then(|root| get_content_hash(root, file, line_start, line_end));

    let expires = expires_days.map(|days| {
        Utc::now() + chrono::Duration::days(days as i64)
    });

    Suppression {
        id: id.to_string(),
        file: file.to_string(),
        line_start,
        line_end,
        finding_type: finding_type.map(|s| s.to_string()),
        pattern: None,
        reason: reason.to_string(),
        suppressed_by: suppressed_by.to_string(),
        suppressed_at: Utc::now(),
        content_hash,
        expires,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_suppression_save_load() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("suppressions.yml");

        let mut suppressions = Suppressions::default();
        suppressions.add(Suppression {
            id: "S001".to_string(),
            file: "src/main.rs".to_string(),
            line_start: 10,
            line_end: 20,
            finding_type: Some("logic".to_string()),
            pattern: None,
            reason: "Test suppression".to_string(),
            suppressed_by: "test".to_string(),
            suppressed_at: Utc::now(),
            content_hash: None,
            expires: None,
        });

        suppressions.save(&path).unwrap();

        let loaded = Suppressions::load(&path).unwrap();
        assert_eq!(loaded.items.len(), 1);
        assert_eq!(loaded.items[0].id, "S001");
    }

    #[test]
    fn test_is_suppressed() {
        let mut suppressions = Suppressions::default();
        suppressions.add(Suppression {
            id: "S001".to_string(),
            file: "src/main.rs".to_string(),
            line_start: 10,
            line_end: 20,
            finding_type: Some("logic".to_string()),
            pattern: None,
            reason: "Test".to_string(),
            suppressed_by: "test".to_string(),
            suppressed_at: Utc::now(),
            content_hash: None,
            expires: None,
        });

        // Should be suppressed (overlapping lines, matching type)
        assert!(suppressions.is_suppressed("src/main.rs", 15, 18, "logic", "Some issue", None).is_some());

        // Should not be suppressed (different file)
        assert!(suppressions.is_suppressed("src/other.rs", 15, 18, "logic", "Some issue", None).is_none());

        // Should not be suppressed (non-overlapping lines)
        assert!(suppressions.is_suppressed("src/main.rs", 25, 30, "logic", "Some issue", None).is_none());

        // Should not be suppressed (different type)
        assert!(suppressions.is_suppressed("src/main.rs", 15, 18, "security", "Some issue", None).is_none());
    }
}
