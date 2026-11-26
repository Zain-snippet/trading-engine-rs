use std::fmt;

use crate::{error::DomainError, validation::ensure_finite};

/// A finite monetary value.
///
/// `Money` deliberately allows negative values so that profit and loss can be
/// represented. Callers that require a positive price must check it explicitly.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
pub struct Money(f64);

impl Money {
    /// Creates a monetary value, rejecting NaN and infinities.
    pub fn new(value: f64) -> Result<Self, DomainError> {
        ensure_finite("money", value)?;
        Ok(Self(value))
    }

    #[must_use]
    pub const fn as_f64(self) -> f64 {
        self.0
    }

    #[must_use]
    pub fn is_positive(self) -> bool {
        self.0 > 0.0
    }

    #[must_use]
    pub fn is_negative(self) -> bool {
        self.0 < 0.0
    }
}

impl fmt::Display for Money {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_finite_values_including_negative_pnl() {
        assert_eq!(Money::new(0.0).unwrap().as_f64(), 0.0);
        assert_eq!(Money::new(1_645.77).unwrap().as_f64(), 1_645.77);
        assert_eq!(Money::new(-12.5).unwrap().as_f64(), -12.5);
    }

    #[test]
    fn rejects_non_finite_values() {
        assert!(matches!(
            Money::new(f64::NAN),
            Err(DomainError::NonFinite { field: "money", .. })
        ));
        assert!(matches!(
            Money::new(f64::INFINITY),
            Err(DomainError::NonFinite { field: "money", .. })
        ));
        assert!(matches!(
            Money::new(f64::NEG_INFINITY),
            Err(DomainError::NonFinite { field: "money", .. })
        ));
    }

    #[test]
    fn reports_sign() {
        assert!(Money::new(1.0).unwrap().is_positive());
        assert!(Money::new(-1.0).unwrap().is_negative());
        assert!(!Money::new(0.0).unwrap().is_positive());
        assert!(!Money::new(0.0).unwrap().is_negative());
    }
}
