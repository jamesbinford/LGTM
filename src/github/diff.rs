use std::process::Command;

use anyhow::{Context, Result};
use tracing::debug;

/// Extract diff between two refs using git
pub fn extract_diff(base_ref: &str, head_ref: &str) -> Result<String> {
    let output = Command::new("git")
        .args(["diff", base_ref, head_ref])
        .output()
        .context("Failed to run git diff")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("git diff failed: {}", stderr);
    }

    let diff = String::from_utf8(output.stdout).context("Invalid UTF-8 in diff")?;

    debug!(bytes = diff.len(), "Extracted diff");

    Ok(diff)
}

/// Extract diff for staged changes
#[allow(dead_code)]
pub fn extract_staged_diff() -> Result<String> {
    let output = Command::new("git")
        .args(["diff", "--cached"])
        .output()
        .context("Failed to run git diff --cached")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("git diff failed: {}", stderr);
    }

    let diff = String::from_utf8(output.stdout).context("Invalid UTF-8 in diff")?;

    Ok(diff)
}

/// Parse a unified diff to extract file-level information
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct DiffFile {
    pub path: String,
    pub old_path: Option<String>,
    pub hunks: Vec<DiffHunk>,
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct DiffHunk {
    pub old_start: u32,
    pub old_count: u32,
    pub new_start: u32,
    pub new_count: u32,
    pub content: String,
}

/// Parse a unified diff into structured data
#[allow(dead_code)]
pub fn parse_diff(diff: &str) -> Vec<DiffFile> {
    let mut files = Vec::new();
    let mut current_file: Option<DiffFile> = None;
    let mut current_hunk: Option<DiffHunk> = None;
    let mut hunk_content = String::new();

    for line in diff.lines() {
        if line.starts_with("diff --git") {
            // Save previous file if exists
            if let Some(mut file) = current_file.take() {
                if let Some(mut hunk) = current_hunk.take() {
                    hunk.content = hunk_content.clone();
                    file.hunks.push(hunk);
                }
                files.push(file);
            }
            hunk_content.clear();

            // Parse file path from "diff --git a/path b/path"
            let parts: Vec<&str> = line.split(' ').collect();
            if parts.len() >= 4 {
                let path = parts[3].trim_start_matches("b/").to_string();
                current_file = Some(DiffFile {
                    path,
                    old_path: None,
                    hunks: Vec::new(),
                });
            }
        } else if line.starts_with("@@") {
            // Save previous hunk if exists
            if let Some(ref mut file) = current_file {
                if let Some(mut hunk) = current_hunk.take() {
                    hunk.content = hunk_content.clone();
                    file.hunks.push(hunk);
                }
            }
            hunk_content.clear();

            // Parse hunk header "@@ -old_start,old_count +new_start,new_count @@"
            if let Some(hunk) = parse_hunk_header(line) {
                current_hunk = Some(hunk);
            }
        } else if current_hunk.is_some() {
            hunk_content.push_str(line);
            hunk_content.push('\n');
        }
    }

    // Save final file
    if let Some(mut file) = current_file {
        if let Some(mut hunk) = current_hunk {
            hunk.content = hunk_content;
            file.hunks.push(hunk);
        }
        files.push(file);
    }

    files
}

#[allow(dead_code)]
fn parse_hunk_header(line: &str) -> Option<DiffHunk> {
    // Format: @@ -old_start,old_count +new_start,new_count @@
    let line = line.trim_start_matches("@@ ");
    let parts: Vec<&str> = line.split(" @@").next()?.split(' ').collect();

    if parts.len() < 2 {
        return None;
    }

    let old_parts: Vec<&str> = parts[0].trim_start_matches('-').split(',').collect();
    let new_parts: Vec<&str> = parts[1].trim_start_matches('+').split(',').collect();

    let old_start = old_parts.first()?.parse().ok()?;
    let old_count = old_parts.get(1).and_then(|s| s.parse().ok()).unwrap_or(1);
    let new_start = new_parts.first()?.parse().ok()?;
    let new_count = new_parts.get(1).and_then(|s| s.parse().ok()).unwrap_or(1);

    Some(DiffHunk {
        old_start,
        old_count,
        new_start,
        new_count,
        content: String::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_diff() {
        let diff = r#"diff --git a/src/main.rs b/src/main.rs
index 1234567..abcdefg 100644
--- a/src/main.rs
+++ b/src/main.rs
@@ -1,3 +1,4 @@
 fn main() {
+    println!("Hello");
     println!("World");
 }
"#;

        let files = parse_diff(diff);
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, "src/main.rs");
        assert_eq!(files[0].hunks.len(), 1);
        assert_eq!(files[0].hunks[0].new_start, 1);
        assert_eq!(files[0].hunks[0].new_count, 4);
    }

    #[test]
    fn test_parse_hunk_header() {
        let hunk = parse_hunk_header("@@ -10,5 +10,7 @@ fn example()").unwrap();
        assert_eq!(hunk.old_start, 10);
        assert_eq!(hunk.old_count, 5);
        assert_eq!(hunk.new_start, 10);
        assert_eq!(hunk.new_count, 7);
    }
}
