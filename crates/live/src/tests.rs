use std::{collections::HashMap, env, fs};

use chrono::NaiveDate;
use trading_engine_binance::{BinanceTrade, Decimal, ExchangeSymbol, OrderStatus, OrderValidator};
use trading_engine_config::BotConfig;
use trading_engine_core::{Candle, Money, Side, SymbolId, Timestamp};
use trading_engine_persistence::{
    Journal, PersistedExchangeOrder, PersistedOrderStatus, StateStore,
};
use trading_engine_strategies::{Strategy, StrategyBar, StrategyError};

use crate::{
    LiveStepResult, RiskContext, RuntimeEnvironment, SpotTestnetExecution, SubmitFailure,
    TestnetEngine,
};

struct FixedStrategy {
    enter: bool,
    exit: bool,
}

impl Strategy for FixedStrategy {
    fn name(&self) -> &'static str {
        "test"
    }
    fn stop_mult(&self) -> f64 {
        2.0
    }
    fn trailing(&self) -> bool {
        false
    }
    fn signals(&self, candles: &[Candle]) -> Result<Vec<StrategyBar>, StrategyError> {
        Ok(candles
            .iter()
            .map(|_| StrategyBar {
                enter_long: self.enter,
                exit_long: self.exit,
                atr: Some(10.0),
            })
            .collect())
    }
}

#[derive(Default)]
struct FakeBroker {
    count: u64,
    submissions: Vec<(Side, String)>,
    trades: HashMap<u64, Vec<BinanceTrade>>,
    responses: HashMap<String, trading_engine_binance::OrderResponse>,
    open_order_snapshot: Vec<trading_engine_binance::OrderResponse>,
    ambiguous_next: bool,
    partial_next: bool,
    commission: Option<(String, String)>,
}

impl SpotTestnetExecution for FakeBroker {
    fn account(&mut self) -> Result<trading_engine_binance::AccountInfo, String> {
        Ok(trading_engine_binance::AccountInfo {
            balances: Vec::new(),
            can_trade: true,
        })
    }

    fn submit_market(
        &mut self,
        validator: &OrderValidator,
        symbol: &SymbolId,
        side: Side,
        quantity: &Decimal,
        reference_price: &Decimal,
        balances: &HashMap<String, Decimal>,
        client_order_id: &str,
    ) -> Result<trading_engine_binance::OrderResponse, SubmitFailure> {
        validator
            .validate_market_order(symbol, side, *quantity, *reference_price, balances)
            .map_err(|e| SubmitFailure {
                ambiguous: false,
                message: e.to_string(),
            })?;
        self.count += 1;
        let id = self.count;
        let fill_qty = if self.partial_next {
            Decimal::parse("0.25").unwrap()
        } else {
            *quantity
        };
        let status = if self.partial_next {
            "PARTIALLY_FILLED"
        } else {
            "FILLED"
        };
        self.partial_next = false;
        let (commission, commission_asset) = self
            .commission
            .clone()
            .unwrap_or_else(|| ("0.1".to_owned(), "USDT".to_owned()));
        self.submissions.push((side, client_order_id.to_owned()));
        self.trades.insert(
            id,
            vec![BinanceTrade {
                id,
                order_id: id,
                price: reference_price.to_string(),
                qty: fill_qty.to_string(),
                quote_qty: (fill_qty.to_string().parse::<f64>().unwrap()
                    * reference_price.to_string().parse::<f64>().unwrap())
                .to_string(),
                commission: commission.clone(),
                commission_asset: commission_asset.clone(),
                time: 1_735_675_200_000 + id,
                is_buyer: side == Side::Buy,
            }],
        );
        let response: trading_engine_binance::OrderResponse = serde_json::from_value(serde_json::json!({
            "symbol":symbol.binance_symbol(), "orderId":id, "clientOrderId":client_order_id,
            "status":status, "executedQty":fill_qty.to_string(), "cummulativeQuoteQty":self.trades[&id][0].quote_qty,
            "fills":[]
        })).unwrap();
        self.responses
            .insert(client_order_id.to_owned(), response.clone());
        if self.ambiguous_next {
            self.ambiguous_next = false;
            Err(SubmitFailure {
                ambiguous: true,
                message: "simulated response timeout".to_owned(),
            })
        } else {
            Ok(response)
        }
    }

    fn query_order(
        &mut self,
        _symbol: &SymbolId,
        client_order_id: &str,
    ) -> Result<trading_engine_binance::OrderResponse, String> {
        self.responses
            .get(client_order_id)
            .cloned()
            .ok_or_else(|| "order not found".to_owned())
    }

    fn cancel_order(
        &mut self,
        symbol: &SymbolId,
        client_order_id: &str,
    ) -> Result<trading_engine_binance::OrderResponse, String> {
        self.query_order(symbol, client_order_id)
    }

    fn open_orders(&mut self) -> Result<Vec<trading_engine_binance::OrderResponse>, String> {
        Ok(self.open_order_snapshot.clone())
    }

    fn my_trades(
        &mut self,
        _symbol: &SymbolId,
        order_id: u64,
    ) -> Result<Vec<BinanceTrade>, String> {
        Ok(self.trades.get(&order_id).cloned().unwrap_or_default())
    }
}

fn symbol() -> SymbolId {
    SymbolId::parse("ETH/USDT").unwrap()
}

fn candles(count: usize) -> Vec<Candle> {
    (0..count)
        .map(|i| {
            let time = Timestamp::from_millis(1_735_675_200_000 + i as i64 * 14_400_000).unwrap();
            Candle::new(
                time,
                Money::new(2_000.0).unwrap(),
                Money::new(2_010.0).unwrap(),
                Money::new(1_990.0).unwrap(),
                Money::new(2_000.0).unwrap(),
                10.0,
            )
            .unwrap()
        })
        .collect()
}

fn rules() -> OrderValidator {
    let symbol: ExchangeSymbol = serde_json::from_value(serde_json::json!({
        "symbol":"ETHUSDT", "status":"TRADING", "baseAsset":"ETH", "quoteAsset":"USDT",
        "filters":[
            {"filterType":"PRICE_FILTER","minPrice":"0.01","maxPrice":"1000000","tickSize":"0.01"},
            {"filterType":"LOT_SIZE","minQty":"0.001","maxQty":"1000","stepSize":"0.001"},
            {"filterType":"MARKET_LOT_SIZE","minQty":"0.001","maxQty":"1000","stepSize":"0.001"},
            {"filterType":"MIN_NOTIONAL","minNotional":"5","applyToMarket":true}
        ]
    }))
    .unwrap();
    OrderValidator::new([&symbol]).unwrap()
}

fn rules_for(symbols: &[&str]) -> OrderValidator {
    let metadata: Vec<ExchangeSymbol> = symbols
        .iter()
        .map(|symbol| {
            let id = SymbolId::parse(symbol).unwrap();
            serde_json::from_value(serde_json::json!({
                "symbol":id.binance_symbol(), "status":"TRADING", "baseAsset":id.base(), "quoteAsset":id.quote(),
                "filters":[
                    {"filterType":"PRICE_FILTER","minPrice":"0.01","maxPrice":"1000000","tickSize":"0.01"},
                    {"filterType":"LOT_SIZE","minQty":"0.001","maxQty":"1000","stepSize":"0.001"},
                    {"filterType":"MARKET_LOT_SIZE","minQty":"0.001","maxQty":"1000","stepSize":"0.001"},
                    {"filterType":"MIN_NOTIONAL","minNotional":"5","applyToMarket":true}
                ]
            }))
            .unwrap()
        })
        .collect();
    OrderValidator::new(metadata.iter()).unwrap()
}

fn engine(enter: bool) -> (TestnetEngine<FakeBroker>, std::path::PathBuf) {
    let dir = env::temp_dir().join(format!(
        "live-engine-{}-{}",
        std::process::id(),
        chrono::Utc::now().timestamp_nanos_opt().unwrap()
    ));
    fs::create_dir_all(&dir).unwrap();
    let config = BotConfig {
        symbols: vec![trading_engine_config::SymbolConfig::new(
            symbol(),
            2,
            1,
            2.0,
        )],
        history: 14,
        capital_cap: Some(10_000.0),
        ..BotConfig::default()
    };
    let strategies = HashMap::from([(
        symbol(),
        Box::new(FixedStrategy { enter, exit: false }) as Box<dyn Strategy>,
    )]);
    let store = StateStore::open(dir.join("state.json")).unwrap();
    let journal = Journal::open(dir.join("journal.csv")).unwrap();
    let engine = TestnetEngine::open(
        RuntimeEnvironment::SpotTestnet,
        config,
        strategies,
        rules(),
        FakeBroker::default(),
        store,
        journal,
    )
    .unwrap();
    (engine, dir)
}

fn context(price: &str, usdt: &str, eth: &str, pause: bool, kill: bool) -> RiskContext {
    RiskContext {
        reference_price: Decimal::parse(price).unwrap(),
        equity: 1_000.0,
        free_balances: HashMap::from([
            ("USDT".to_owned(), Decimal::parse(usdt).unwrap()),
            ("ETH".to_owned(), Decimal::parse(eth).unwrap()),
        ]),
        today_utc: NaiveDate::from_ymd_opt(2024, 12, 31).unwrap(),
        pause_active: pause,
        kill_active: kill,
    }
}

#[test]
fn entry_passes_strategy_risk_validation_fills_position_persists_and_deduplicates() {
    let (mut engine, dir) = engine(true);
    let symbol = symbol();
    let result = engine
        .step(
            &symbol,
            &candles(1),
            &context("2000", "1000", "0", false, false),
        )
        .unwrap();
    assert!(matches!(
        result,
        LiveStepResult::OrderSubmitted {
            status: OrderStatus::Filled,
            ..
        }
    ));
    let position = &engine.state().positions["ETH/USDT"];
    assert!((position.qty - 0.497).abs() < 1e-9);
    assert!(position.stop < position.entry);
    assert_eq!(engine.state().exchange.as_ref().unwrap().orders.len(), 1);
    assert_eq!(
        engine
            .step(
                &symbol,
                &candles(1),
                &context("2000", "1000", "0", false, false)
            )
            .unwrap(),
        LiveStepResult::DuplicateCandle
    );
    assert_eq!(engine.broker.submissions.len(), 1);
    let journal = fs::read_to_string(dir.join("journal.csv")).unwrap();
    assert_eq!(journal.lines().count(), 2);
}

#[test]
fn partial_fills_increment_position_once_and_reconcile_later_fill() {
    let (mut engine, _) = engine(true);
    engine.broker.partial_next = true;
    let result = engine
        .step(
            &symbol(),
            &candles(1),
            &context("2000", "1000", "0", false, false),
        )
        .unwrap();
    assert!(matches!(
        result,
        LiveStepResult::OrderSubmitted {
            status: OrderStatus::PartiallyFilled,
            ..
        }
    ));
    assert!((engine.state().positions["ETH/USDT"].qty - 0.25).abs() < 1e-9);
    let client_id = engine
        .state()
        .exchange
        .as_ref()
        .unwrap()
        .orders
        .keys()
        .next()
        .unwrap()
        .clone();
    let order_id = engine.state().exchange.as_ref().unwrap().orders[&client_id]
        .exchange_order_id
        .as_ref()
        .unwrap()
        .parse::<u64>()
        .unwrap();

    engine
        .reconcile_startup(&HashMap::from([(
            "ETH".to_owned(),
            Decimal::parse("0.25").unwrap(),
        )]))
        .unwrap();
    assert!((engine.state().positions["ETH/USDT"].qty - 0.25).abs() < 1e-9);

    let mut fills = engine.broker.trades[&order_id].clone();
    fills.push(BinanceTrade {
        id: 99,
        order_id,
        price: "2000".to_owned(),
        qty: "0.247".to_owned(),
        quote_qty: "494".to_owned(),
        commission: "0.1".to_owned(),
        commission_asset: "USDT".to_owned(),
        time: 1_735_675_200_099,
        is_buyer: true,
    });
    engine.broker.trades.insert(order_id, fills);
    let response: trading_engine_binance::OrderResponse =
        serde_json::from_value(serde_json::json!({
            "symbol":"ETHUSDT", "orderId":order_id, "clientOrderId":client_id,
            "status":"FILLED", "executedQty":"0.497", "cummulativeQuoteQty":"994", "fills":[]
        }))
        .unwrap();
    engine.broker.responses.insert(client_id.clone(), response);

    assert!(engine
        .reconcile_startup(&HashMap::from([(
            "ETH".to_owned(),
            Decimal::parse("0.497").unwrap(),
        )]))
        .unwrap()
        .is_empty());
    let position = &engine.state().positions["ETH/USDT"];
    assert!((position.qty - 0.497).abs() < 1e-9);
    let order = &engine.state().exchange.as_ref().unwrap().orders[&client_id];
    assert_eq!(
        order.status,
        trading_engine_persistence::PersistedOrderStatus::Filled
    );
    assert_eq!(order.fills.len(), 2);
}

#[test]
fn base_asset_buy_commission_reduces_received_quantity_without_double_counting_cost() {
    let (mut engine, _) = engine(true);
    engine.broker.commission = Some(("0.001".to_owned(), "ETH".to_owned()));
    engine
        .step(
            &symbol(),
            &candles(1),
            &context("2000", "1000", "0", false, false),
        )
        .unwrap();

    let position = &engine.state().positions["ETH/USDT"];
    assert!((position.qty - 0.496).abs() < 1e-9);
    assert!((position.entry - (994.0 / 0.496)).abs() < 1e-8);
}

#[test]
fn engine_tracks_independent_positions_for_multiple_symbols() {
    let dir = env::temp_dir().join(format!(
        "live-multi-symbol-{}-{}",
        std::process::id(),
        chrono::Utc::now().timestamp_nanos_opt().unwrap()
    ));
    fs::create_dir_all(&dir).unwrap();
    let btc = SymbolId::parse("BTC/USDT").unwrap();
    let config = BotConfig {
        symbols: vec![
            trading_engine_config::SymbolConfig::new(symbol(), 2, 1, 2.0),
            trading_engine_config::SymbolConfig::new(btc.clone(), 2, 1, 2.0),
        ],
        history: 14,
        capital_cap: Some(10_000.0),
        ..BotConfig::default()
    };
    let strategies = HashMap::from([
        (
            symbol(),
            Box::new(FixedStrategy {
                enter: true,
                exit: false,
            }) as Box<dyn Strategy>,
        ),
        (
            btc.clone(),
            Box::new(FixedStrategy {
                enter: true,
                exit: false,
            }) as Box<dyn Strategy>,
        ),
    ]);
    let store = StateStore::open(dir.join("state.json")).unwrap();
    let journal = Journal::open(dir.join("journal.csv")).unwrap();
    let mut engine = TestnetEngine::open(
        RuntimeEnvironment::SpotTestnet,
        config,
        strategies,
        rules_for(&["ETH/USDT", "BTC/USDT"]),
        FakeBroker::default(),
        store,
        journal,
    )
    .unwrap();

    for sym in [symbol(), btc] {
        engine
            .step(
                &sym,
                &candles(1),
                &context("2000", "1000", "0", false, false),
            )
            .unwrap();
    }
    assert_eq!(engine.state().positions.len(), 2);
    assert!(engine.state().positions.contains_key("ETH/USDT"));
    assert!(engine.state().positions.contains_key("BTC/USDT"));
    assert_eq!(engine.broker.submissions.len(), 2);
}

#[test]
fn pause_and_kill_block_new_entries_but_not_stop_loss_exits() {
    for (pause, kill, label) in [(true, false, "pause file"), (false, true, "kill switch")] {
        let (mut engine, _) = engine(true);
        let result = engine
            .step(
                &symbol(),
                &candles(1),
                &context("2000", "1000", "0", pause, kill),
            )
            .unwrap();
        assert_eq!(result, LiveStepResult::EntryBlocked(label.to_owned()));
        assert!(engine.broker.submissions.is_empty());
    }

    let (mut engine, _) = engine(true);
    let sym = symbol();
    engine
        .step(
            &sym,
            &candles(1),
            &context("2000", "1000", "0", false, false),
        )
        .unwrap();
    let balance = engine.state().positions["ETH/USDT"].qty.to_string();
    let result = engine
        .step(
            &sym,
            &candles(2),
            &context("1900", "0", &balance, true, true),
        )
        .unwrap();
    assert!(matches!(
        result,
        LiveStepResult::OrderSubmitted {
            status: OrderStatus::Filled,
            ..
        }
    ));
    assert!(!engine.state().positions.contains_key("ETH/USDT"));
    assert_eq!(engine.state().closed.len(), 1);
    assert_eq!(engine.broker.submissions.len(), 2);
    assert_eq!(engine.broker.submissions[1].0, Side::Sell);
}

#[test]
fn restart_queries_ambiguous_client_id_restores_actual_fill_and_records_balance_conflict() {
    let (mut engine, dir) = engine(true);
    engine.broker.ambiguous_next = true;
    let result = engine
        .step(
            &symbol(),
            &candles(1),
            &context("2000", "1000", "0", false, false),
        )
        .unwrap();
    assert!(matches!(result, LiveStepResult::AmbiguousSubmission { .. }));
    assert!(engine.state().positions.is_empty());
    let broker = std::mem::take(&mut engine.broker);
    drop(engine);

    let config = BotConfig {
        symbols: vec![trading_engine_config::SymbolConfig::new(
            symbol(),
            2,
            1,
            2.0,
        )],
        history: 14,
        capital_cap: Some(10_000.0),
        ..BotConfig::default()
    };
    let strategies = HashMap::from([(
        symbol(),
        Box::new(FixedStrategy {
            enter: true,
            exit: false,
        }) as Box<dyn Strategy>,
    )]);
    let store = StateStore::open(dir.join("state.json")).unwrap();
    let journal = Journal::open(dir.join("journal.csv")).unwrap();
    let mut restarted = TestnetEngine::open(
        RuntimeEnvironment::SpotTestnet,
        config,
        strategies,
        rules(),
        broker,
        store,
        journal,
    )
    .unwrap();
    let actual_balance = HashMap::from([("ETH".to_owned(), Decimal::parse("0.497").unwrap())]);
    assert!(restarted
        .reconcile_startup(&actual_balance)
        .unwrap()
        .is_empty());
    assert!((restarted.state().positions["ETH/USDT"].qty - 0.497).abs() < 1e-9);

    let mismatch = HashMap::from([("ETH".to_owned(), Decimal::parse("0.3").unwrap())]);
    let findings = restarted.reconcile_startup(&mismatch).unwrap();
    assert_eq!(findings.len(), 1);
    assert_eq!(restarted.state().positions["ETH/USDT"].qty, 0.497);
    assert_eq!(
        restarted
            .state()
            .exchange
            .as_ref()
            .unwrap()
            .discrepancies
            .len(),
        1
    );
}

#[test]
fn untracked_exchange_open_order_halts_new_entries_and_is_reported() {
    let (mut engine, _) = engine(true);
    let open: trading_engine_binance::OrderResponse = serde_json::from_value(serde_json::json!({
        "symbol":"ETHUSDT", "orderId":91, "clientOrderId":"operator-order",
        "status":"NEW", "executedQty":"0", "cummulativeQuoteQty":"0", "fills":[]
    }))
    .unwrap();
    engine.broker.open_order_snapshot.push(open);

    let findings = engine.reconcile_startup(&HashMap::new()).unwrap();
    assert_eq!(findings.len(), 1);
    assert!(engine.state().halted);
    assert_eq!(
        engine
            .state()
            .exchange
            .as_ref()
            .unwrap()
            .untracked_open_orders
            .len(),
        1
    );
    let result = engine
        .step(
            &symbol(),
            &candles(1),
            &context("2000", "1000", "0", false, false),
        )
        .unwrap();
    assert!(matches!(result, LiveStepResult::EntryBlocked(_)));
    assert!(engine.broker.submissions.is_empty());
}

#[test]
fn restart_recovers_terminal_order_when_status_was_saved_before_fills() {
    let (mut engine, dir) = engine(true);
    engine
        .step(
            &symbol(),
            &candles(1),
            &context("2000", "1000", "0", false, false),
        )
        .unwrap();
    let broker = std::mem::take(&mut engine.broker);
    drop(engine);

    // Model the durable crash window after the terminal order response was
    // saved, but before the fill ledger and local position commit.
    let state_path = dir.join("state.json");
    let mut state: serde_json::Value =
        serde_json::from_slice(&fs::read(&state_path).unwrap()).unwrap();
    state["positions"] = serde_json::json!({});
    for order in state["exchange"]["orders"]
        .as_object_mut()
        .unwrap()
        .values_mut()
    {
        order["fills"] = serde_json::json!({});
    }
    fs::write(&state_path, serde_json::to_vec_pretty(&state).unwrap()).unwrap();
    let _ = fs::remove_file(dir.join("journal.csv"));

    let config = BotConfig {
        symbols: vec![trading_engine_config::SymbolConfig::new(
            symbol(),
            2,
            1,
            2.0,
        )],
        history: 14,
        capital_cap: Some(10_000.0),
        ..BotConfig::default()
    };
    let strategies = HashMap::from([(
        symbol(),
        Box::new(FixedStrategy {
            enter: true,
            exit: false,
        }) as Box<dyn Strategy>,
    )]);
    let store = StateStore::open(state_path).unwrap();
    let journal = Journal::open(dir.join("journal.csv")).unwrap();
    let mut restarted = TestnetEngine::open(
        RuntimeEnvironment::SpotTestnet,
        config,
        strategies,
        rules(),
        broker,
        store,
        journal,
    )
    .unwrap();
    let actual_balance = HashMap::from([("ETH".to_owned(), Decimal::parse("0.497").unwrap())]);
    assert!(restarted
        .reconcile_startup(&actual_balance)
        .unwrap()
        .is_empty());
    assert!((restarted.state().positions["ETH/USDT"].qty - 0.497).abs() < 1e-9);
    assert_eq!(restarted.state().exchange.as_ref().unwrap().orders.len(), 1);
}

#[test]
fn restart_closes_prepared_intent_without_posting_and_prevents_signal_replay() {
    let (mut original, dir) = engine(true);
    let broker = std::mem::take(&mut original.broker);
    drop(original);

    let mut store = StateStore::open(dir.join("state.json")).unwrap();
    store
        .record_exchange_intent(PersistedExchangeOrder {
            symbol: "ETH/USDT".to_owned(),
            side: "BUY".to_owned(),
            client_order_id: "prepared-but-not-submitted".to_owned(),
            requested_qty: "0.5".to_owned(),
            status: PersistedOrderStatus::Prepared,
            exchange_order_id: None,
            executed_qty: "0".to_owned(),
            cumulative_quote_qty: "0".to_owned(),
            fills: HashMap::new(),
            signal_candle: Some("1735675200000".to_owned()),
            atr: Some(10.0),
            stop_price: Some("1980".to_owned()),
            reason: Some("entry".to_owned()),
            created_at_ms: 1,
            last_reconciled_at_ms: None,
            discrepancy: None,
        })
        .unwrap();
    drop(store);

    let config = BotConfig {
        symbols: vec![trading_engine_config::SymbolConfig::new(
            symbol(),
            2,
            1,
            2.0,
        )],
        history: 14,
        capital_cap: Some(10_000.0),
        ..BotConfig::default()
    };
    let strategies = HashMap::from([(
        symbol(),
        Box::new(FixedStrategy {
            enter: true,
            exit: false,
        }) as Box<dyn Strategy>,
    )]);
    let store = StateStore::open(dir.join("state.json")).unwrap();
    let journal = Journal::open(dir.join("journal.csv")).unwrap();
    let mut restarted = TestnetEngine::open(
        RuntimeEnvironment::SpotTestnet,
        config,
        strategies,
        rules(),
        broker,
        store,
        journal,
    )
    .unwrap();
    let findings = restarted.reconcile_startup(&HashMap::new()).unwrap();
    assert_eq!(findings.len(), 1);
    assert_eq!(restarted.broker.submissions.len(), 0);
    assert_eq!(
        restarted
            .state()
            .last_candle
            .get("ETH/USDT")
            .map(String::as_str),
        Some("1735675200000")
    );
    let order = &restarted.state().exchange.as_ref().unwrap().orders["prepared-but-not-submitted"];
    assert_eq!(order.status, PersistedOrderStatus::Rejected);
    assert!(order.discrepancy.is_none());
}
