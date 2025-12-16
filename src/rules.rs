use chrono::{DateTime, Utc};
use tracing::{debug, info};

use crate::config::{AutoAction, AutoRule, Config};
use crate::models::{
    DecisionRecord, HumanDecision, Review, Severity, SuggestionItem, SuggestionType,
};

/// Rules engine for automatic decisions
pub struct RulesEngine {
    rules: Vec<AutoRule>,
}

impl RulesEngine {
    pub fn new(rules: Vec<AutoRule>) -> Self {
        Self { rules }
    }

    pub fn from_config(config: &Config) -> Self {
        Self::new(config.auto_rules.clone())
    }

    /// Apply auto-rules to a review, returning the number of auto-decisions made
    pub fn apply(&self, review: &mut Review) -> usize {
        let mut count = 0;

        for item in &mut review.suggestions {
            if item.decision.is_some() {
                continue; // Already decided
            }

            if let Some((action, reason)) = self.evaluate_rules(item, review.created_at) {
                let decision = match action {
                    AutoAction::AutoAccept => HumanDecision::Accepted,
                    AutoAction::AutoDismiss => HumanDecision::Rejected,
                    AutoAction::AutoDefer => HumanDecision::Deferred,
                };

                item.decision = Some(DecisionRecord {
                    suggestion_id: item.suggestion.id.clone(),
                    decision,
                    reason: Some(format!("[Auto] {}", reason)),
                    decided_by: "auto-rules".to_string(),
                    decided_at: Utc::now(),
                });

                info!(
                    suggestion_id = %item.suggestion.id,
                    action = ?action,
                    "Auto-rule applied"
                );

                count += 1;
            }
        }

        count
    }

    fn evaluate_rules(
        &self,
        item: &SuggestionItem,
        created_at: DateTime<Utc>,
    ) -> Option<(AutoAction, String)> {
        let context = RuleContext::from_suggestion(item, created_at);

        for rule in &self.rules {
            if self.matches_condition(&rule.condition, &context) {
                debug!(
                    condition = %rule.condition,
                    suggestion_id = %item.suggestion.id,
                    "Rule matched"
                );
                return Some((rule.action, rule.reason.clone()));
            }
        }

        None
    }

    fn matches_condition(&self, condition: &str, ctx: &RuleContext) -> bool {
        // Simple expression parser for conditions like:
        // "severity == 'low' AND type == 'style' AND age_days > 14"

        let parts: Vec<&str> = condition.split(" AND ").collect();

        for part in parts {
            if !self.evaluate_expression(part.trim(), ctx) {
                return false;
            }
        }

        true
    }

    fn evaluate_expression(&self, expr: &str, ctx: &RuleContext) -> bool {
        // Parse expressions like "field == 'value'" or "field > 0.95"
        let expr = expr.trim();

        // Handle equality checks
        if expr.contains("==") {
            let parts: Vec<&str> = expr.split("==").collect();
            if parts.len() != 2 {
                return false;
            }
            let field = parts[0].trim();
            let value = parts[1].trim().trim_matches('\'').trim_matches('"');
            return self.get_field_value(field, ctx) == value;
        }

        // Handle greater than
        if expr.contains(">") && !expr.contains(">=") {
            let parts: Vec<&str> = expr.split('>').collect();
            if parts.len() != 2 {
                return false;
            }
            let field = parts[0].trim();
            let threshold: f64 = match parts[1].trim().parse() {
                Ok(v) => v,
                Err(_) => return false,
            };
            let field_value: f64 = match self.get_field_value(field, ctx).parse() {
                Ok(v) => v,
                Err(_) => return false,
            };
            return field_value > threshold;
        }

        // Handle greater than or equal
        if expr.contains(">=") {
            let parts: Vec<&str> = expr.split(">=").collect();
            if parts.len() != 2 {
                return false;
            }
            let field = parts[0].trim();
            let threshold: f64 = match parts[1].trim().parse() {
                Ok(v) => v,
                Err(_) => return false,
            };
            let field_value: f64 = match self.get_field_value(field, ctx).parse() {
                Ok(v) => v,
                Err(_) => return false,
            };
            return field_value >= threshold;
        }

        // Handle less than
        if expr.contains("<") && !expr.contains("<=") {
            let parts: Vec<&str> = expr.split('<').collect();
            if parts.len() != 2 {
                return false;
            }
            let field = parts[0].trim();
            let threshold: f64 = match parts[1].trim().parse() {
                Ok(v) => v,
                Err(_) => return false,
            };
            let field_value: f64 = match self.get_field_value(field, ctx).parse() {
                Ok(v) => v,
                Err(_) => return false,
            };
            return field_value < threshold;
        }

        false
    }

    fn get_field_value(&self, field: &str, ctx: &RuleContext) -> String {
        match field {
            "severity" => ctx.severity.clone(),
            "type" => ctx.suggestion_type.clone(),
            "age_days" => ctx.age_days.to_string(),
            "file_path" => ctx.file_path.clone(),
            _ => String::new(),
        }
    }
}

/// Context extracted from a suggestion for rule evaluation
struct RuleContext {
    severity: String,
    suggestion_type: String,
    age_days: i64,
    file_path: String,
}

impl RuleContext {
    fn from_suggestion(item: &SuggestionItem, created_at: DateTime<Utc>) -> Self {
        let age_days = Utc::now().signed_duration_since(created_at).num_days();

        Self {
            severity: severity_to_string(item.suggestion.severity),
            suggestion_type: type_to_string(item.suggestion.suggestion_type),
            age_days,
            file_path: item.suggestion.location.file.clone(),
        }
    }
}

fn severity_to_string(s: Severity) -> String {
    match s {
        Severity::Critical => "critical",
        Severity::High => "high",
        Severity::Medium => "medium",
        Severity::Low => "low",
    }
    .to_string()
}

fn type_to_string(t: SuggestionType) -> String {
    match t {
        SuggestionType::Security => "security",
        SuggestionType::Performance => "performance",
        SuggestionType::Style => "style",
        SuggestionType::Logic => "logic",
        SuggestionType::Documentation => "documentation",
    }
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{Location, Suggestion};

    fn make_suggestion(severity: Severity, stype: SuggestionType) -> SuggestionItem {
        SuggestionItem {
            suggestion: Suggestion {
                id: "S001".to_string(),
                suggestion_type: stype,
                severity,
                location: Location {
                    file: "src/main.rs".to_string(),
                    line_start: 10,
                    line_end: 15,
                },
                description: "Test suggestion".to_string(),
                proposed_fix: None,
            },
            decision: None,
        }
    }

    #[test]
    fn test_severity_match() {
        let rules = vec![AutoRule {
            condition: "severity == 'low'".to_string(),
            action: AutoAction::AutoDismiss,
            reason: "Low severity auto-dismissed".to_string(),
        }];

        let engine = RulesEngine::new(rules);
        let mut item = make_suggestion(Severity::Low, SuggestionType::Style);
        let created_at = Utc::now();

        let result = engine.evaluate_rules(&item, created_at);
        assert!(result.is_some());
        assert_eq!(result.unwrap().0, AutoAction::AutoDismiss);

        // High severity should not match
        item.suggestion.severity = Severity::High;
        let result = engine.evaluate_rules(&item, created_at);
        assert!(result.is_none());
    }

    #[test]
    fn test_compound_condition() {
        let rules = vec![AutoRule {
            condition: "severity == 'low' AND type == 'style'".to_string(),
            action: AutoAction::AutoDismiss,
            reason: "Low style auto-dismissed".to_string(),
        }];

        let engine = RulesEngine::new(rules);
        let created_at = Utc::now();

        // Matches both conditions
        let item = make_suggestion(Severity::Low, SuggestionType::Style);
        let result = engine.evaluate_rules(&item, created_at);
        assert!(result.is_some());

        // Only matches severity
        let item = make_suggestion(Severity::Low, SuggestionType::Security);
        let result = engine.evaluate_rules(&item, created_at);
        assert!(result.is_none());
    }
}
