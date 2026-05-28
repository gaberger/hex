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
        Ok(Money(value))
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
}

// docs/specs/ebay-spec-008