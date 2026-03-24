//! Agent implementations for `hex dev` pipeline.
//!
//! Each agent wraps inference logic into a task-oriented interface:
//! load a prompt template, call inference via hex-nexus, parse the result.

pub mod documenter;
pub mod reviewer;
pub mod tester;
pub mod ux_reviewer;

pub use documenter::{DocResult, DocumenterAgent};
pub use reviewer::{ReviewerAgent, ReviewIssue, ReviewResult};
pub use tester::{TesterAgent, TestAgentResult};
pub use ux_reviewer::{UxIssue, UxReviewResult, UxReviewerAgent};
