//! Core domain types for the trading engine.
//!
//! This crate intentionally contains no indicators, strategies, exchange clients,
//! or execution logic. It provides the strongly typed values that later crates
//! will share.

mod candle;
mod error;
mod money;
mod order;
mod position;
mod signal;
mod symbol;
mod timeframe;
mod timestamp;
mod validation;

pub use candle::Candle;
pub use error::DomainError;
pub use money::Money;
pub use order::{Fill, OrderIntent, Side};
pub use position::Position;
pub use signal::Signal;
pub use symbol::SymbolId;
pub use timeframe::TimeFrame;
pub use timestamp::Timestamp;
