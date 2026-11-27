//! Binance Spot Testnet REST integration.
//!
//! The endpoint is deliberately fixed to the Spot Testnet. This crate does not
//! expose a configurable host and currently implements public market data only.

mod authenticated;
mod market_data;

pub use authenticated::{
    AccountInfo, AssetBalance, BinanceTestnetClient, ExchangeInfo, ExchangeSymbol, OrderResponse,
    OrderStatus, TestnetCredentials, TestnetError,
};
pub use market_data::{BinancePublicClient, MarketCandle, MarketDataError};
