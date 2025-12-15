use anyhow::{Context, Result};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use tracing::{debug, info, instrument};

use crate::models::{Location, ReviewContext, Severity, Suggestion, SuggestionType};

/// Adapter for OpenAI Codex code review
pub struct CodexAdapter {
    client: Client,
    api_key: String,
    model: String,
}

#[derive(Debug, Serialize)]
struct ChatRequest {
    model: String,
    messages: Vec<Message>,
    response_format: ResponseFormat,
    temperature: f32,
}

#[derive(Debug, Serialize)]
struct Message {
    role: String,
    content: String,
}

#[derive(Debug, Serialize)]
struct ResponseFormat {
    #[serde(rename = "type")]
    format_type: String,
    json_schema: JsonSchema,
}

#[derive(Debug, Serialize)]
struct JsonSchema {
    name: String,
    strict: bool,
    schema: serde_json::Value,
}

#[derive(Debug, Deserialize)]
struct ChatResponse {
    choices: Vec<Choice>,
}

#[derive(Debug, Deserialize)]
struct Choice {
    message: ResponseMessage,
}

#[derive(Debug, Deserialize)]
struct ResponseMessage {
    content: String,
}

#[derive(Debug, Deserialize)]
struct CodexReviewOutput {
    suggestions: Vec<CodexSuggestion>,
}

#[derive(Debug, Deserialize)]
struct CodexSuggestion {
    id: String,
    #[serde(rename = "type")]
    suggestion_type: String,
    severity: String,
    location: CodexLocation,
    description: String,
    proposed_fix: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CodexLocation {
    file: String,
    line_start: u32,
    line_end: u32,
}

impl CodexAdapter {
    pub fn new(api_key: String) -> Self {
        Self {
            client: Client::new(),
            api_key,
            model: "gpt-4o".to_string(),
        }
    }

    pub fn with_model(mut self, model: &str) -> Self {
        self.model = model.to_string();
        self
    }

    #[instrument(skip(self, diff), fields(pr = context.pr_number, repo = %context.repo))]
    pub async fn review(&self, diff: &str, context: &ReviewContext) -> Result<Vec<Suggestion>> {
        info!("Starting Codex review");

        let system_prompt = self.build_system_prompt();
        let user_prompt = self.build_user_prompt(diff, context);

        let request = ChatRequest {
            model: self.model.clone(),
            messages: vec![
                Message {
                    role: "system".to_string(),
                    content: system_prompt,
                },
                Message {
                    role: "user".to_string(),
                    content: user_prompt,
                },
            ],
            response_format: self.build_response_format(),
            temperature: 0.1,
        };

        debug!("Sending request to OpenAI API");

        let response = self
            .client
            .post("https://api.openai.com/v1/chat/completions")
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&request)
            .send()
            .await
            .context("Failed to send request to OpenAI")?;

        let status = response.status();
        if !status.is_success() {
            let error_text = response.text().await.unwrap_or_default();
            anyhow::bail!("OpenAI API error ({}): {}", status, error_text);
        }

        let chat_response: ChatResponse = response
            .json()
            .await
            .context("Failed to parse OpenAI response")?;

        let content = chat_response
            .choices
            .first()
            .map(|c| c.message.content.as_str())
            .unwrap_or("{}");

        let output: CodexReviewOutput =
            serde_json::from_str(content).context("Failed to parse Codex review output")?;

        let suggestions: Vec<Suggestion> = output
            .suggestions
            .into_iter()
            .map(|s| self.convert_suggestion(s))
            .collect();

        info!(count = suggestions.len(), "Codex review complete");

        Ok(suggestions)
    }

    fn build_system_prompt(&self) -> String {
        r#"You are an expert code reviewer. Analyze the provided diff and identify issues in these categories:
- security: vulnerabilities, injection risks, authentication issues
- performance: inefficient algorithms, unnecessary allocations, N+1 queries
- logic: bugs, edge cases, incorrect behavior
- style: readability issues, naming, code organization (only significant issues)

For each issue, provide:
- A unique ID (S001, S002, etc.)
- The category (security, performance, logic, style)
- Severity (critical, high, medium, low)
- Exact file and line numbers
- Clear description of the problem
- A proposed fix (actual code when possible)

Focus on substantive issues. Ignore minor style preferences.
Only review the changed lines (+ lines in diff), not removed lines."#
            .to_string()
    }

    fn build_user_prompt(&self, diff: &str, context: &ReviewContext) -> String {
        let target = match context.pr_number {
            Some(pr) => format!("PR #{}", pr),
            None => format!("commit {}", &context.commit_sha[..7.min(context.commit_sha.len())]),
        };
        format!(
            "Review this diff from {} in {}:\n\n```diff\n{}\n```",
            target, context.repo, diff
        )
    }

    fn build_response_format(&self) -> ResponseFormat {
        ResponseFormat {
            format_type: "json_schema".to_string(),
            json_schema: JsonSchema {
                name: "code_review".to_string(),
                strict: true,
                schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "suggestions": {
                            "type": "array",
                            "items": {
                                "type": "object",
                                "properties": {
                                    "id": { "type": "string" },
                                    "type": { "type": "string", "enum": ["security", "performance", "logic", "style", "documentation"] },
                                    "severity": { "type": "string", "enum": ["critical", "high", "medium", "low"] },
                                    "location": {
                                        "type": "object",
                                        "properties": {
                                            "file": { "type": "string" },
                                            "line_start": { "type": "integer" },
                                            "line_end": { "type": "integer" }
                                        },
                                        "required": ["file", "line_start", "line_end"],
                                        "additionalProperties": false
                                    },
                                    "description": { "type": "string" },
                                    "proposed_fix": { "type": ["string", "null"] }
                                },
                                "required": ["id", "type", "severity", "location", "description", "proposed_fix"],
                                "additionalProperties": false
                            }
                        }
                    },
                    "required": ["suggestions"],
                    "additionalProperties": false
                }),
            },
        }
    }

    fn convert_suggestion(&self, s: CodexSuggestion) -> Suggestion {
        Suggestion {
            id: s.id,
            suggestion_type: match s.suggestion_type.as_str() {
                "security" => SuggestionType::Security,
                "performance" => SuggestionType::Performance,
                "style" => SuggestionType::Style,
                "logic" => SuggestionType::Logic,
                "documentation" => SuggestionType::Documentation,
                _ => SuggestionType::Logic,
            },
            severity: match s.severity.as_str() {
                "critical" => Severity::Critical,
                "high" => Severity::High,
                "medium" => Severity::Medium,
                _ => Severity::Low,
            },
            location: Location {
                file: s.location.file,
                line_start: s.location.line_start,
                line_end: s.location.line_end,
            },
            description: s.description,
            proposed_fix: s.proposed_fix,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_convert_suggestion() {
        let adapter = CodexAdapter::new("test-key".to_string());
        let codex_suggestion = CodexSuggestion {
            id: "S001".to_string(),
            suggestion_type: "security".to_string(),
            severity: "critical".to_string(),
            location: CodexLocation {
                file: "src/main.rs".to_string(),
                line_start: 10,
                line_end: 15,
            },
            description: "SQL injection vulnerability".to_string(),
            proposed_fix: Some("Use parameterized queries".to_string()),
        };

        let suggestion = adapter.convert_suggestion(codex_suggestion);

        assert_eq!(suggestion.id, "S001");
        assert_eq!(suggestion.suggestion_type, SuggestionType::Security);
        assert_eq!(suggestion.severity, Severity::Critical);
        assert_eq!(suggestion.location.file, "src/main.rs");
    }
}
