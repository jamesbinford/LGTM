use anyhow::{Context, Result};
use reqwest::Client;
use serde::Serialize;
use tracing::{debug, info, warn};

use crate::config::SlackConfig;
use crate::models::{Review, Severity};

/// Notification service for sending alerts
pub struct NotificationService {
    client: Client,
    slack: Option<SlackNotifier>,
}

impl NotificationService {
    pub fn new(slack_config: &SlackConfig) -> Self {
        let slack = if slack_config.enabled {
            slack_config
                .webhook_url
                .as_ref()
                .map(|url| SlackNotifier::new(url.clone(), slack_config.channel.clone()))
        } else {
            None
        };

        Self {
            client: Client::new(),
            slack,
        }
    }

    /// Send notification for a new review
    pub async fn notify_new_review(&self, review: &Review) -> Result<()> {
        if let Some(ref slack) = self.slack {
            slack.notify_new_review(&self.client, review).await?;
        }
        Ok(())
    }

    /// Send notification for critical issues found
    pub async fn notify_critical(&self, review: &Review) -> Result<()> {
        let critical_count = review
            .suggestions
            .iter()
            .filter(|s| s.suggestion.severity == Severity::Critical)
            .count();

        if critical_count == 0 {
            return Ok(());
        }

        if let Some(ref slack) = self.slack {
            slack
                .notify_critical(&self.client, review, critical_count)
                .await?;
        }

        Ok(())
    }

    /// Send notification for stale reviews
    pub async fn notify_stale(&self, review: &Review, age_days: i64) -> Result<()> {
        if let Some(ref slack) = self.slack {
            slack
                .notify_stale(&self.client, review, age_days)
                .await?;
        }
        Ok(())
    }
}

/// Slack webhook notifier
struct SlackNotifier {
    webhook_url: String,
    channel: Option<String>,
}

#[derive(Serialize)]
struct SlackMessage {
    #[serde(skip_serializing_if = "Option::is_none")]
    channel: Option<String>,
    text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    blocks: Option<Vec<SlackBlock>>,
}

#[derive(Serialize)]
struct SlackBlock {
    #[serde(rename = "type")]
    block_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    text: Option<SlackText>,
}

#[derive(Serialize)]
struct SlackText {
    #[serde(rename = "type")]
    text_type: String,
    text: String,
}

impl SlackNotifier {
    fn new(webhook_url: String, channel: Option<String>) -> Self {
        Self {
            webhook_url,
            channel,
        }
    }

    async fn send(&self, client: &Client, message: SlackMessage) -> Result<()> {
        debug!("Sending Slack notification");

        let response = client
            .post(&self.webhook_url)
            .json(&message)
            .send()
            .await
            .context("Failed to send Slack notification")?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            warn!(status = %status, body = %body, "Slack notification failed");
            anyhow::bail!("Slack webhook returned error: {} - {}", status, body);
        }

        info!("Slack notification sent");
        Ok(())
    }

    async fn notify_new_review(&self, client: &Client, review: &Review) -> Result<()> {
        let critical = review
            .suggestions
            .iter()
            .filter(|s| s.suggestion.severity == Severity::Critical)
            .count();
        let high = review
            .suggestions
            .iter()
            .filter(|s| s.suggestion.severity == Severity::High)
            .count();

        let text = format!(
            "🔍 New AI Code Review for PR #{} in `{}`\n\
             📊 {} suggestions ({} critical, {} high)",
            review.pr_number,
            review.repo,
            review.suggestions.len(),
            critical,
            high
        );

        let message = SlackMessage {
            channel: self.channel.clone(),
            text: text.clone(),
            blocks: Some(vec![
                SlackBlock {
                    block_type: "section".to_string(),
                    text: Some(SlackText {
                        text_type: "mrkdwn".to_string(),
                        text,
                    }),
                },
                SlackBlock {
                    block_type: "section".to_string(),
                    text: Some(SlackText {
                        text_type: "mrkdwn".to_string(),
                        text: format!("Review ID: `{}`", review.id),
                    }),
                },
            ]),
        };

        self.send(client, message).await
    }

    async fn notify_critical(&self, client: &Client, review: &Review, count: usize) -> Result<()> {
        let text = format!(
            "🚨 *CRITICAL* issues found in PR #{} in `{}`\n\
             Found {} critical security/logic issues requiring immediate attention.",
            review.pr_number, review.repo, count
        );

        let issues: Vec<String> = review
            .suggestions
            .iter()
            .filter(|s| s.suggestion.severity == Severity::Critical)
            .map(|s| {
                format!(
                    "• `{}` in `{}`: {}",
                    s.suggestion.id, s.suggestion.location.file, s.suggestion.description
                )
            })
            .collect();

        let message = SlackMessage {
            channel: self.channel.clone(),
            text: text.clone(),
            blocks: Some(vec![
                SlackBlock {
                    block_type: "section".to_string(),
                    text: Some(SlackText {
                        text_type: "mrkdwn".to_string(),
                        text,
                    }),
                },
                SlackBlock {
                    block_type: "section".to_string(),
                    text: Some(SlackText {
                        text_type: "mrkdwn".to_string(),
                        text: issues.join("\n"),
                    }),
                },
            ]),
        };

        self.send(client, message).await
    }

    async fn notify_stale(&self, client: &Client, review: &Review, age_days: i64) -> Result<()> {
        let pending = review.pending_suggestions().len();

        let text = format!(
            "⏰ Stale review alert: PR #{} in `{}`\n\
             This review has been pending for {} days with {} undecided suggestions.",
            review.pr_number, review.repo, age_days, pending
        );

        let message = SlackMessage {
            channel: self.channel.clone(),
            text: text.clone(),
            blocks: Some(vec![SlackBlock {
                block_type: "section".to_string(),
                text: Some(SlackText {
                    text_type: "mrkdwn".to_string(),
                    text,
                }),
            }]),
        };

        self.send(client, message).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::SlackConfig;

    #[test]
    fn test_notification_service_disabled() {
        let config = SlackConfig {
            enabled: false,
            ..Default::default()
        };

        let service = NotificationService::new(&config);
        assert!(service.slack.is_none());
    }

    #[test]
    fn test_notification_service_enabled() {
        let config = SlackConfig {
            enabled: true,
            webhook_url: Some("https://hooks.slack.com/test".to_string()),
            channel: Some("#reviews".to_string()),
            on_critical: true,
            on_new_review: true,
        };

        let service = NotificationService::new(&config);
        assert!(service.slack.is_some());
    }
}
