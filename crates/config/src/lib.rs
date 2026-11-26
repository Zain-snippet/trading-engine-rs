//! Typed configuration matching the original Python `paperbot/config.py`.
//!
//! The defaults are verified against the Python source, not against the audit
//! summary. `ATR_PERIOD` is intentionally a constant: the Python bot does not
//! expose an ATR period in `BotCfg`; `tradebot.indicators.atr` defaults to 14.

use std::{collections::HashSet, fmt, path::PathBuf};

use serde::Deserialize;
use trading_engine_core::{SymbolId, TimeFrame};

/// ATR period used by the original bot (`tradebot/indicators.py`).
///
/// This is not a `BotCfg` field. It must not become a configurable strategy
/// parameter during parity work.
pub const ATR_PERIOD: usize = 14;

fn default_stop_mult() -> f64 {
    2.0
}

/// Per-symbol strategy configuration, matching Python `SymbolCfg`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SymbolConfig {
    pub symbol: SymbolId,
    pub entry_n: usize,
    pub exit_n: usize,
    #[serde(default = "default_stop_mult")]
    pub stop_mult: f64,
}

impl SymbolConfig {
    #[must_use]
    pub fn new(symbol: SymbolId, entry_n: usize, exit_n: usize, stop_mult: f64) -> Self {
        Self {
            symbol,
            entry_n,
            exit_n,
            stop_mult,
        }
    }

    /// Validates the per-symbol lookback and stop-multiple invariants.
    pub fn validate(&self) -> Result<(), ValidationError> {
        if self.entry_n == 0 {
            return Err(ValidationError::InvalidEntryLookback {
                symbol: self.symbol.clone(),
                value: self.entry_n,
            });
        }
        if self.exit_n == 0 {
            return Err(ValidationError::InvalidExitLookback {
                symbol: self.symbol.clone(),
                value: self.exit_n,
            });
        }
        if self.entry_n <= self.exit_n {
            return Err(ValidationError::EntryNotGreaterThanExit {
                symbol: self.symbol.clone(),
                entry_n: self.entry_n,
                exit_n: self.exit_n,
            });
        }
        if !self.stop_mult.is_finite() {
            return Err(ValidationError::NonFinite {
                field: "stop_mult",
                value: self.stop_mult,
            });
        }
        if self.stop_mult <= 0.0 {
            return Err(ValidationError::InvalidStopMultiple {
                symbol: self.symbol.clone(),
                value: self.stop_mult,
            });
        }

        Ok(())
    }
}

/// Bot configuration matching Python `BotCfg`.
///
/// Fields are public for programmatic construction. Call [`Self::validate`]
/// after mutating a value; [`Self::from_toml_str`] validates automatically.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct BotConfig {
    pub symbols: Vec<SymbolConfig>,
    pub timeframe: TimeFrame,
    pub quote: String,
    pub risk_per_trade: f64,
    pub daily_loss_cap: f64,
    pub capital_cap: Option<f64>,
    pub candle_delay_s: u64,
    pub poll_s: u64,
    pub history: usize,
    pub sim_fee: f64,
    pub sim_slippage: f64,
    pub sim_start_usdt: f64,
    pub state_dir: PathBuf,
    pub log_dir: PathBuf,
}

impl Default for BotConfig {
    fn default() -> Self {
        Self {
            symbols: vec![
                SymbolConfig::new(
                    SymbolId::parse("ETH/USDT").expect("static default symbol is valid"),
                    55,
                    50,
                    2.0,
                ),
                SymbolConfig::new(
                    SymbolId::parse("BTC/USDT").expect("static default symbol is valid"),
                    100,
                    20,
                    2.0,
                ),
            ],
            timeframe: TimeFrame::FourHour,
            quote: "USDT".to_owned(),
            risk_per_trade: 0.01,
            daily_loss_cap: 0.03,
            capital_cap: Some(10_000.0),
            candle_delay_s: 60,
            poll_s: 60,
            history: 1000,
            sim_fee: 0.001,
            sim_slippage: 0.0005,
            sim_start_usdt: 10_000.0,
            state_dir: PathBuf::from("state"),
            log_dir: PathBuf::from("logs"),
        }
    }
}

impl BotConfig {
    /// Parses a TOML document, applies defaults for omitted fields, and validates it.
    ///
    /// TOML has no `null`, so `capital_cap` can only be disabled programmatically
    /// (`Some` vs `None`); an omitted TOML field keeps the Python default.
    pub fn from_toml_str(input: &str) -> Result<Self, ConfigError> {
        let config: Self =
            toml::from_str(input).map_err(|error| ConfigError::Parse(error.to_string()))?;
        config.validate().map_err(ConfigError::Validation)?;
        Ok(config)
    }

    /// Validates all configuration invariants required by the existing bot.
    pub fn validate(&self) -> Result<(), ValidationError> {
        validate_asset_code(&self.quote)?;

        if self.symbols.is_empty() {
            return Err(ValidationError::EmptySymbols);
        }

        let mut seen = HashSet::new();
        for symbol_config in &self.symbols {
            symbol_config.validate()?;

            if symbol_config.symbol.quote() != self.quote {
                return Err(ValidationError::QuoteMismatch {
                    symbol: symbol_config.symbol.clone(),
                    quote: self.quote.clone(),
                });
            }
            if !seen.insert(&symbol_config.symbol) {
                return Err(ValidationError::DuplicateSymbol(
                    symbol_config.symbol.clone(),
                ));
            }
        }

        if !self.risk_per_trade.is_finite() {
            return Err(ValidationError::NonFinite {
                field: "risk_per_trade",
                value: self.risk_per_trade,
            });
        }
        if self.risk_per_trade <= 0.0 || self.risk_per_trade > 1.0 {
            return Err(ValidationError::InvalidRiskPerTrade(self.risk_per_trade));
        }

        if !self.daily_loss_cap.is_finite() {
            return Err(ValidationError::NonFinite {
                field: "daily_loss_cap",
                value: self.daily_loss_cap,
            });
        }
        if self.daily_loss_cap <= 0.0 || self.daily_loss_cap > 1.0 {
            return Err(ValidationError::InvalidDailyLossCap(self.daily_loss_cap));
        }

        if let Some(capital_cap) = self.capital_cap {
            if !capital_cap.is_finite() {
                return Err(ValidationError::NonFinite {
                    field: "capital_cap",
                    value: capital_cap,
                });
            }
            if capital_cap <= 0.0 {
                return Err(ValidationError::InvalidCapitalCap(capital_cap));
            }
        }

        if self.poll_s == 0 {
            return Err(ValidationError::InvalidPollInterval);
        }

        let max_entry_n = self
            .symbols
            .iter()
            .map(|symbol| symbol.entry_n)
            .max()
            .unwrap_or(0);
        let required_history = max_entry_n.saturating_add(1).max(ATR_PERIOD);
        if self.history < required_history {
            return Err(ValidationError::InvalidHistory {
                history: self.history,
                required: required_history,
            });
        }

        if !self.sim_fee.is_finite() {
            return Err(ValidationError::NonFinite {
                field: "sim_fee",
                value: self.sim_fee,
            });
        }
        if !(0.0..1.0).contains(&self.sim_fee) {
            return Err(ValidationError::InvalidFee(self.sim_fee));
        }

        if !self.sim_slippage.is_finite() {
            return Err(ValidationError::NonFinite {
                field: "sim_slippage",
                value: self.sim_slippage,
            });
        }
        if !(0.0..1.0).contains(&self.sim_slippage) {
            return Err(ValidationError::InvalidSlippage(self.sim_slippage));
        }

        if !self.sim_start_usdt.is_finite() {
            return Err(ValidationError::NonFinite {
                field: "sim_start_usdt",
                value: self.sim_start_usdt,
            });
        }
        if self.sim_start_usdt <= 0.0 {
            return Err(ValidationError::InvalidSimStart(self.sim_start_usdt));
        }

        if self.state_dir.as_os_str().is_empty() {
            return Err(ValidationError::EmptyStateDir);
        }
        if self.log_dir.as_os_str().is_empty() {
            return Err(ValidationError::EmptyLogDir);
        }

        Ok(())
    }

    /// Python `BotCfg.kill_file`.
    #[must_use]
    pub fn kill_file(&self) -> PathBuf {
        self.state_dir.join("KILL")
    }

    /// Python `BotCfg.pause_file`.
    #[must_use]
    pub fn pause_file(&self) -> PathBuf {
        self.state_dir.join("PAUSE")
    }
}

fn validate_asset_code(value: &str) -> Result<(), ValidationError> {
    if value.is_empty() {
        return Err(ValidationError::EmptyQuote);
    }
    if !value
        .chars()
        .all(|character| character.is_ascii_uppercase() || character.is_ascii_digit())
    {
        return Err(ValidationError::InvalidQuote(value.to_owned()));
    }
    Ok(())
}

/// A configuration value violated a domain invariant.
#[derive(Debug, Clone, PartialEq)]
pub enum ValidationError {
    EmptySymbols,
    DuplicateSymbol(SymbolId),
    EmptyQuote,
    InvalidQuote(String),
    QuoteMismatch {
        symbol: SymbolId,
        quote: String,
    },
    InvalidEntryLookback {
        symbol: SymbolId,
        value: usize,
    },
    InvalidExitLookback {
        symbol: SymbolId,
        value: usize,
    },
    EntryNotGreaterThanExit {
        symbol: SymbolId,
        entry_n: usize,
        exit_n: usize,
    },
    InvalidStopMultiple {
        symbol: SymbolId,
        value: f64,
    },
    NonFinite {
        field: &'static str,
        value: f64,
    },
    InvalidRiskPerTrade(f64),
    InvalidDailyLossCap(f64),
    InvalidCapitalCap(f64),
    InvalidHistory {
        history: usize,
        required: usize,
    },
    InvalidFee(f64),
    InvalidSlippage(f64),
    InvalidSimStart(f64),
    InvalidPollInterval,
    EmptyStateDir,
    EmptyLogDir,
}

impl fmt::Display for ValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptySymbols => f.write_str("configuration must contain at least one symbol"),
            Self::DuplicateSymbol(symbol) => write!(f, "duplicate symbol: {symbol}"),
            Self::EmptyQuote => f.write_str("quote asset must not be empty"),
            Self::InvalidQuote(value) => write!(f, "invalid quote asset code: {value}"),
            Self::QuoteMismatch { symbol, quote } => {
                write!(f, "symbol {symbol} does not use configured quote {quote}")
            }
            Self::InvalidEntryLookback { symbol, value } => {
                write!(f, "invalid entry lookback for {symbol}: {value}")
            }
            Self::InvalidExitLookback { symbol, value } => {
                write!(f, "invalid exit lookback for {symbol}: {value}")
            }
            Self::EntryNotGreaterThanExit {
                symbol,
                entry_n,
                exit_n,
            } => write!(
                f,
                "entry lookback must be greater than exit lookback for {symbol}: {entry_n} <= {exit_n}"
            ),
            Self::InvalidStopMultiple { symbol, value } => {
                write!(f, "invalid stop multiple for {symbol}: {value}")
            }
            Self::NonFinite { field, value } => {
                write!(f, "non-finite value for {field}: {value}")
            }
            Self::InvalidRiskPerTrade(value) => {
                write!(f, "risk_per_trade must be in (0, 1], got {value}")
            }
            Self::InvalidDailyLossCap(value) => {
                write!(f, "daily_loss_cap must be in (0, 1], got {value}")
            }
            Self::InvalidCapitalCap(value) => {
                write!(f, "capital_cap must be positive when set, got {value}")
            }
            Self::InvalidHistory { history, required } => write!(
                f,
                "history must be at least {required} candles for the configured lookbacks, got {history}"
            ),
            Self::InvalidFee(value) => write!(f, "sim_fee must be in [0, 1), got {value}"),
            Self::InvalidSlippage(value) => {
                write!(f, "sim_slippage must be in [0, 1), got {value}")
            }
            Self::InvalidSimStart(value) => {
                write!(f, "sim_start_usdt must be positive, got {value}")
            }
            Self::InvalidPollInterval => f.write_str("poll_s must be greater than zero"),
            Self::EmptyStateDir => f.write_str("state_dir must not be empty"),
            Self::EmptyLogDir => f.write_str("log_dir must not be empty"),
        }
    }
}

impl std::error::Error for ValidationError {}

/// An error while parsing or validating configuration.
#[derive(Debug, Clone, PartialEq)]
pub enum ConfigError {
    Parse(String),
    Validation(ValidationError),
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Parse(message) => write!(f, "failed to parse configuration: {message}"),
            Self::Validation(error) => write!(f, "invalid configuration: {error}"),
        }
    }
}

impl std::error::Error for ConfigError {}
