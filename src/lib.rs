pub mod adapters;
pub mod github;
pub mod ledger;
pub mod models;
pub mod orchestrator;

pub use adapters::{ClaudeAdapter, CodexAdapter};
pub use github::GitHubClient;
pub use ledger::{JsonLedger, Ledger, PostgresLedger, RepoStats};
pub use models::*;
pub use orchestrator::{generate_summary, Orchestrator};
