use anyhow::{Context, Result};
use octocrab::Octocrab;
use tracing::{debug, info};

/// GitHub API client for PR interactions
pub struct GitHubClient {
    client: Octocrab,
}

impl GitHubClient {
    /// Create a new GitHub client with the given token
    pub fn new(token: &str) -> Result<Self> {
        let client = Octocrab::builder()
            .personal_token(token.to_string())
            .build()
            .context("Failed to create GitHub client")?;

        Ok(Self { client })
    }

    /// Post a comment on a PR
    pub async fn post_comment(
        &self,
        owner: &str,
        repo: &str,
        pr_number: u64,
        body: &str,
    ) -> Result<u64> {
        info!(owner, repo, pr_number, "Posting PR comment");

        let comment = self
            .client
            .issues(owner, repo)
            .create_comment(pr_number, body)
            .await
            .context("Failed to post PR comment")?;

        debug!(comment_id = comment.id.0, "Comment posted");

        Ok(comment.id.0)
    }

    /// Update an existing comment
    pub async fn update_comment(
        &self,
        owner: &str,
        repo: &str,
        comment_id: u64,
        body: &str,
    ) -> Result<()> {
        info!(owner, repo, comment_id, "Updating PR comment");

        self.client
            .issues(owner, repo)
            .update_comment(comment_id.into(), body)
            .await
            .context("Failed to update PR comment")?;

        Ok(())
    }

    /// Get the diff for a PR
    pub async fn get_pr_diff(&self, owner: &str, repo: &str, pr_number: u64) -> Result<String> {
        info!(owner, repo, pr_number, "Fetching PR diff");

        // Use the pulls API to get diff
        let pr = self
            .client
            .pulls(owner, repo)
            .get(pr_number)
            .await
            .context("Failed to get PR")?;

        // Get the diff URL and fetch it
        let diff_url = pr.diff_url.context("PR has no diff URL")?;

        let response = reqwest::Client::new()
            .get(diff_url.as_str())
            .header("Accept", "application/vnd.github.v3.diff")
            .header(
                "Authorization",
                format!("Bearer {}", self.get_token()?),
            )
            .send()
            .await
            .context("Failed to fetch diff")?;

        let diff = response.text().await.context("Failed to read diff body")?;

        debug!(bytes = diff.len(), "Fetched PR diff");

        Ok(diff)
    }

    /// Get file contents at a specific commit
    pub async fn get_file_contents(
        &self,
        owner: &str,
        repo: &str,
        path: &str,
        commit_sha: &str,
    ) -> Result<String> {
        let content = self
            .client
            .repos(owner, repo)
            .get_content()
            .path(path)
            .r#ref(commit_sha)
            .send()
            .await
            .context("Failed to get file contents")?;

        match content.items.first() {
            Some(item) => {
                let decoded = item
                    .decoded_content()
                    .context("Failed to decode file content")?;
                Ok(decoded)
            }
            None => anyhow::bail!("File not found: {}", path),
        }
    }

    /// List files changed in a PR
    pub async fn list_pr_files(
        &self,
        owner: &str,
        repo: &str,
        pr_number: u64,
    ) -> Result<Vec<PrFile>> {
        let files = self
            .client
            .pulls(owner, repo)
            .list_files(pr_number)
            .await
            .context("Failed to list PR files")?;

        let result: Vec<PrFile> = files
            .items
            .into_iter()
            .map(|f| PrFile {
                filename: f.filename,
                status: format!("{:?}", f.status),
                additions: f.additions,
                deletions: f.deletions,
                changes: f.changes,
            })
            .collect();

        Ok(result)
    }

    fn get_token(&self) -> Result<String> {
        // This is a workaround since octocrab doesn't expose the token
        // In a real implementation, we'd store the token separately
        std::env::var("GITHUB_TOKEN").context("GITHUB_TOKEN not set")
    }
}

/// Information about a file changed in a PR
#[derive(Debug, Clone)]
pub struct PrFile {
    pub filename: String,
    pub status: String,
    pub additions: u64,
    pub deletions: u64,
    pub changes: u64,
}

/// Parse owner and repo from a repo string like "owner/repo"
#[allow(dead_code)]
pub fn parse_repo(repo: &str) -> Result<(&str, &str)> {
    let parts: Vec<&str> = repo.split('/').collect();
    if parts.len() != 2 {
        anyhow::bail!("Invalid repo format. Expected 'owner/repo', got: {}", repo);
    }
    Ok((parts[0], parts[1]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_repo() {
        let (owner, repo) = parse_repo("octocat/hello-world").unwrap();
        assert_eq!(owner, "octocat");
        assert_eq!(repo, "hello-world");
    }

    #[test]
    fn test_parse_repo_invalid() {
        assert!(parse_repo("invalid").is_err());
        assert!(parse_repo("too/many/parts").is_err());
    }
}
