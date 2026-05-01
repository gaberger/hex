/// Critical system files that should not be modified
const CRITICAL_PATHS: &[&str] = &[
    "/etc/passwd",
    "/etc/shadow",
    "/boot/grub2/grub.cfg",
    "/etc/sudoers",
];

/// Check if a path is a critical system file.
/// Used by validation rules to prevent modification of sensitive system files.
pub fn is_critical_path(path: &str) -> bool {
    CRITICAL_PATHS.iter().any(|&critical| path == critical)
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