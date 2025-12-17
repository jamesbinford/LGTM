use std::fs;
use std::path::Path;

use anyhow::Result;
use tracing::{debug, info};

/// A rejected finding extracted from review markdown files
#[derive(Debug, Clone)]
pub struct RejectedFinding {
    pub file: String,
    pub line_start: u32,
    pub line_end: u32,
    pub finding_type: String,
    pub description: String,
    pub reason: String,
}

/// Collection of rejected findings from review files
#[derive(Debug, Clone, Default)]
pub struct Rejections {
    pub items: Vec<RejectedFinding>,
}

impl Rejections {
    /// Load rejections by parsing markdown files in lgtm-reviews/
    pub fn load_from_reviews(reviews_dir: impl AsRef<Path>) -> Result<Self> {
        let reviews_dir = reviews_dir.as_ref();
        let mut items = Vec::new();

        if !reviews_dir.exists() {
            debug!(path = %reviews_dir.display(), "Reviews directory not found");
            return Ok(Self::default());
        }

        // Read all .md files in the directory
        for entry in fs::read_dir(reviews_dir)? {
            let entry = entry?;
            let path = entry.path();

            if path.extension().map_or(false, |ext| ext == "md") {
                if let Ok(content) = fs::read_to_string(&path) {
                    let findings = parse_rejections(&content);
                    items.extend(findings);
                }
            }
        }

        info!(count = items.len(), "Loaded rejected findings from reviews");

        Ok(Self { items })
    }

    /// Load from the default location
    pub fn load_default() -> Result<Self> {
        Self::load_from_reviews("lgtm-reviews")
    }

    /// Generate a prompt snippet listing rejected findings for OpenAI
    pub fn to_prompt(&self) -> String {
        if self.items.is_empty() {
            return String::new();
        }

        let mut prompt = String::from("\n\nPreviously reviewed and REJECTED findings (DO NOT report these again):\n");

        for r in &self.items {
            prompt.push_str(&format!(
                "- {} (lines {}-{}) [{}]: {} [Rejection reason: {}]\n",
                r.file, r.line_start, r.line_end, r.finding_type, r.description, r.reason
            ));
        }

        prompt
    }
}

/// Parse a review markdown file and extract rejected findings
fn parse_rejections(content: &str) -> Vec<RejectedFinding> {
    let mut findings = Vec::new();
    let lines: Vec<&str> = content.lines().collect();

    let mut i = 0;
    while i < lines.len() {
        // Look for suggestion headers like "#### 🟠 HIGH `S001` - Logic"
        if lines[i].starts_with("####") && lines[i].contains('`') {
            // Extract finding type from header
            let finding_type = extract_finding_type(lines[i]);

            // Look for file info on next line: "**File:** `src/foo.rs` (lines 10-20)"
            let mut file = String::new();
            let mut line_start = 0u32;
            let mut line_end = 0u32;

            if i + 1 < lines.len() && lines[i + 1].starts_with("**File:**") {
                if let Some((f, ls, le)) = parse_file_line(lines[i + 1]) {
                    file = f;
                    line_start = ls;
                    line_end = le;
                }
            }

            // Collect description lines until we hit a code block or section marker
            let mut description = String::new();
            let mut j = i + 2;
            while j < lines.len() {
                let line = lines[j];
                if line.starts_with("**Proposed fix:**")
                    || line.starts_with("**Decision:**")
                    || line.starts_with("####")
                    || line.starts_with("---")
                {
                    break;
                }
                if !line.is_empty() && !line.starts_with("**File:**") {
                    if !description.is_empty() {
                        description.push(' ');
                    }
                    description.push_str(line);
                }
                j += 1;
            }

            // Look for rejection marker: "**Decision:** ❌ REJECTED"
            while j < lines.len() {
                if lines[j].contains("❌ REJECTED") {
                    // Next line(s) contain the reason (quoted with >)
                    let mut reason = String::new();
                    let mut k = j + 1;
                    while k < lines.len() && lines[k].starts_with('>') {
                        let r = lines[k].trim_start_matches('>').trim();
                        if !reason.is_empty() {
                            reason.push(' ');
                        }
                        reason.push_str(r);
                        k += 1;
                    }

                    if !file.is_empty() {
                        findings.push(RejectedFinding {
                            file,
                            line_start,
                            line_end,
                            finding_type: finding_type.clone(),
                            description: description.trim().to_string(),
                            reason,
                        });
                    }
                    break;
                }
                if lines[j].starts_with("####") || lines[j].starts_with("---") {
                    break;
                }
                j += 1;
            }

            i = j;
        } else {
            i += 1;
        }
    }

    findings
}

/// Extract finding type from header like "#### 🟠 HIGH `S001` - Logic"
fn extract_finding_type(header: &str) -> String {
    if let Some(dash_pos) = header.rfind(" - ") {
        header[dash_pos + 3..].trim().to_lowercase()
    } else {
        "unknown".to_string()
    }
}

/// Parse file line like "**File:** `src/foo.rs` (lines 10-20)"
fn parse_file_line(line: &str) -> Option<(String, u32, u32)> {
    // Extract file path between backticks
    let start = line.find('`')? + 1;
    let end = line[start..].find('`')? + start;
    let file = line[start..end].to_string();

    // Extract line numbers from "(lines X-Y)"
    let lines_start = line.find("(lines ")? + 7;
    let lines_end = line[lines_start..].find(')')? + lines_start;
    let lines_part = &line[lines_start..lines_end];

    let parts: Vec<&str> = lines_part.split('-').collect();
    if parts.len() == 2 {
        let line_start: u32 = parts[0].parse().ok()?;
        let line_end: u32 = parts[1].parse().ok()?;
        return Some((file, line_start, line_end));
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_rejections() {
        let content = r#"## AI Code Review Summary

#### 🟠 HIGH `S001` - Security
**File:** `src/main.rs` (lines 10-15)

This is a security issue description.

**Proposed fix:**
```
Fix the issue
```

**Decision:** ❌ REJECTED by claude
> This is intentional behavior for testing purposes.

---

#### 🟢 LOW `S002` - Style
**File:** `src/lib.rs` (lines 20-25)

This is a style issue.

**Proposed fix:**
```
Format better
```

---
"#;

        let findings = parse_rejections(content);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].file, "src/main.rs");
        assert_eq!(findings[0].line_start, 10);
        assert_eq!(findings[0].line_end, 15);
        assert_eq!(findings[0].finding_type, "security");
        assert!(findings[0].reason.contains("intentional behavior"));
    }

    #[test]
    fn test_parse_file_line() {
        let line = "**File:** `src/adapters/codex.rs` (lines 195-200)";
        let result = parse_file_line(line);
        assert!(result.is_some());
        let (file, start, end) = result.unwrap();
        assert_eq!(file, "src/adapters/codex.rs");
        assert_eq!(start, 195);
        assert_eq!(end, 200);
    }
}
