use crate::domain::validation::ValidationRule;

/// IValidator port defines the contract for validation services.
/// Adapters can register multiple ValidationRule implementations and
/// validate paths/content against all registered rules.
pub trait IValidator: Send + Sync {
    /// Register a new validation rule.
    fn add_rule(&mut self, rule: Box<dyn ValidationRule>);

    /// Validate a path and its content against all registered rules.
    /// Returns Ok(()) if all rules pass, or Err with a Vec of all error messages.
    fn validate_all(&self, path: &str, content: &str) -> Result<(), Vec<String>>;
}
