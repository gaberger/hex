use super::PathRule;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_path_rule_valid() {
        let rule = PathRule::new("/valid/path");
        assert!(rule.is_valid());
    }

    #[test]
    fn test_path_rule_invalid() {
        let rule = PathRule::new("invalid/path");
        assert!(!rule.is_valid());
    }
}