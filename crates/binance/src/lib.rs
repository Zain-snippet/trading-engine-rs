//! Binance Spot Testnet REST integration.
//!
//! The endpoint is deliberately fixed to the Spot Testnet. This crate does not
//! expose a configurable host. Public market data and authenticated Testnet
//! account/trade operations remain isolated from strategy code.

mod authenticated;
mod market_data;
mod validation;

pub use authenticated::{
    AccountInfo, AssetBalance, BinanceTestnetClient, ExchangeInfo, ExchangeSymbol, OrderResponse,
    OrderStatus, TestnetCredentials, TestnetError,
};
pub use market_data::{BinancePublicClient, MarketCandle, MarketDataError};
pub use validation::{
    Decimal, ExchangeRules, OrderValidationError, OrderValidator, PriceFilter, QuantityFilter,
};
