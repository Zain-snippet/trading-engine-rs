use std::path::PathBuf;

use trading_engine_config::{BotConfig, ConfigError, SymbolConfig, ValidationError, ATR_PERIOD};
use trading_engine_core::{SymbolId, TimeFrame};

fn symbol_config(symbol: &str, entry_n: usize, exit_n: usize) -> SymbolConfig {
    SymbolConfig::new(SymbolId::parse(symbol).unwrap(), entry_n, exit_n, 2.0)
}

fn config_with(mutate: impl FnOnce(&mut BotConfig)) -> BotConfig {
    let mut config = BotConfig::default();
    mutate(&mut config);
    config
}

#[test]
fn defaults_match_python_bot_cfg_and_symbol_cfg() {
    let config = BotConfig::default();

    assert_eq!(config.symbols.len(), 2);

    let eth = &config.symbols[0];
    assert_eq!(eth.symbol.to_string(), "ETH/USDT");
    assert_eq!(eth.symbol.base(), "ETH");
    assert_eq!(eth.symbol.quote(), "USDT");
    assert_eq!(eth.symbol.binance_symbol(), "ETHUSDT");
    assert_eq!(eth.entry_n, 55);
    assert_eq!(eth.exit_n, 50);
    assert_eq!(eth.stop_mult, 2.0);

    let btc = &config.symbols[1];
    assert_eq!(btc.symbol.to_string(), "BTC/USDT");
    assert_eq!(btc.symbol.base(), "BTC");
    assert_eq!(btc.symbol.quote(), "USDT");
    assert_eq!(btc.symbol.binance_symbol(), "BTCUSDT");
    assert_eq!(btc.entry_n, 100);
    assert_eq!(btc.exit_n, 20);
    assert_eq!(btc.stop_mult, 2.0);

    assert_eq!(config.timeframe, TimeFrame::FourHour);
    assert_eq!(config.quote, "USDT");
    assert_eq!(config.risk_per_trade, 0.01);
    assert_eq!(config.daily_loss_cap, 0.03);
    assert_eq!(config.capital_cap, Some(10_000.0));
    assert_eq!(config.candle_delay_s, 60);
    assert_eq!(config.poll_s, 60);
    assert_eq!(config.history, 1000);
    assert_eq!(config.sim_fee, 0.001);
    assert_eq!(config.sim_slippage, 0.0005);
    assert_eq!(config.sim_start_usdt, 10_000.0);
    assert_eq!(config.state_dir, PathBuf::from("state"));
    assert_eq!(config.log_dir, PathBuf::from("logs"));
    assert_eq!(config.kill_file(), PathBuf::from("state").join("KILL"));
    assert_eq!(config.pause_file(), PathBuf::from("state").join("PAUSE"));
    assert_eq!(ATR_PERIOD, 14);

    assert!(config.validate().is_ok());
}

#[test]
fn parses_partial_toml_and_keeps_python_defaults() {
    let config = BotConfig::from_toml_str(
        r#"
        timeframe = "1h"
        risk_per_trade = 0.02

        [[symbols]]
        symbol = "SOL/USDT"
        entry_n = 20
        exit_n = 10
        "#,
    )
    .unwrap();

    assert_eq!(config.symbols.len(), 1);
    assert_eq!(config.symbols[0].symbol.to_string(), "SOL/USDT");
    assert_eq!(config.symbols[0].entry_n, 20);
    assert_eq!(config.symbols[0].exit_n, 10);
    assert_eq!(config.symbols[0].stop_mult, 2.0);
    assert_eq!(config.timeframe, TimeFrame::OneHour);
    assert_eq!(config.risk_per_trade, 0.02);
    assert_eq!(config.daily_loss_cap, 0.03);
    assert_eq!(config.capital_cap, Some(10_000.0));
    assert_eq!(config.history, 1000);
    assert_eq!(config.state_dir, PathBuf::from("state"));
    assert_eq!(config.log_dir, PathBuf::from("logs"));
}

#[test]
fn parses_explicit_capital_cap_override() {
    let config = BotConfig::from_toml_str("capital_cap = 5000.0").unwrap();
    assert_eq!(config.capital_cap, Some(5_000.0));
}

#[test]
fn allows_disabling_capital_cap_programmatically() {
    let config = config_with(|config| config.capital_cap = None);
    assert!(config.validate().is_ok());
}

#[test]
fn rejects_unsupported_timeframe() {
    let error = BotConfig::from_toml_str(r#"timeframe = "2h""#).unwrap_err();
    assert!(matches!(error, ConfigError::Parse(_)));
}

#[test]
fn rejects_zero_lookbacks() {
    let error = BotConfig::from_toml_str(
        r#"
        [[symbols]]
        symbol = "ETH/USDT"
        entry_n = 0
        exit_n = 50
        "#,
    )
    .unwrap_err();
    assert!(matches!(
        error,
        ConfigError::Validation(ValidationError::InvalidEntryLookback { value: 0, .. })
    ));

    let error = BotConfig::from_toml_str(
        r#"
        [[symbols]]
        symbol = "ETH/USDT"
        entry_n = 55
        exit_n = 0
        "#,
    )
    .unwrap_err();
    assert!(matches!(
        error,
        ConfigError::Validation(ValidationError::InvalidExitLookback { value: 0, .. })
    ));
}

#[test]
fn rejects_negative_lookbacks_at_parse_time() {
    let error = BotConfig::from_toml_str(
        r#"
        [[symbols]]
        symbol = "ETH/USDT"
        entry_n = -1
        exit_n = 50
        "#,
    )
    .unwrap_err();
    assert!(matches!(error, ConfigError::Parse(_)));

    let error = BotConfig::from_toml_str(
        r#"
        [[symbols]]
        symbol = "ETH/USDT"
        entry_n = 55
        exit_n = -1
        "#,
    )
    .unwrap_err();
    assert!(matches!(error, ConfigError::Parse(_)));
}

#[test]
fn rejects_entry_lookback_not_greater_than_exit_lookback() {
    for (entry_n, exit_n) in [(50, 50), (40, 50)] {
        let config = config_with(|config| {
            config.symbols = vec![symbol_config("ETH/USDT", entry_n, exit_n)];
        });

        assert!(matches!(
            config.validate(),
            Err(ValidationError::EntryNotGreaterThanExit { .. })
        ));
    }
}

#[test]
fn rejects_invalid_risk_and_capital_values() {
    assert!(matches!(
        config_with(|config| config.risk_per_trade = 0.0).validate(),
        Err(ValidationError::InvalidRiskPerTrade(value)) if value == 0.0
    ));
    assert!(matches!(
        config_with(|config| config.risk_per_trade = -0.1).validate(),
        Err(ValidationError::InvalidRiskPerTrade(_))
    ));
    assert!(matches!(
        config_with(|config| config.risk_per_trade = 1.1).validate(),
        Err(ValidationError::InvalidRiskPerTrade(_))
    ));
    assert!(matches!(
        config_with(|config| config.risk_per_trade = f64::NAN).validate(),
        Err(ValidationError::NonFinite {
            field: "risk_per_trade",
            ..
        })
    ));

    assert!(matches!(
        config_with(|config| config.daily_loss_cap = 0.0).validate(),
        Err(ValidationError::InvalidDailyLossCap(_))
    ));
    assert!(matches!(
        config_with(|config| config.daily_loss_cap = 1.1).validate(),
        Err(ValidationError::InvalidDailyLossCap(_))
    ));
    assert!(matches!(
        config_with(|config| config.daily_loss_cap = f64::INFINITY).validate(),
        Err(ValidationError::NonFinite {
            field: "daily_loss_cap",
            ..
        })
    ));

    assert!(matches!(
        config_with(|config| config.capital_cap = Some(0.0)).validate(),
        Err(ValidationError::InvalidCapitalCap(value)) if value == 0.0
    ));
    assert!(matches!(
        config_with(|config| config.capital_cap = Some(-1.0)).validate(),
        Err(ValidationError::InvalidCapitalCap(_))
    ));
    assert!(matches!(
        config_with(|config| config.capital_cap = Some(f64::NAN)).validate(),
        Err(ValidationError::NonFinite {
            field: "capital_cap",
            ..
        })
    ));
}

#[test]
fn rejects_non_finite_symbol_and_simulation_values() {
    assert!(matches!(
        config_with(|config| config.symbols[0].stop_mult = f64::NAN).validate(),
        Err(ValidationError::NonFinite {
            field: "stop_mult",
            ..
        })
    ));
    assert!(matches!(
        config_with(|config| config.sim_fee = f64::INFINITY).validate(),
        Err(ValidationError::NonFinite {
            field: "sim_fee",
            ..
        })
    ));
    assert!(matches!(
        config_with(|config| config.sim_slippage = f64::NAN).validate(),
        Err(ValidationError::NonFinite {
            field: "sim_slippage",
            ..
        })
    ));
    assert!(matches!(
        config_with(|config| config.sim_start_usdt = f64::NAN).validate(),
        Err(ValidationError::NonFinite {
            field: "sim_start_usdt",
            ..
        })
    ));
}

#[test]
fn rejects_invalid_fee_slippage_and_starting_cash() {
    assert!(matches!(
        config_with(|config| config.sim_fee = -0.1).validate(),
        Err(ValidationError::InvalidFee(_))
    ));
    assert!(matches!(
        config_with(|config| config.sim_fee = 1.0).validate(),
        Err(ValidationError::InvalidFee(_))
    ));

    assert!(matches!(
        config_with(|config| config.sim_slippage = -0.1).validate(),
        Err(ValidationError::InvalidSlippage(_))
    ));
    assert!(matches!(
        config_with(|config| config.sim_slippage = 1.0).validate(),
        Err(ValidationError::InvalidSlippage(_))
    ));

    assert!(matches!(
        config_with(|config| config.sim_start_usdt = 0.0).validate(),
        Err(ValidationError::InvalidSimStart(value)) if value == 0.0
    ));
    assert!(matches!(
        config_with(|config| config.sim_start_usdt = -1.0).validate(),
        Err(ValidationError::InvalidSimStart(_))
    ));
}

#[test]
fn rejects_history_too_short_for_the_configured_lookbacks() {
    let config = config_with(|config| {
        config.symbols = vec![symbol_config("ETH/USDT", 55, 50)];
        config.history = 55;
    });

    assert!(matches!(
        config.validate(),
        Err(ValidationError::InvalidHistory {
            history: 55,
            required: 56
        })
    ));
}

#[test]
fn rejects_history_shorter_than_atr_warmup() {
    let config = config_with(|config| {
        config.symbols = vec![symbol_config("ETH/USDT", 10, 5)];
        config.history = 13;
    });

    assert!(matches!(
        config.validate(),
        Err(ValidationError::InvalidHistory {
            history: 13,
            required: 14
        })
    ));

    let config = config_with(|config| {
        config.symbols = vec![symbol_config("ETH/USDT", 10, 5)];
        config.history = 14;
    });
    assert!(config.validate().is_ok());
}

#[test]
fn rejects_duplicate_symbols() {
    let config = config_with(|config| {
        let duplicate = config.symbols[0].clone();
        config.symbols.push(duplicate);
    });

    assert!(matches!(
        config.validate(),
        Err(ValidationError::DuplicateSymbol(_))
    ));
}

#[test]
fn rejects_symbols_with_a_different_quote_asset() {
    let config = config_with(|config| {
        config.symbols[0].symbol = SymbolId::parse("ETH/BTC").unwrap();
    });

    assert!(matches!(
        config.validate(),
        Err(ValidationError::QuoteMismatch { .. })
    ));
}

#[test]
fn rejects_empty_symbols_quote_and_directories() {
    assert!(matches!(
        config_with(|config| config.symbols.clear()).validate(),
        Err(ValidationError::EmptySymbols)
    ));
    assert!(matches!(
        config_with(|config| config.quote.clear()).validate(),
        Err(ValidationError::EmptyQuote)
    ));
    assert!(matches!(
        config_with(|config| config.quote = "usdt".to_owned()).validate(),
        Err(ValidationError::InvalidQuote(_))
    ));
    assert!(matches!(
        config_with(|config| config.state_dir = PathBuf::new()).validate(),
        Err(ValidationError::EmptyStateDir)
    ));
    assert!(matches!(
        config_with(|config| config.log_dir = PathBuf::new()).validate(),
        Err(ValidationError::EmptyLogDir)
    ));
}

#[test]
fn rejects_zero_poll_interval() {
    assert!(matches!(
        config_with(|config| config.poll_s = 0).validate(),
        Err(ValidationError::InvalidPollInterval)
    ));
}

#[test]
fn rejects_non_finite_values_from_toml() {
    let error = BotConfig::from_toml_str("risk_per_trade = nan").unwrap_err();
    assert!(matches!(
        error,
        ConfigError::Validation(ValidationError::NonFinite {
            field: "risk_per_trade",
            ..
        })
    ));

    let error = BotConfig::from_toml_str("capital_cap = inf").unwrap_err();
    assert!(matches!(
        error,
        ConfigError::Validation(ValidationError::NonFinite {
            field: "capital_cap",
            ..
        })
    ));
}

#[test]
fn rejects_unknown_toml_fields() {
    let error = BotConfig::from_toml_str("unknown_field = 1").unwrap_err();
    assert!(matches!(error, ConfigError::Parse(_)));
}

#[test]
fn debug_output_does_not_expose_secrets() {
    let rendered = format!("{:?}", BotConfig::default()).to_ascii_lowercase();

    for forbidden in [
        "secret",
        "api_key",
        "apikey",
        "password",
        "token",
        "private_key",
    ] {
        assert!(
            !rendered.contains(forbidden),
            "debug output unexpectedly contained {forbidden:?}"
        );
    }
}
