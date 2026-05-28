use serde::{Serialize, Deserialize};
use std::convert::TryFrom;
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Money(u64);

impl Money {
    pub fn cents(&self) -> u64 {
        self.0
    }
}

#[derive(Debug, Error)]
pub enum MoneyError {
    #[error("Money value cannot be negative")]
    NegativeValue,
}

impl TryFrom<u64> for Money {
    type Error = MoneyError;

    fn try_from(value: u64) -> Result<Self, Self::Error> {
        // Since u64 is unsigned, this check is redundant but included for clarity.
        if value < 0 {
            Err(MoneyError::NegativeValue)
        } else {
            Ok(Money(value))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_money_valid() {
        let money = Money::try_from(100).unwrap();
        assert_eq!(money.cents(), 100);
    }

    #[test]
    fn test_money_zero() {
        let money = Money::try_from(0).unwrap();
        assert_eq!(money.cents(), 0);
    }

    // Note: The following test is redundant due to u64 being unsigned, but kept for spec adherence.
    #[test]
    fn test_money_negative_value_fails() {
        assert!(Money::try_from(-1 as i64 as u64).is_err());
    }
}

// docs/specs/ebay-spec-008