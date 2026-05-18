/// Critical system files that should not be modified
const CRITICAL_PATHS: &[&str] = &[
    "/etc/passwd",
    "/etc/shadow",
    "/boot/grub2/grub.cfg",
    "/etc/sudoers",
];

/// Critical hex infrastructure files autonomous agents cannot modify
/// without explicit operator approval. Each entry is a path *suffix* —
/// matched against the trailing components of the candidate path so that
/// e.g. "hex-nexus/src/sched.rs" matches whether passed absolute,
/// repo-relative, or as bare basename.
pub const CRITICAL_FILES: &[&str] = &[
    "sched.rs",
    "monitor.rs",
    "workplan_executor.rs",
    "main.rs",
];

/// Check if a path is a critical system file OR a critical hex
/// infrastructure file. Used by validation rules to prevent modification
/// of sensitive files by autonomous agents (SafeFileWriter adapter).
pub fn is_critical_path(path: &str) -> bool {
    if CRITICAL_PATHS.contains(&path) {
        return true;
    }
    let normalized = path.trim_end_matches('/');
    CRITICAL_FILES.iter().any(|&infra| {
        normalized == infra
            || normalized.ends_with(&format!("/{}", infra))
    })
}

/// ValidationRule trait defines the contract for validation logic.
/// All validation rules must be Send + Sync for use in async/threaded contexts.
pub trait ValidationRule: Send + Sync {
    fn validate(&self, path: &str, content: &str) -> Result<(), String>;
}

/// CriticalPathRule encapsulates the existing is_critical_path logic,
/// checking if a path matches system-critical files.
pub struct CriticalPathRule;

impl ValidationRule for CriticalPathRule {
    fn validate(&self, path: &str, _content: &str) -> Result<(), String> {
        if is_critical_path(path) {
            Err(format!("Cannot modify critical system file: {}", path))
        } else {
            Ok(())
        }
    }
}