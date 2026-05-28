use std::convert::{TryFrom, TryInto};
use serde::{Serialize, Deserialize};

/// Represents a validated Username in lowercase with no control characters.
///
/// # Validation Rules:
/// - Must be canonical lowercase.
/// - No control characters (ASCII codes 0-31 or 127).
///
/// Refer to docs/specs/ebay-spec-008 for detailed specifications.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Username(String);

impl TryFrom<String> for Username {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        if !value.chars().all(|c| c.is_ascii_lowercase() && !c.is_control()) {
            return Err("Username must be lowercase and contain no control characters.".to_string());
        }
        Ok(Username(value))
    }
}

impl Username {
    /// Constructs a new `Username` from a string slice.
    ///
    /// # Errors
    /// Returns an error if the username does not meet the validation criteria.
    pub fn new(username: &str) -> Result<Self, String> {
        Self::try_from(username.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_username_validation() {
        assert!(Username::new("validusername").is_ok());
        assert!(Username::new("ValidUsername").is_err());
        assert!(Username::new("invalid@username").is_err());
        assert!(Username::new("username\nwithnewline").is_err());
        assert!(Username::new("").is_ok()); // Assuming empty string is allowed, adjust spec if needed
    }
}