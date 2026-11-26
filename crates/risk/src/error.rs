use std::fmt;

use trading_engine_core::DomainError;

/// Errors produced by risk calculations or sizing inputs.
#[derive(Debug, Clone, PartialEq)]
pub enum RiskError {
    NonFinite { field: &'static str, value: f64 },
    InvalidFee(f64),
    InvalidRisk(f64),
    InvalidDailyLossCap(f64),
    InvalidCapitalCap(f64),
    InvalidLeverage(f64),
    Domain(DomainError),
}

impl From<DomainError> for RiskError {
    fn from(value: DomainError) -> Self {
        Self::Domain(value)
    }
}

impl fmt::Display for RiskError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NonFinite { field, value } => write!(f, "non-finite {field}: {value}"),
            Self::InvalidFee(value) => write!(f, "fee must be in [0, 1), got {value}"),
            Self::InvalidRisk(value) => write!(f, "risk must be in (0, 1], got {value}"),
            Self::InvalidDailyLossCap(value) => {
                write!(f, "daily_loss_cap must be in (0, 1], got {value}")
            }
            Self::InvalidCapitalCap(value) => {
                write!(f, "capital_cap must be positive when set, got {value}")
            }
            Self::InvalidLeverage(value) => {
                write!(f, "max_leverage must be positive, got {value}")
            }
            Self::Domain(err) => write!(f, "domain error: {err}"),
        }
    }
}

impl std::error::Error for RiskError {}
