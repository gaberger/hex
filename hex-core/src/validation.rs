use crate::domain::validation::is_critical_path;

pub trait ValidationRule {
    fn validate(&self, path: &str, content: &str) -> Result<(), String>;
}

/// CriticalPathRule encapsulates the existing is_critical_path logic
/// from domain::validation, checking if a path matches system-critical files.
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