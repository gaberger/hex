use serde::{Serialize, Deserialize};
use std::convert::TryFrom;
use std::str;

/// ListingTitle represents a title for a listing on eBay.
///
/// It must be 3 to 120 characters long after trimming whitespace and cannot contain control characters.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ListingTitle(String);

impl TryFrom<String> for ListingTitle {
    type Error = &'static str;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        let trimmed_value = value.trim();

        if !trimmed_value.is_empty()
            && trimmed_value.len() >= 3
            && trimmed_value.len() <= 120
            && !trimmed_value.chars().any(|c| c.is_control())
        {
            Ok(ListingTitle(trimmed_value.to_string()))
        } else {
            Err("Invalid ListingTitle: must be 3-120 characters long, trimmed, and contain no control characters")
        }
    }
}

impl AsRef<str> for ListingTitle {
    fn as_ref(&self) -> &str {
        self.0.as_str()
    }
}

#[cfg(test)]
mod tests {
    use super::ListingTitle;
    use std::convert::TryFrom;

    #[test]
    fn valid_titles() {
        assert!(ListingTitle::try_from("A".repeat(3)).is_ok());
        assert!(ListingTitle::try_from("Valid Title").is_ok());
        assert!(ListingTitle::try_from(" ".to_string() + "Trimmed Title" + " ").is_ok());
        assert!(ListingTitle::try_from("A".repeat(120)).is_ok());
    }

    #[test]
    fn invalid_titles() {
        assert!(ListingTitle::try_from("").is_err()); // Empty string
        assert!(ListingTitle::try_from("  ").is_err()); // Only whitespace
        assert!(ListingTitle::try_from("A".repeat(2)).is_err()); // Less than 3 characters
        assert!(ListingTitle::try_from("A".repeat(121)).is_err()); // More than 120 characters
        assert!(ListingTitle::try_from("Invalid\nTitle").is_err()); // Contains newline control character
    }
}
// docs/specs/ebay-spec-007