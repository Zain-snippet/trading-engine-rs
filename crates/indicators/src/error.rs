use std::fmt;

/// Errors produced when computing technical indicators.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IndicatorError {
    /// An indicator period or lookback window was zero.
    InvalidPeriod {
        indicator: &'static str,
        period: usize,
    },
    /// An input series contained a `NaN` or infinite value.
    NonFiniteInput {
        indicator: &'static str,
        index: usize,
    },
}

impl fmt::Display for IndicatorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidPeriod { indicator, period } => {
                write!(f, "invalid period for {indicator}: {period} (must be >= 1)")
            }
            Self::NonFiniteInput { indicator, index } => {
                write!(f, "non-finite input for {indicator} at index {index}")
            }
        }
    }
}

impl std::error::Error for IndicatorError {}
