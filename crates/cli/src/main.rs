use std::{collections::HashMap, env, error::Error, fs, path::PathBuf, thread, time::Duration};

use chrono::Utc;
use trading_engine_binance::{BinancePublicClient, BinanceTestnetClient, Decimal, OrderValidator};
use trading_engine_config::{BotConfig, SymbolConfig};
use trading_engine_core::{SymbolId, Timestamp};
use trading_engine_live::{LiveStepResult, RiskContext, RuntimeEnvironment, TestnetEngine};
use trading_engine_persistence::{Journal, StateStore};
use trading_engine_risk::compute_equity;
use trading_engine_strategies::{Donchian, Strategy};

type AnyResult<T> = Result<T, Box<dyn Error>>;

fn main() {
    if let Err(error) = run() {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
}

fn run() -> AnyResult<()> {
    let mut args = env::args().skip(1);
    let Some(command) = args.next() else {
        print_usage();
        return Ok(());
    };
    let rest: Vec<String> = args.collect();
    match command.as_str() {
        "help" | "--help" | "-h" => print_usage(),
        "validate-config" => {
            let path = rest
                .first()
                .map(PathBuf::from)
                .unwrap_or_else(|| "config.toml".into());
            let config = read_config(&path)?;
            println!(
                "configuration valid: {} symbols, {} timeframe",
                config.symbols.len(),
                config.timeframe
            );
        }
        "status" => show_status(config_path(&rest))?,
        "pause" | "kill" | "resume" | "stop" => control(&command, config_path(&rest))?,
        "account" | "open-orders" => {
            require_testnet(&rest)?;
            let client = BinanceTestnetClient::from_env()?;
            if command == "account" {
                let account = client.account()?;
                println!("Spot Testnet account can_trade={}", account.can_trade);
                println!("asset\tfree\tlocked");
                for balance in account
                    .balances
                    .iter()
                    .filter(|balance| balance.free != "0" || balance.locked != "0")
                {
                    println!("{}\t{}\t{}", balance.asset, balance.free, balance.locked);
                }
            } else {
                let orders = client.open_orders(None)?;
                if orders.is_empty() {
                    println!("no open Spot Testnet orders");
                } else {
                    println!("symbol\torder_id\tclient_order_id\tstatus\texecuted_qty");
                    for order in orders {
                        println!(
                            "{}\t{}\t{}\t{:?}\t{}",
                            order.symbol,
                            order.order_id,
                            order.client_order_id,
                            order.status,
                            order.executed_qty
                        );
                    }
                }
            }
        }
        "reconcile" => {
            require_testnet(&rest)?;
            let config = read_config(&config_path(&rest))?;
            let mut engine = build_engine(&config)?;
            let account = engine.account()?;
            let findings = engine.reconcile_startup(&total_balances(&account.balances)?)?;
            if findings.is_empty() {
                println!("Testnet orders and tracked positions reconciled without discrepancies");
            } else {
                for finding in findings {
                    println!("reconciliation: {finding}");
                }
            }
        }
        "candles" => {
            require_testnet(&rest)?;
            let symbol_text = rest
                .first()
                .ok_or("usage: trading-engine candles SYMBOL --environment testnet")?;
            let symbol = SymbolId::parse(symbol_text)?;
            let public = BinancePublicClient::new()?;
            let now = Timestamp::from_millis(Utc::now().timestamp_millis())?;
            let bars = public.completed_klines(&symbol, config_timeframe(&rest)?, 30, now)?;
            println!("completed candles for {symbol}");
            println!("open_time_utc\topen\thigh\tlow\tclose\tvolume");
            for bar in bars {
                println!(
                    "{}\t{}\t{}\t{}\t{}\t{}",
                    bar.open_time(),
                    bar.open(),
                    bar.high(),
                    bar.low(),
                    bar.close(),
                    bar.volume()
                );
            }
        }
        "rules" => {
            require_testnet(&rest)?;
            let symbol = SymbolId::parse(
                rest.first()
                    .ok_or("usage: trading-engine rules SYMBOL --environment testnet")?,
            )?;
            let client = BinancePublicClient::new()?;
            let info = client.exchange_info(Some(&symbol))?;
            let metadata = info
                .symbols
                .first()
                .ok_or("exchange returned no metadata for symbol")?;
            println!(
                "{} status={} base={} quote={}",
                metadata.symbol, metadata.status, metadata.base_asset, metadata.quote_asset
            );
            for filter in &metadata.filters {
                println!("{filter}");
            }
        }
        "run" => {
            require_testnet(&rest)?;
            let config = read_config(&config_path(&rest))?;
            run_engine(config)?;
        }
        other => return Err(format!("unknown command: {other}").into()),
    }
    Ok(())
}

fn require_testnet(args: &[String]) -> AnyResult<()> {
    if args
        .windows(2)
        .any(|pair| pair[0] == "--environment" && pair[1] == "testnet")
    {
        Ok(())
    } else {
        Err("pass --environment testnet explicitly; this application has no production mode".into())
    }
}

fn config_path(args: &[String]) -> PathBuf {
    args.iter()
        .find(|arg| !arg.starts_with('-'))
        .map(PathBuf::from)
        .unwrap_or_else(|| "config.toml".into())
}

fn config_timeframe(args: &[String]) -> AnyResult<trading_engine_core::TimeFrame> {
    let interval = args
        .windows(2)
        .find(|pair| pair[0] == "--interval")
        .map(|pair| pair[1].as_str())
        .unwrap_or("4h");
    Ok(trading_engine_core::TimeFrame::parse(interval)?)
}

fn read_config(path: &PathBuf) -> AnyResult<BotConfig> {
    let source = fs::read_to_string(path)?;
    Ok(BotConfig::from_toml_str(&source)?)
}

fn control(command: &str, path: PathBuf) -> AnyResult<()> {
    let config = read_config(&path)?;
    fs::create_dir_all(&config.state_dir)?;
    let pause = config.state_dir.join("PAUSE");
    let kill = config.state_dir.join("KILL");
    let stop = config.state_dir.join("STOP");
    match command {
        "pause" => fs::write(pause, "new entries paused\n")?,
        "kill" => fs::write(kill, "new entries killed\n")?,
        "stop" => fs::write(stop, "stop requested; keep protective exits running\n")?,
        "resume" => {
            let state_path = config.state_dir.join("testnet-state.json");
            let mut state = StateStore::open(&state_path)?;
            if let Some(exchange) = state.state.exchange.as_ref() {
                let unresolved_order = exchange.orders.values().any(|order| {
                    order.discrepancy.is_some()
                        || matches!(
                            order.status,
                            trading_engine_persistence::PersistedOrderStatus::Prepared
                                | trading_engine_persistence::PersistedOrderStatus::Submitting
                                | trading_engine_persistence::PersistedOrderStatus::Unknown
                                | trading_engine_persistence::PersistedOrderStatus::New
                                | trading_engine_persistence::PersistedOrderStatus::PartiallyFilled
                        )
                });
                if !exchange.untracked_open_orders.is_empty()
                    || !exchange.discrepancies.is_empty()
                    || unresolved_order
                {
                    return Err("cannot resume while Testnet reconciliation issues remain; inspect status and reconcile first".into());
                }
            }
            for marker in [pause, kill, stop] {
                if marker.exists() {
                    fs::remove_file(marker)?;
                }
            }
            state.state.halted = false;
            state.save()?;
        }
        _ => unreachable!(),
    }
    println!(
        "{command} control updated under {}",
        config.state_dir.display()
    );
    Ok(())
}

fn show_status(path: PathBuf) -> AnyResult<()> {
    let config = read_config(&path)?;
    let store = StateStore::open(config.state_dir.join("testnet-state.json"))?;
    println!(
        "halted={} pause={} kill={} stop_requested={}",
        store.state.halted,
        config.state_dir.join("PAUSE").exists(),
        config.state_dir.join("KILL").exists(),
        config.state_dir.join("STOP").exists()
    );
    println!("open positions: {}", store.state.positions.len());
    for (symbol, position) in &store.state.positions {
        println!(
            "{symbol}\tqty={} entry={} stop={}",
            position.qty, position.entry, position.stop
        );
    }
    if let Some(exchange) = store.state.exchange {
        println!("exchange orders: {}", exchange.orders.len());
        for order in exchange.orders.values() {
            println!(
                "{}\t{}\t{:?}\texecuted={}",
                order.client_order_id, order.symbol, order.status, order.executed_qty
            );
        }
        for discrepancy in exchange.discrepancies {
            println!(
                "reconciliation warning: {} local={} exchange={} ({})",
                discrepancy.symbol,
                discrepancy.local_quantity,
                discrepancy.exchange_quantity,
                discrepancy.reason
            );
        }
        for order in exchange.untracked_open_orders {
            println!("untracked exchange order: {order}");
        }
    }
    Ok(())
}

fn run_engine(config: BotConfig) -> AnyResult<()> {
    let public = BinancePublicClient::new()?;
    let mut engine = build_engine(&config)?;
    let account = engine.account()?;
    if !account.can_trade {
        eprintln!(
            "Spot Testnet account reports trading disabled; new entries will remain blocked."
        );
    }
    let total_balances = total_balances(&account.balances)?;
    let findings = engine.reconcile_startup(&total_balances)?;
    for finding in findings {
        eprintln!("reconciliation: {finding}");
    }
    println!(
        "Spot Testnet loop started; pause/kill block entries while protective exits continue."
    );

    loop {
        for symbol_config in &config.symbols {
            let account = engine.account()?;
            let marks = ticker_marks(&public, &config.symbols)?;
            let mark = marks
                .get(&symbol_config.symbol)
                .copied()
                .ok_or("missing configured ticker mark")?;
            let account_equity = marked_equity(&config, engine.state(), &account.balances, &marks)?;
            let free = free_balances(&account.balances)?;
            let delay_seconds = i64::try_from(config.candle_delay_s).unwrap_or(i64::MAX / 1000);
            let decision_time = Utc::now()
                .timestamp_millis()
                .saturating_sub(delay_seconds.saturating_mul(1000));
            let now = Timestamp::from_millis(decision_time)?;
            let limit = config.history.min(1000) as u16;
            let candles =
                public.completed_klines(&symbol_config.symbol, config.timeframe, limit, now)?;
            let context = RiskContext {
                reference_price: Decimal::parse(&mark.to_string())?,
                equity: account_equity,
                free_balances: free,
                today_utc: Utc::now().date_naive(),
                pause_active: config.state_dir.join("PAUSE").exists()
                    || config.state_dir.join("STOP").exists(),
                kill_active: config.state_dir.join("KILL").exists() || !account.can_trade,
            };
            match engine.step(&symbol_config.symbol, &candles, &context)? {
                LiveStepResult::NoAction
                | LiveStepResult::DuplicateCandle
                | LiveStepResult::Warmup => {}
                result => println!("{}: {result:?}", symbol_config.symbol),
            }
        }
        if config.state_dir.join("STOP").exists() && engine.state().positions.is_empty() {
            println!("stop complete: no tracked positions remain");
            break;
        }
        thread::sleep(Duration::from_secs(config.poll_s));
    }
    Ok(())
}

fn build_engine(config: &BotConfig) -> AnyResult<TestnetEngine<BinanceTestnetClient>> {
    let client = BinanceTestnetClient::from_env()?;
    let mut metadata = Vec::new();
    for symbol_config in &config.symbols {
        metadata.extend(client.exchange_info(Some(&symbol_config.symbol))?.symbols);
    }
    let validator = OrderValidator::new(metadata.iter())?;
    let strategies = strategies(&config.symbols)?;
    fs::create_dir_all(&config.state_dir)?;
    fs::create_dir_all(&config.log_dir)?;
    let store = StateStore::open(config.state_dir.join("testnet-state.json"))?;
    let journal = Journal::open(config.log_dir.join("testnet-journal.csv"))?;
    let engine = TestnetEngine::open(
        RuntimeEnvironment::SpotTestnet,
        config.clone(),
        strategies,
        validator,
        client,
        store,
        journal,
    )?;
    Ok(engine)
}

fn strategies(symbols: &[SymbolConfig]) -> AnyResult<HashMap<SymbolId, Box<dyn Strategy>>> {
    let mut strategies: HashMap<SymbolId, Box<dyn Strategy>> = HashMap::new();
    for symbol in symbols {
        strategies.insert(
            symbol.symbol.clone(),
            Box::new(Donchian::new(
                symbol.entry_n,
                symbol.exit_n,
                symbol.stop_mult,
            )?),
        );
    }
    Ok(strategies)
}

fn ticker_marks(
    public: &BinancePublicClient,
    symbols: &[SymbolConfig],
) -> AnyResult<HashMap<SymbolId, f64>> {
    symbols
        .iter()
        .map(|item| Ok((item.symbol.clone(), public.ticker_price(&item.symbol)?)))
        .collect()
}

fn free_balances(
    balances: &[trading_engine_binance::AssetBalance],
) -> AnyResult<HashMap<String, Decimal>> {
    balances
        .iter()
        .map(|balance| Ok((balance.asset.clone(), Decimal::parse(&balance.free)?)))
        .collect()
}

fn total_balances(
    balances: &[trading_engine_binance::AssetBalance],
) -> AnyResult<HashMap<String, Decimal>> {
    balances
        .iter()
        .map(|balance| {
            let free = Decimal::parse(&balance.free)?;
            let locked = Decimal::parse(&balance.locked)?;
            Ok((balance.asset.clone(), free.checked_add(locked)?))
        })
        .collect()
}

fn marked_equity(
    config: &BotConfig,
    state: &trading_engine_persistence::BotState,
    balances: &[trading_engine_binance::AssetBalance],
    marks: &HashMap<SymbolId, f64>,
) -> AnyResult<f64> {
    let total = total_balances(balances)?;
    let quote = total
        .get(&config.quote)
        .copied()
        .unwrap_or_default()
        .to_string()
        .parse::<f64>()?;
    let mut position_marks = Vec::new();
    for symbol_config in &config.symbols {
        let symbol = &symbol_config.symbol;
        let qty = total
            .get(symbol.base())
            .copied()
            .unwrap_or_default()
            .to_string()
            .parse::<f64>()?;
        let mark = *marks.get(symbol).ok_or("missing ticker mark")?;
        let entry = state
            .positions
            .get(&symbol.to_string())
            .map_or(mark, |position| position.entry);
        position_marks.push(trading_engine_risk::OpenPositionMark {
            quantity: qty,
            entry_price: entry,
            mark_price: mark,
        });
    }
    let realized: f64 = state.closed.iter().map(|trade| trade.pnl).sum();
    Ok(compute_equity(
        quote,
        &position_marks,
        realized,
        config.capital_cap,
    )?)
}

fn print_usage() {
    println!("trading-engine validate-config [config.toml]");
    println!("trading-engine account|open-orders --environment testnet");
    println!("trading-engine reconcile --environment testnet [config.toml]");
    println!("trading-engine rules SYMBOL --environment testnet");
    println!("trading-engine candles SYMBOL --interval 1h|4h --environment testnet");
    println!("trading-engine run --environment testnet [config.toml]");
    println!("trading-engine status|pause|kill|stop|resume [config.toml]");
}

#[cfg(test)]
mod tests {
    use super::*;
    use trading_engine_persistence::{ExchangeState, ReconciliationDiscrepancy};

    #[test]
    fn example_config_is_valid_and_exchange_requires_explicit_testnet() {
        let config = BotConfig::from_toml_str(include_str!("../../../config.example.toml"))
            .expect("checked-in example config should validate");
        assert_eq!(config.symbols.len(), 2);
        assert!(require_testnet(&["--environment".into(), "testnet".into()]).is_ok());
        assert!(require_testnet(&["--environment".into(), "production".into()]).is_err());
        assert!(require_testnet(&[]).is_err());
    }

    #[test]
    fn resume_refuses_to_clear_control_markers_with_unresolved_reconciliation() {
        let suffix = chrono::Utc::now().timestamp_nanos_opt().unwrap();
        let root = env::temp_dir().join(format!("testnet-cli-{suffix}"));
        let state_dir = root.join("state");
        let log_dir = root.join("logs");
        fs::create_dir_all(&state_dir).unwrap();
        fs::write(state_dir.join("PAUSE"), "paused").unwrap();
        let path_text = |path: &PathBuf| path.to_string_lossy().replace('\\', "/");
        let config_path = root.join("config.toml");
        fs::write(
            &config_path,
            format!(
                "symbols = [{{ symbol = \"ETH/USDT\", entry_n = 2, exit_n = 1 }}]\nstate_dir = \"{}\"\nlog_dir = \"{}\"\n",
                path_text(&state_dir),
                path_text(&log_dir)
            ),
        )
        .unwrap();
        let mut store = StateStore::open(state_dir.join("testnet-state.json")).unwrap();
        store.state.exchange = Some(ExchangeState {
            discrepancies: vec![ReconciliationDiscrepancy {
                symbol: "ETH/USDT".to_owned(),
                local_quantity: "1".to_owned(),
                exchange_quantity: "0".to_owned(),
                reason: "test mismatch".to_owned(),
                observed_at_ms: 1,
            }],
            ..ExchangeState::default()
        });
        store.save().unwrap();

        assert!(control("resume", config_path).is_err());
        assert!(state_dir.join("PAUSE").exists());
        let _ = fs::remove_dir_all(root);
    }
}
