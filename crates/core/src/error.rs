use std::fmt;

/// Errors produced while constructing or validating domain values.
#[derive(Debug, Clone, PartialEq)]
pub enum DomainError {
    /// A symbol string or symbol component is not valid.
    InvalidSymbol(String),
    /// The timeframe is not supported by the existing bot.
    InvalidTimeFrame(String),
    /// A timestamp could not be parsed or is outside the supported range.
    InvalidTimestamp(String),
    /// A numerical field was NaN or infinite.
    NonFinite { field: &'static str, value: f64 },
    /// A numerical field that must be non-negative was negative.
    Negative { field: &'static str, value: f64 },
    /// A price was zero, negative, or otherwise invalid.
    InvalidPrice { field: &'static str, value: f64 },
    /// A quantity was zero, negative, or non-finite.
    InvalidQuantity { value: f64 },
    /// The OHLC relationship or another candle invariant was violated.
    InvalidCandle(&'static str),
    /// A position invariant was violated.
    InvalidPosition(&'static str),
}

impl fmt::Display for DomainError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidSymbol(value) => write!(f, "invalid symbol: {value}"),
            Self::InvalidTimeFrame(value) => write!(f, "unsupported timeframe: {value}"),
            Self::InvalidTimestamp(value) => write!(f, "invalid timestamp: {value}"),
            Self::NonFinite { field, value } => {
                write!(f, "non-finite value for {field}: {value}")
            }
            Self::Negative { field, value } => write!(f, "negative value for {field}: {value}"),
            Self::InvalidPrice { field, value } => write!(f, "invalid price for {field}: {value}"),
            Self::InvalidQuantity { value } => write!(f, "invalid quantity: {value}"),
            Self::InvalidCandle(reason) => write!(f, "invalid candle: {reason}"),
            Self::InvalidPosition(reason) => write!(f, "invalid position: {reason}"),
        }
    }
}

impl std::error::Error for DomainError {}
