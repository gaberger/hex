use super::ValidationResult;
use crate::model::Path;

pub trait ValidationRule {
    fn validate(&self, path: &str, content: &str) -> Result<(), String>;
}

// Existing is_critical_path logic
fn is_critical_path(path: &Path) -> bool {
    // Example implementation
    path.to_string().contains("critical")
}

pub struct CriticalPathRule;

impl ValidationRule for CriticalPathRule {
    fn validate(&self, path: &str, _content: &str) -> Result<(), String> {
        let path_obj = Path::from(path);
        if is_critical_path(&path_obj) {
            Ok(())
        } else {
            Err("Path is not a critical path".to_string())
        }
    }
}