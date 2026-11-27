//! Rule-based trading strategies matching `tradebot/strategies.py` and
//! `paperbot/engine.py:compute_signal`.
//!
//! All strategies are strictly causal: each bar's signal uses only data up to
//! and including that bar's close.

mod donchian;
mod ema_cross;
mod error;
mod rsi_reversion;

pub use donchian::{Donchian, DonchianEvaluation};
pub use ema_cross::EmaCross;
pub use error::StrategyError;
pub use rsi_reversion::RsiReversion;

use trading_engine_core::Candle;

/// Per-bar strategy signal output matching `SIG_COLS` in `tradebot/strategies.py`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StrategyBar {
    pub enter_long: bool,
    pub exit_long: bool,
    pub atr: Option<f64>,
}

/// A causal bar-by-bar trading strategy.
pub trait Strategy {
    fn name(&self) -> &'static str;
    fn stop_mult(&self) -> f64;
    fn trailing(&self) -> bool;
    fn signals(&self, candles: &[Candle]) -> Result<Vec<StrategyBar>, StrategyError>;
}
