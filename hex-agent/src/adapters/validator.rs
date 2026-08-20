use hex_core::domain::validation::ValidationRule;
use hex_core::ports::validator::IValidator;

/// Validator adapter that aggregates multiple validation rules.
/// Implements the IValidator port by maintaining a collection of
/// ValidationRule trait objects and running them all on validate_all.
pub struct Validator {
    rules: Vec<Box<dyn ValidationRule>>,
}

impl Validator {
    /// Create a new Validator with no rules registered.
    pub fn new() -> Self {
        Self { rules: Vec::new() }
    }
}

impl IValidator for Validator {
    fn add_rule(&mut self, rule: Box<dyn ValidationRule>) {
        self.rules.push(rule);
    }

    fn validate_all(&self, path: &str, content: &str) -> Result<(), Vec<String>> {
        let errors: Vec<String> = self
            .rules
            .iter()
            .filter_map(|rule| rule.validate(path, content).err())
            .collect();

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hex_core::domain::validation::CriticalPathRule;

    struct AlwaysFailRule(String);
    impl ValidationRule for AlwaysFailRule {
        fn validate(&self, _path: &str, _content: &str) -> Result<(), String> {
            Err(self.0.clone())
        }
    }

    struct AlwaysPassRule;
    impl ValidationRule for AlwaysPassRule {
        fn validate(&self, _path: &str, _content: &str) -> Result<(), String> {
            Ok(())
        }
    }

    #[test]
    fn test_no_rules_passes() {
        let validator = Validator::new();
        assert!(validator.validate_all("src/test.rs", "").is_ok());
    }

    #[test]
    fn test_single_passing_rule() {
        let mut validator = Validator::new();
        validator.add_rule(Box::new(AlwaysPassRule));
        assert!(validator.validate_all("src/test.rs", "").is_ok());
    }

    #[test]
    fn test_single_failing_rule() {
        let mut validator = Validator::new();
        validator.add_rule(Box::new(AlwaysFailRule("error 1".to_string())));
        let result = validator.validate_all("src/test.rs", "");
        assert!(result.is_err());
        let errors = result.unwrap_err();
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0], "error 1");
    }

    #[test]
    fn test_multiple_failing_rules_collect_all_errors() {
        let mut validator = Validator::new();
        validator.add_rule(Box::new(AlwaysFailRule("error 1".to_string())));
        validator.add_rule(Box::new(AlwaysFailRule("error 2".to_string())));
        validator.add_rule(Box::new(AlwaysFailRule("error 3".to_string())));

        let result = validator.validate_all("src/test.rs", "");
        assert!(result.is_err());
        let errors = result.unwrap_err();
        assert_eq!(errors.len(), 3);
        assert!(errors.contains(&"error 1".to_string()));
        assert!(errors.contains(&"error 2".to_string()));
        assert!(errors.contains(&"error 3".to_string()));
    }

    #[test]
    fn test_mixed_passing_and_failing_rules() {
        let mut validator = Validator::new();
        validator.add_rule(Box::new(AlwaysPassRule));
        validator.add_rule(Box::new(AlwaysFailRule("error 1".to_string())));
        validator.add_rule(Box::new(AlwaysPassRule));
        validator.add_rule(Box::new(AlwaysFailRule("error 2".to_string())));

        let result = validator.validate_all("src/test.rs", "");
        assert!(result.is_err());
        let errors = result.unwrap_err();
        assert_eq!(errors.len(), 2);
    }

    #[test]
    fn test_critical_path_rule_integration() {
        let mut validator = Validator::new();
        validator.add_rule(Box::new(CriticalPathRule));

        // Non-critical path should pass
        assert!(validator.validate_all("src/adapters/foo.rs", "").is_ok());

        // Critical path should fail
        let result = validator.validate_all("/etc/passwd", "");
        assert!(result.is_err());
        let errors = result.unwrap_err();
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("critical"));
    }
}