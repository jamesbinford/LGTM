use anyhow::{Context, Result};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use tracing::{debug, info, instrument};

use crate::models::{Recommendation, RecommendedAction, Suggestion};

/// Adapter for Claude recommendation evaluation
pub struct ClaudeAdapter {
    client: Client,
    api_key: String,
    model: String,
}

#[derive(Debug, Serialize)]
struct AnthropicRequest {
    model: String,
    max_tokens: u32,
    messages: Vec<AnthropicMessage>,
    system: String,
}

#[derive(Debug, Serialize)]
struct AnthropicMessage {
    role: String,
    content: String,
}

#[derive(Debug, Deserialize)]
struct AnthropicResponse {
    content: Vec<ContentBlock>,
}

#[derive(Debug, Deserialize)]
struct ContentBlock {
    text: String,
}

#[derive(Debug, Deserialize)]
struct ClaudeRecommendationOutput {
    recommendations: Vec<ClaudeRecommendation>,
}

#[derive(Debug, Deserialize)]
struct ClaudeRecommendation {
    suggestion_id: String,
    action: String,
    confidence: f64,
    rationale: String,
    modified_fix: Option<String>,
}

impl ClaudeAdapter {
    pub fn new(api_key: String) -> Self {
        Self {
            client: Client::new(),
            api_key,
            model: "claude-sonnet-4-20250514".to_string(),
        }
    }

    pub fn with_model(mut self, model: &str) -> Self {
        self.model = model.to_string();
        self
    }

    #[instrument(skip(self, suggestions, diff), fields(suggestion_count = suggestions.len()))]
    pub async fn recommend(
        &self,
        suggestions: &[Suggestion],
        diff: &str,
    ) -> Result<Vec<Recommendation>> {
        if suggestions.is_empty() {
            info!("No suggestions to evaluate");
            return Ok(Vec::new());
        }

        info!("Starting Claude recommendation evaluation");

        let system_prompt = self.build_system_prompt();
        let user_prompt = self.build_user_prompt(suggestions, diff);

        let request = AnthropicRequest {
            model: self.model.clone(),
            max_tokens: 4096,
            messages: vec![AnthropicMessage {
                role: "user".to_string(),
                content: user_prompt,
            }],
            system: system_prompt,
        };

        debug!("Sending request to Anthropic API");

        let response = self
            .client
            .post("https://api.anthropic.com/v1/messages")
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .json(&request)
            .send()
            .await
            .context("Failed to send request to Anthropic")?;

        let status = response.status();
        if !status.is_success() {
            let error_text = response.text().await.unwrap_or_default();
            anyhow::bail!("Anthropic API error ({}): {}", status, error_text);
        }

        let anthropic_response: AnthropicResponse = response
            .json()
            .await
            .context("Failed to parse Anthropic response")?;

        let content = anthropic_response
            .content
            .first()
            .map(|c| c.text.as_str())
            .unwrap_or("{}");

        // Extract JSON from response (Claude may include markdown formatting)
        let json_content = extract_json(content)?;

        let output: ClaudeRecommendationOutput =
            serde_json::from_str(&json_content).context("Failed to parse Claude recommendation output")?;

        let recommendations: Vec<Recommendation> = output
            .recommendations
            .into_iter()
            .map(|r| self.convert_recommendation(r))
            .collect();

        info!(count = recommendations.len(), "Claude evaluation complete");

        Ok(recommendations)
    }

    fn build_system_prompt(&self) -> String {
        r#"You are an expert code reviewer evaluating suggestions from another AI reviewer.

For each suggestion, you must decide:
- accept: The suggestion is valid and the proposed fix is correct
- reject: The suggestion is a false positive or not worth addressing
- modify: The suggestion is valid but the proposed fix needs improvement

Consider:
1. Is the identified issue real or a false positive?
2. Is the severity assessment accurate?
3. Is the proposed fix correct and complete?
4. Could the fix introduce new issues?
5. Is this worth the developer's time to address?

Be rigorous but practical. Reject suggestions that are:
- False positives or misunderstandings
- Too minor to be worth addressing
- Style preferences rather than real issues

Respond with valid JSON matching the schema exactly."#
            .to_string()
    }

    fn build_user_prompt(&self, suggestions: &[Suggestion], diff: &str) -> String {
        let suggestions_json = serde_json::to_string_pretty(suggestions).unwrap_or_default();

        format!(
            r#"Evaluate these code review suggestions:

## Suggestions to evaluate:
```json
{suggestions_json}
```

## Original diff for context:
```diff
{diff}
```

Respond with JSON in this exact format:
```json
{{
  "recommendations": [
    {{
      "suggestion_id": "S001",
      "action": "accept|reject|modify",
      "confidence": 0.95,
      "rationale": "Explanation of your decision",
      "modified_fix": null or "improved fix code if action is modify"
    }}
  ]
}}
```"#
        )
    }

    fn convert_recommendation(&self, r: ClaudeRecommendation) -> Recommendation {
        Recommendation {
            suggestion_id: r.suggestion_id,
            action: match r.action.as_str() {
                "accept" => RecommendedAction::Accept,
                "reject" => RecommendedAction::Reject,
                "modify" => RecommendedAction::Modify,
                _ => RecommendedAction::Reject,
            },
            confidence: r.confidence.clamp(0.0, 1.0),
            rationale: r.rationale,
            modified_fix: r.modified_fix,
        }
    }
}

/// Extract JSON from a response that may contain markdown code blocks
fn extract_json(content: &str) -> Result<String> {
    // Try to find JSON in code blocks first
    if let Some(start) = content.find("```json") {
        let json_start = start + 7;
        if let Some(end) = content[json_start..].find("```") {
            return Ok(content[json_start..json_start + end].trim().to_string());
        }
    }

    // Try to find raw JSON object
    if let Some(start) = content.find('{') {
        if let Some(end) = content.rfind('}') {
            return Ok(content[start..=end].to_string());
        }
    }

    // Return as-is and let serde handle errors
    Ok(content.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_json_from_code_block() {
        let content = r#"Here's my analysis:

```json
{"recommendations": []}
```

Done!"#;
        let result = extract_json(content).unwrap();
        assert_eq!(result, r#"{"recommendations": []}"#);
    }

    #[test]
    fn test_extract_json_raw() {
        let content = r#"{"recommendations": []}"#;
        let result = extract_json(content).unwrap();
        assert_eq!(result, r#"{"recommendations": []}"#);
    }

    #[test]
    fn test_convert_recommendation() {
        let adapter = ClaudeAdapter::new("test-key".to_string());
        let claude_rec = ClaudeRecommendation {
            suggestion_id: "S001".to_string(),
            action: "accept".to_string(),
            confidence: 0.95,
            rationale: "Valid security issue".to_string(),
            modified_fix: None,
        };

        let rec = adapter.convert_recommendation(claude_rec);

        assert_eq!(rec.suggestion_id, "S001");
        assert_eq!(rec.action, RecommendedAction::Accept);
        assert!((rec.confidence - 0.95).abs() < f64::EPSILON);
    }
}
