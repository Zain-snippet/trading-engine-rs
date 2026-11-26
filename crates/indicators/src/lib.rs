//! Technical indicators matching the original Python `tradebot/indicators.py`
//! and Donchian rolling channel calculations in `tradebot/strategies.py`.
//!
//! All indicators return `Vec<Option<f64>>` (or structs of `Option<f64>`) aligned
//! 1:1 with the input slice, where `None` represents pandas warmup `NaN` values.

mod donchian;
mod error;
mod ewm;

pub use donchian::{donchian_channels, shifted_rolling_max, shifted_rolling_min, DonchianLevel};
pub use error::IndicatorError;
pub use ewm::{atr, ema, rsi, true_range};
