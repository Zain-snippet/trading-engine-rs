use std::fmt;

use trading_engine_core::DomainError;
use trading_engine_indicators::IndicatorError;

/// Errors produced when validating parameters or evaluating a strategy.
#[derive(Debug, Clone, PartialEq)]
pub enum StrategyError {
    InvalidParameter {
        strategy: &'static str,
        reason: String,
    },
    EmptyCandles,
    MissingAtr {
        candle_index: usize,
    },
    Indicator(IndicatorError),
    Domain(DomainError),
}

impl From<IndicatorError> for StrategyError {
    fn from(value: IndicatorError) -> Self {
        Self::Indicator(value)
    }
}

impl From<DomainError> for StrategyError {
    fn from(value: DomainError) -> Self {
        Self::Domain(value)
    }
}

impl fmt::Display for StrategyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidParameter { strategy, reason } => {
                write!(f, "invalid parameter for {strategy}: {reason}")
            }
            Self::EmptyCandles => f.write_str("cannot evaluate strategy on empty candle slice"),
            Self::MissingAtr { candle_index } => {
                write!(
                    f,
                    "ATR is not warmed up at candle index {candle_index} (requires >= 14 candles)"
                )
            }
            Self::Indicator(err) => write!(f, "indicator error: {err}"),
            Self::Domain(err) => write!(f, "domain error: {err}"),
        }
    }
}

impl std::error::Error for StrategyError {}
