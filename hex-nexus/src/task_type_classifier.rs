//! Task-type-aware inference routing (ADR-2604142000).
//!
//! Complements `score_complexity()` with task-type signals that require a
//! minimum inference tier regardless of prompt length. Shell command generation,
//! precise syntax translation, and reasoning-heavy tasks all need T2.5 even
//! when the prompt is short.

use crate::remote::transport::TaskTier;

/// Task type signals that raise the minimum inference tier.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskType {
    /// Shell command generation or system operation translation.
    ShellCommand,
    /// File format conversion, migration, or multi-file transformation.
    FileTransform,
    /// Debugging, failure analysis, causal reasoning.
    Reasoning,
    /// Precise API/syntax/flag selection from options.
    PreciseSyntax,
    /// No specific type — use complexity-based routing only.
    General,
}

impl TaskType {
    /// Minimum inference tier required for this task type.
    pub fn min_tier(self) -> TaskTier {
        match self {
            Self::ShellCommand => TaskTier::T2_5,
            Self::FileTransform => TaskTier::T2,
            Self::Reasoning => TaskTier::T2_5,
            Self::PreciseSyntax => TaskTier::T2_5,
            Self::General => TaskTier::T1,
        }
    }

    /// Human-readable name for logging and tracing.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::ShellCommand => "shell_command",
            Self::FileTransform => "file_transform",
            Self::Reasoning => "reasoning",
            Self::PreciseSyntax => "precise_syntax",
            Self::General => "general",
        }
    }
}

/// Raised-tier result from task-type classification.
#[derive(Debug, Clone, Copy)]
pub struct TaskTypeResult {
    pub task_type: TaskType,
    pub raised_tier: TaskTier,
}

/// Classify a prompt and return the task type + minimum required tier as a tuple.
///
/// Thin ergonomic shim over [`classify_task_type`] matching the ADR-2604142000
/// surface: `classify(prompt) -> Option<(TaskType, TaskTier)>`.
pub fn classify(prompt: &str) -> Option<(TaskType, TaskTier)> {
    classify_task_type(prompt).map(|r| (r.task_type, r.raised_tier))
}

#[allow(dead_code)]
pub struct TaskTypeRule {
    pub label: &'static str,
    pub task_type: TaskType,
    pub raised_tier: TaskTier,
    pub signals: &'static [&'static str],
    pub matches: fn(&str) -> bool,
}

pub static TASK_TYPE_RULES: &[TaskTypeRule] = &[
    TaskTypeRule { label: "shell_command", task_type: TaskType::ShellCommand, raised_tier: TaskTier::T2_5, signals: &["run+tool", "execute+tool", "ssh", "docker", "cargo"], matches: is_shell_command },
    TaskTypeRule { label: "file_transform", task_type: TaskType::FileTransform, raised_tier: TaskTier::T2, signals: &["convert", "migrate", "transform", "rename across"], matches: is_file_transform },
    TaskTypeRule { label: "reasoning", task_type: TaskType::Reasoning, raised_tier: TaskTier::T2_5, signals: &["debug", "trace", "root cause", "investigate"], matches: is_reasoning },
    TaskTypeRule { label: "precise_syntax", task_type: TaskType::PreciseSyntax, raised_tier: TaskTier::T2_5, signals: &["api endpoint", "exact flag", "correct syntax"], matches: is_precise_syntax },
];

/// Classify a prompt by task type and return the minimum required tier.
///
/// Classification is deterministic keyword + pattern matching — no LLM call.
pub fn classify_task_type(prompt: &str) -> Option<TaskTypeResult> {
    let lower = prompt.to_lowercase();
    TASK_TYPE_RULES
        .iter()
        .find(|r| (r.matches)(&lower))
        .map(|r| TaskTypeResult {
            task_type: r.task_type,
            raised_tier: r.raised_tier,
        })
}

/// Shell command generation signal.
///
/// Recognizes prompts asking to run commands, execute operations, or translate
/// intent to shell syntax — including SSH targets and system tools.
fn is_shell_command(prompt: &str) -> bool {
    let shell_verbs = [
        "run ", "run the", "execute", "check ", "verify ", "test ", "run '", "run \"", "run `",
        "run this",
    ];
    let remote_targets = ["ssh ", "on ", "via ssh", "remote ", "over ssh"];
    let system_tools = [
        "ollama",
        "systemctl",
        "docker",
        "kubectl",
        "git ",
        "curl ",
        "curl'",
        "curl\"",
        "npm ",
        "cargo ",
        "pip ",
        "apt",
        "yum",
        "dnf",
        "brew ",
        "ssh ",
        "scp ",
        "journalctl",
        "ss -",
        "netstat",
        "ps aux",
        "top -",
    ];

    let has_verb = shell_verbs.iter().any(|v| prompt.contains(v));
    let has_remote = remote_targets.iter().any(|t| prompt.contains(t));
    let has_tool = system_tools.iter().any(|t| prompt.contains(t));
    let has_ssh_host = prompt.contains("bazzite")
        || prompt.contains("remote")
        || prompt.contains("host ")
        || prompt.contains("server ");

    (has_verb && has_tool) || (has_remote && has_tool) || (has_ssh_host && has_verb)
}

/// File transformation signal.
///
/// Recognizes format conversion, migration, and multi-file rename/replace.
fn is_file_transform(prompt: &str) -> bool {
    let patterns = [
        "convert",
        "migrate",
        "rename across",
        "replace across",
        "transform",
        "rewrite in",
        "port to",
        "update all",
    ];
    patterns.iter().any(|p| prompt.contains(p))
}

/// Reasoning signal.
///
/// Recognizes debugging, failure analysis, and causal reasoning tasks.
fn is_reasoning(prompt: &str) -> bool {
    let patterns = [
        "explain why",
        "debug",
        "trace",
        "why did",
        "what caused",
        "root cause",
        "figure out",
        "find the bug",
        "what went wrong",
        "analyze failure",
        "investigate",
        "diagnose",
    ];
    patterns.iter().any(|p| prompt.contains(p))
}

/// Precise syntax signal.
///
/// Recognizes requests that require exact API endpoints, HTTP methods, or
/// command-line flags — where a wrong answer is worse than no answer.
fn is_precise_syntax(prompt: &str) -> bool {
    let patterns = [
        "api endpoint",
        "http method",
        "exact flag",
        "correct syntax",
        "which flag",
        "what parameter",
        "which parameter",
    ];
    patterns.iter().any(|p| prompt.contains(p))
}

/// Raise a tier if the task type requires a higher minimum than the current tier.
///
/// Returns the higher tier if task_type is more demanding; otherwise returns
/// the original tier (no change).
pub fn raise_tier_if_needed(current: TaskTier, task_type: Option<TaskTypeResult>) -> TaskTier {
    match task_type {
        Some(tt) if tier_order(tt.raised_tier) > tier_order(current) => tt.raised_tier,
        _ => current,
    }
}

fn tier_order(tier: TaskTier) -> u8 {
    match tier {
        TaskTier::T1 => 1,
        TaskTier::T2 => 2,
        TaskTier::T2_5 => 3,
        TaskTier::T3 => 4,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(tier: TaskTier) -> TaskTier {
        tier
    }

    #[test]
    fn short_shell_command_routes_to_t2_5() {
        let result = classify_task_type("Run 'ollama list' on bazzite via SSH");
        assert!(result.is_some());
        assert_eq!(result.unwrap().task_type, TaskType::ShellCommand);
        assert_eq!(result.unwrap().raised_tier, TaskTier::T2_5);
    }

    #[test]
    fn ollama_health_wrong_command() {
        let result = classify_task_type("check if ollama is running on bazzite");
        assert!(result.is_some());
        assert_eq!(result.unwrap().task_type, TaskType::ShellCommand);
    }

    #[test]
    fn ssh_with_verb_routes_to_t2_5() {
        let result = classify_task_type("ssh bazzite docker ps");
        assert!(result.is_some());
        assert_eq!(result.unwrap().task_type, TaskType::ShellCommand);
    }

    #[test]
    fn generic_short_prompt_is_none() {
        assert!(classify_task_type("add a comment").is_none());
    }

    #[test]
    fn file_transform_routes_to_t2() {
        let result = classify_task_type("migrate this to TypeScript");
        assert!(result.is_some());
        assert_eq!(result.unwrap().task_type, TaskType::FileTransform);
        assert_eq!(result.unwrap().raised_tier, TaskTier::T2);
    }

    #[test]
    fn reasoning_routes_to_t2_5() {
        let result = classify_task_type("explain why this is failing");
        assert!(result.is_some());
        assert_eq!(result.unwrap().task_type, TaskType::Reasoning);
    }

    #[test]
    fn api_endpoint_routes_to_t2_5() {
        let result = classify_task_type("which API endpoint should I use for this");
        assert!(result.is_some());
        assert_eq!(result.unwrap().task_type, TaskType::PreciseSyntax);
    }

    #[test]
    fn classify_tuple_api_matches_classify_task_type() {
        let tuple = classify("run ollama ps on bazzite");
        let full = classify_task_type("run ollama ps on bazzite");
        assert!(tuple.is_some() && full.is_some());
        let (tt, tier) = tuple.unwrap();
        assert_eq!(tt, full.unwrap().task_type);
        assert_eq!(tier, full.unwrap().raised_tier);
    }

    #[test]
    fn classify_tuple_returns_none_for_trivial_prompt() {
        assert!(classify("fix typo").is_none());
    }

    #[test]
    fn systemctl_with_verb_routes_to_shell() {
        let (tt, tier) = classify("check systemctl status on server").unwrap();
        assert_eq!(tt, TaskType::ShellCommand);
        assert_eq!(tier, TaskTier::T2_5);
    }

    #[test]
    fn execute_verb_with_system_tool() {
        let (tt, _) = classify("execute docker ps -a").unwrap();
        assert_eq!(tt, TaskType::ShellCommand);
    }

    #[test]
    fn journalctl_on_remote_host() {
        let (tt, _) = classify("run journalctl -u hex on bazzite").unwrap();
        assert_eq!(tt, TaskType::ShellCommand);
    }

    #[test]
    fn debug_keyword_routes_to_reasoning() {
        let (tt, tier) = classify("debug this deadlock").unwrap();
        assert_eq!(tt, TaskType::Reasoning);
        assert_eq!(tier, TaskTier::T2_5);
    }

    #[test]
    fn convert_keyword_routes_to_file_transform() {
        let (tt, tier) = classify("convert this JSON schema to a Rust struct").unwrap();
        assert_eq!(tt, TaskType::FileTransform);
        assert_eq!(tier, TaskTier::T2);
    }

    #[test]
    fn which_flag_routes_to_precise_syntax() {
        let (tt, tier) = classify("which flag enables verbose output").unwrap();
        assert_eq!(tt, TaskType::PreciseSyntax);
        assert_eq!(tier, TaskTier::T2_5);
    }

    #[test]
    fn plain_docs_prompt_is_none() {
        assert!(classify("write documentation for the login flow").is_none());
    }

    #[test]
    fn min_tier_reflects_enum_variants() {
        assert_eq!(TaskType::ShellCommand.min_tier(), TaskTier::T2_5);
        assert_eq!(TaskType::FileTransform.min_tier(), TaskTier::T2);
        assert_eq!(TaskType::Reasoning.min_tier(), TaskTier::T2_5);
        assert_eq!(TaskType::PreciseSyntax.min_tier(), TaskTier::T2_5);
        assert_eq!(TaskType::General.min_tier(), TaskTier::T1);
    }

    #[test]
    fn raise_tier_never_downgrades_from_t3() {
        assert_eq!(
            raise_tier_if_needed(TaskTier::T3, classify_task_type("run ollama ps on bazzite")),
            TaskTier::T3
        );
    }

    #[test]
    fn raise_tier_conservative() {
        assert_eq!(
            raise_tier_if_needed(
                t(TaskTier::T1),
                classify_task_type("run ollama ps on bazzite")
            ),
            t(TaskTier::T2_5)
        );
        assert_eq!(
            raise_tier_if_needed(
                t(TaskTier::T2_5),
                classify_task_type("run ollama ps on bazzite")
            ),
            t(TaskTier::T2_5)
        );
        assert_eq!(
            raise_tier_if_needed(t(TaskTier::T2), classify_task_type("add a comment")),
            t(TaskTier::T2)
        );
    }

    #[test]
    fn task_type_rule_table_invariants() {
        assert_eq!(TASK_TYPE_RULES.len(), 4, "expected 4 task type rules");
        for rule in TASK_TYPE_RULES {
            assert!(!rule.label.is_empty());
            assert!(!rule.signals.is_empty(), "rule {:?} has no signals", rule.label);
        }
        assert_eq!(TASK_TYPE_RULES[0].label, "shell_command",
            "shell_command must be first (most specific compound match)");
    }
}
