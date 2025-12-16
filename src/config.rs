use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use tracing::info;

/// Main configuration structure
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub review: ReviewConfig,
    pub severity_thresholds: SeverityThresholds,
    pub auto_rules: Vec<AutoRule>,
    pub staleness: StalenessConfig,
    pub notifications: NotificationsConfig,
    pub models: ModelsConfig,
}

/// Review file filtering configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ReviewConfig {
    pub include_patterns: Vec<String>,
    pub exclude_patterns: Vec<String>,
}

impl Default for ReviewConfig {
    fn default() -> Self {
        Self {
            include_patterns: vec![
                "**/*.rs".to_string(),
                "**/*.py".to_string(),
                "**/*.ts".to_string(),
                "**/*.tsx".to_string(),
                "**/*.js".to_string(),
                "**/*.jsx".to_string(),
                "**/*.go".to_string(),
            ],
            exclude_patterns: vec![
                "**/test_*.py".to_string(),
                "**/*_test.go".to_string(),
                "**/*.test.ts".to_string(),
                "**/*.spec.ts".to_string(),
                "**/node_modules/**".to_string(),
                "**/vendor/**".to_string(),
                "**/target/**".to_string(),
            ],
        }
    }
}

/// Severity threshold configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SeverityThresholds {
    /// Severities that block merge if undecided
    pub blocking: Vec<String>,
    /// Severities that warn but allow merge
    pub warning: Vec<String>,
}

impl Default for SeverityThresholds {
    fn default() -> Self {
        Self {
            blocking: vec!["critical".to_string()],
            warning: vec!["high".to_string(), "medium".to_string()],
        }
    }
}

/// Auto-rule for automatic decisions
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutoRule {
    pub condition: String,
    pub action: AutoAction,
    pub reason: String,
}

/// Automatic action to take
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AutoAction {
    AutoAccept,
    AutoDismiss,
    AutoDefer,
}

/// Staleness handling configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct StalenessConfig {
    pub warn_after_days: u32,
    pub escalate_after_days: u32,
}

impl Default for StalenessConfig {
    fn default() -> Self {
        Self {
            warn_after_days: 3,
            escalate_after_days: 7,
        }
    }
}

/// Notifications configuration
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct NotificationsConfig {
    pub slack: SlackConfig,
}

/// Slack notification configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SlackConfig {
    pub enabled: bool,
    pub webhook_url: Option<String>,
    pub channel: Option<String>,
    pub on_critical: bool,
    pub on_new_review: bool,
}

impl Default for SlackConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            webhook_url: None,
            channel: None,
            on_critical: true,
            on_new_review: false,
        }
    }
}

/// Model configuration
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ModelsConfig {
    pub codex: CodexModelConfig,
}

/// Codex (OpenAI) model configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct CodexModelConfig {
    pub model: String,
    pub temperature: f32,
}

impl Default for CodexModelConfig {
    fn default() -> Self {
        Self {
            model: "gpt-4o".to_string(),
            temperature: 0.1,
        }
    }
}

impl Config {
    /// Load configuration from a YAML file
    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();

        if !path.exists() {
            info!(path = %path.display(), "Config file not found, using defaults");
            return Ok(Self::default());
        }

        let content = fs::read_to_string(path)
            .with_context(|| format!("Failed to read config file: {}", path.display()))?;

        let config: Config = serde_yaml::from_str(&content)
            .with_context(|| format!("Failed to parse config file: {}", path.display()))?;

        info!(path = %path.display(), "Loaded configuration");

        Ok(config)
    }

    /// Load configuration from the default location (.ai-review/config.yml)
    pub fn load_default() -> Result<Self> {
        Self::load(".ai-review/config.yml")
    }

    /// Check if a file path should be included in review
    pub fn should_review_file(&self, file_path: &str) -> bool {
        // Check exclude patterns first
        for pattern in &self.review.exclude_patterns {
            if let Ok(glob_pattern) = glob::Pattern::new(pattern) {
                if glob_pattern.matches(file_path) {
                    return false;
                }
            }
        }

        // Check include patterns
        for pattern in &self.review.include_patterns {
            if let Ok(glob_pattern) = glob::Pattern::new(pattern) {
                if glob_pattern.matches(file_path) {
                    return true;
                }
            }
        }

        // Default: exclude if no include pattern matched
        false
    }

    /// Check if a severity level is blocking
    pub fn is_blocking_severity(&self, severity: &str) -> bool {
        self.severity_thresholds
            .blocking
            .iter()
            .any(|s| s.eq_ignore_ascii_case(severity))
    }

    /// Check if a severity level is warning
    pub fn is_warning_severity(&self, severity: &str) -> bool {
        self.severity_thresholds
            .warning
            .iter()
            .any(|s| s.eq_ignore_ascii_case(severity))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = Config::default();
        assert!(!config.review.include_patterns.is_empty());
        assert!(!config.review.exclude_patterns.is_empty());
        assert_eq!(config.staleness.warn_after_days, 3);
    }

    #[test]
    fn test_should_review_file() {
        let config = Config::default();

        // Should include
        assert!(config.should_review_file("src/main.rs"));
        assert!(config.should_review_file("lib/utils.py"));
        assert!(config.should_review_file("components/Button.tsx"));

        // Should exclude
        assert!(!config.should_review_file("test_utils.py"));
        assert!(!config.should_review_file("node_modules/lodash/index.js"));
        assert!(!config.should_review_file("target/debug/main"));
    }

    #[test]
    fn test_severity_checks() {
        let config = Config::default();

        assert!(config.is_blocking_severity("critical"));
        assert!(config.is_blocking_severity("CRITICAL"));
        assert!(!config.is_blocking_severity("high"));

        assert!(config.is_warning_severity("high"));
        assert!(config.is_warning_severity("medium"));
        assert!(!config.is_warning_severity("low"));
    }

    #[test]
    fn test_parse_yaml() {
        let yaml = r#"
review:
  include_patterns:
    - "**/*.rs"
  exclude_patterns:
    - "**/test_*.rs"

severity_thresholds:
  blocking:
    - critical
    - high

staleness:
  warn_after_days: 5
  escalate_after_days: 10
"#;

        let config: Config = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.review.include_patterns.len(), 1);
        assert_eq!(config.severity_thresholds.blocking.len(), 2);
        assert_eq!(config.staleness.warn_after_days, 5);
    }
}
