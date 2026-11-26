//! Risk management, equity calculation, and position sizing matching
//! `paperbot/engine.py` and `tradebot/backtest.py`.

mod equity;
mod error;
mod gate;
mod sizing;

pub use equity::{compute_equity, OpenPositionMark};
pub use error::RiskError;
pub use gate::{evaluate_risk_gate, DailyRiskState, RiskBlockReason, RiskGateOutcome};
pub use sizing::{initial_long_stop, size_backtest_position, size_paper_position};
