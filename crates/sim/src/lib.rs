//! Deterministic simulated paper execution at a caller-supplied ticker price.
//! This crate does not implement historical next-candle-open backtesting.

use std::collections::HashMap;

use chrono::NaiveDate;
use trading_engine_config::BotConfig;
use trading_engine_core::{Candle, Fill, Money, OrderIntent, Position, Side, SymbolId};
use trading_engine_persistence::{
    BotState, ClosedTradeRecord, Journal, JournalEntry, PendingOrder, PendingOrderKind,
    PersistedPosition, SimBrokerState, StateStore,
};
use trading_engine_risk::{
    compute_equity, evaluate_risk_gate, size_paper_position, OpenPositionMark, RiskError,
};
use trading_engine_strategies::{Strategy, StrategyError};

#[derive(Debug, Clone, PartialEq)]
pub enum StepResult {
    DuplicateCandle,
    Warmup,
    NoAction {
        equity: f64,
    },
    EntryBlocked {
        reason: String,
        equity: f64,
    },
    Entered {
        quantity: f64,
        price: f64,
        equity: f64,
    },
    Exited {
        quantity: f64,
        price: f64,
        reason: String,
        equity: f64,
    },
}

#[derive(Debug)]
pub enum SimError {
    Config(String),
    Strategy(StrategyError),
    Risk(RiskError),
    Persistence(trading_engine_persistence::PersistenceError),
    Domain(trading_engine_core::DomainError),
    MissingStrategy(SymbolId),
    InvalidTicker(f64),
    InvalidCandleOrder,
}

impl From<StrategyError> for SimError {
    fn from(e: StrategyError) -> Self {
        Self::Strategy(e)
    }
}
impl From<RiskError> for SimError {
    fn from(e: RiskError) -> Self {
        Self::Risk(e)
    }
}
impl From<trading_engine_persistence::PersistenceError> for SimError {
    fn from(e: trading_engine_persistence::PersistenceError) -> Self {
        Self::Persistence(e)
    }
}
impl From<trading_engine_core::DomainError> for SimError {
    fn from(e: trading_engine_core::DomainError) -> Self {
        Self::Domain(e)
    }
}

/// Single-process, single-writer paper simulator. The caller supplies each
/// symbol's completed candle history and the current public ticker price.
pub struct SimEngine {
    config: BotConfig,
    strategies: HashMap<SymbolId, Box<dyn Strategy>>,
    store: StateStore,
    journal: Journal,
}

impl SimEngine {
    pub fn open(
        config: BotConfig,
        strategies: HashMap<SymbolId, Box<dyn Strategy>>,
        store: StateStore,
        journal: Journal,
    ) -> Result<Self, SimError> {
        config
            .validate()
            .map_err(|e| SimError::Config(e.to_string()))?;
        let mut engine = Self {
            config,
            strategies,
            store,
            journal,
        };
        if engine.store.state.sim.is_none() {
            engine.store.state.sim = Some(SimBrokerState::new_with_quote(
                engine.config.quote.clone(),
                engine.config.sim_start_usdt,
            ));
            engine.store.save()?;
        }
        Ok(engine)
    }

    pub fn state(&self) -> &BotState {
        &self.store.state
    }

    pub fn step(
        &mut self,
        symbol: &SymbolId,
        candles: &[Candle],
        ticker_price: f64,
        today_utc: NaiveDate,
        kill_active: bool,
        pause_active: bool,
    ) -> Result<StepResult, SimError> {
        if !ticker_price.is_finite() || ticker_price <= 0.0 {
            return Err(SimError::InvalidTicker(ticker_price));
        }
        if candles.is_empty() {
            return Ok(StepResult::Warmup);
        }
        if candles
            .windows(2)
            .any(|w| w[0].open_time() >= w[1].open_time())
        {
            return Err(SimError::InvalidCandleOrder);
        }
        let last = candles.last().expect("nonempty checked");
        let candle_id = last.open_time().as_millis().to_string();
        if self.store.state.last_candle.get(&symbol.to_string()) == Some(&candle_id) {
            return Ok(StepResult::DuplicateCandle);
        }
        let strategy = self
            .strategies
            .get(symbol)
            .ok_or_else(|| SimError::MissingStrategy(symbol.clone()))?;
        let bars = strategy.signals(candles)?;
        let Some(bar) = bars.last() else {
            return Ok(StepResult::Warmup);
        };
        let Some(atr) = bar.atr else {
            return Ok(StepResult::Warmup);
        };
        let key = symbol.to_string();
        let marked_equity =
            self.equity_with_marks(&HashMap::from([(key.clone(), ticker_price)]))?;
        let outcome = evaluate_risk_gate(
            &mut self.store.state.risk,
            today_utc,
            marked_equity,
            self.config.daily_loss_cap,
            kill_active,
            pause_active,
        )?;
        let existing = self.store.state.positions.get(&key).cloned();
        let result = if let Some(position) = existing {
            let stop_hit = ticker_price <= position.stop;
            if stop_hit || bar.exit_long {
                let reason = if stop_hit { "stop" } else { "signal" };
                self.close_position(symbol, &position, ticker_price, last.open_time(), reason)?;
                let equity =
                    self.equity_with_marks(&HashMap::from([(key.clone(), ticker_price)]))?;
                StepResult::Exited {
                    quantity: position.qty,
                    price: ticker_price,
                    reason: reason.to_owned(),
                    equity,
                }
            } else {
                StepResult::NoAction {
                    equity: self
                        .equity_with_marks(&HashMap::from([(key.clone(), ticker_price)]))?,
                }
            }
        } else if bar.enter_long {
            if let Some(block) = outcome.blocked {
                StepResult::EntryBlocked {
                    reason: block.as_str().to_owned(),
                    equity: self
                        .equity_with_marks(&HashMap::from([(key.clone(), ticker_price)]))?,
                }
            } else {
                self.open_position(
                    symbol,
                    ticker_price,
                    atr,
                    strategy.stop_mult(),
                    last.open_time(),
                )?
            }
        } else {
            StepResult::NoAction {
                equity: self.equity_with_marks(&HashMap::from([(key.clone(), ticker_price)]))?,
            }
        };
        self.store.state.last_candle.insert(key, candle_id);
        self.store.save()?;
        Ok(result)
    }

    fn equity_with_marks(&self, marks: &HashMap<String, f64>) -> Result<f64, SimError> {
        let sim = self.store.state.sim.as_ref().expect("sim initialized");
        let quote = *sim.balances.get(&self.config.quote).unwrap_or(&0.0);
        let positions: Vec<_> = self
            .store
            .state
            .positions
            .iter()
            .map(|(symbol, p)| OpenPositionMark {
                quantity: p.qty,
                entry_price: p.entry,
                mark_price: *marks.get(symbol).unwrap_or(&p.entry),
            })
            .collect();
        let realized: f64 = self.store.state.closed.iter().map(|t| t.pnl).sum();
        Ok(compute_equity(
            quote,
            &positions,
            realized,
            self.config.capital_cap,
        )?)
    }

    fn open_position(
        &mut self,
        symbol: &SymbolId,
        price: f64,
        atr: f64,
        stop_mult: f64,
        time: trading_engine_core::Timestamp,
    ) -> Result<StepResult, SimError> {
        let equity = self.equity_with_marks(&HashMap::new())?;
        let sim = self.store.state.sim.as_ref().expect("sim initialized");
        let free = *sim.balances.get(&self.config.quote).unwrap_or(&0.0);
        let qty = size_paper_position(
            equity,
            atr,
            stop_mult,
            price,
            free,
            self.config.risk_per_trade,
            self.config.sim_fee,
        )?;
        if qty <= 0.0 {
            return Ok(StepResult::NoAction { equity });
        }
        let fee = qty * price * self.config.sim_fee;
        let stop = price - stop_mult * atr;
        if stop <= 0.0 {
            return Ok(StepResult::NoAction { equity });
        }
        let client_id = format!("sim-{}-{}", symbol.binance_symbol(), time.as_millis());
        let _intent = OrderIntent::new(symbol.clone(), Side::Buy, qty)?
            .with_client_order_id(client_id.clone())
            .with_reason("entry");
        let fill = Fill::new(
            symbol.clone(),
            Side::Buy,
            qty,
            Money::new(price)?,
            Money::new(fee)?,
            time,
        )?
        .with_client_order_id(client_id.clone())
        .with_order_id(client_id.clone());
        let position = Position::from_entry_fill(&fill, Money::new(stop)?, Money::new(atr)?)?;
        let sim = self.store.state.sim.as_mut().expect("sim initialized");
        *sim.balances.entry(self.config.quote.clone()).or_default() -= qty * price + fee;
        *sim.balances.entry(symbol.base().to_owned()).or_default() += qty;
        self.store.state.positions.insert(
            symbol.to_string(),
            PersistedPosition {
                qty,
                entry: price,
                stop,
                atr,
                entry_time: time.as_millis().to_string(),
                candle: time.as_millis().to_string(),
                entry_fee: fee,
                client_id: client_id.clone(),
                order_id: client_id.clone(),
                risk_quote: position.risk_quote().as_f64(),
            },
        );
        self.store.state.pending.insert(
            symbol.to_string(),
            PendingOrder {
                kind: PendingOrderKind::Entry,
                client_id: client_id.clone(),
                atr: Some(atr),
                candle: Some(time.as_millis().to_string()),
                reason: Some("entry".to_owned()),
            },
        );
        self.journal.write(
            &journal_at(time, "fill")
                .with_symbol(symbol.to_string())
                .with_side("buy")
                .with_qty(qty)
                .with_price(price)
                .with_stop(stop)
                .with_client_id(client_id)
                .with_details(format!("simulated entry fee={fee}")),
        )?;
        self.store.state.pending.remove(&symbol.to_string());
        Ok(StepResult::Entered {
            quantity: qty,
            price,
            equity: self.equity_with_marks(&HashMap::from([(symbol.to_string(), price)]))?,
        })
    }

    fn close_position(
        &mut self,
        symbol: &SymbolId,
        p: &PersistedPosition,
        price: f64,
        time: trading_engine_core::Timestamp,
        reason: &str,
    ) -> Result<(), SimError> {
        let fee = p.qty * price * self.config.sim_fee;
        let client_id = format!("sim-exit-{}-{}", symbol.binance_symbol(), time.as_millis());
        let _intent = OrderIntent::new(symbol.clone(), Side::Sell, p.qty)?
            .with_client_order_id(client_id.clone())
            .with_reason(reason);
        let _fill = Fill::new(
            symbol.clone(),
            Side::Sell,
            p.qty,
            Money::new(price)?,
            Money::new(fee)?,
            time,
        )?
        .with_client_order_id(client_id.clone())
        .with_order_id(client_id.clone());
        let sim = self.store.state.sim.as_mut().expect("sim initialized");
        *sim.balances.entry(self.config.quote.clone()).or_default() += p.qty * price - fee;
        *sim.balances.entry(symbol.base().to_owned()).or_default() -= p.qty;
        let pnl = p.qty * (price - p.entry) - p.entry_fee - fee;
        self.store.state.closed.push(ClosedTradeRecord {
            symbol: symbol.to_string(),
            entry_time: p.entry_time.clone(),
            exit_time: time.as_millis().to_string(),
            entry: p.entry,
            exit: price,
            qty: p.qty,
            pnl,
            r: (p.risk_quote > 0.0).then_some(pnl / p.risk_quote),
            reason: reason.to_owned(),
        });
        self.store.state.positions.remove(&symbol.to_string());
        self.store.state.pending.remove(&symbol.to_string());
        self.journal.write(
            &journal_at(time, "fill")
                .with_symbol(symbol.to_string())
                .with_side("sell")
                .with_qty(p.qty)
                .with_price(price)
                .with_pnl(pnl)
                .with_client_id(client_id)
                .with_details(format!("{reason} fee={fee}")),
        )?;
        Ok(())
    }
}

fn journal_at(time: trading_engine_core::Timestamp, event: &str) -> JournalEntry {
    let ts = chrono::DateTime::from_timestamp_millis(time.as_millis())
        .expect("domain timestamp is range-checked")
        .format("%Y-%m-%dT%H:%M:%SZ")
        .to_string();
    JournalEntry {
        ts_utc: ts,
        event: event.to_owned(),
        ..JournalEntry::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{env, fs};
    use trading_engine_core::Timestamp;
    use trading_engine_strategies::{StrategyBar, StrategyError};

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

    fn fixture_engine(
        enter: bool,
        exit: bool,
        suffix: &str,
    ) -> (SimEngine, SymbolId, std::path::PathBuf) {
        let dir = env::temp_dir().join(format!("sim-engine-{}-{suffix}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let symbol = SymbolId::parse("ETH/USDT").unwrap();
        let config = BotConfig {
            symbols: vec![trading_engine_config::SymbolConfig::new(
                symbol.clone(),
                2,
                1,
                2.0,
            )],
            history: 14,
            sim_start_usdt: 1_000.0,
            capital_cap: Some(1_000.0),
            ..BotConfig::default()
        };
        let store = StateStore::open(dir.join("state.json")).unwrap();
        let journal = Journal::open(dir.join("journal.csv")).unwrap();
        let strategies = HashMap::from([(
            symbol.clone(),
            Box::new(FixedStrategy { enter, exit }) as Box<dyn Strategy>,
        )]);
        (
            SimEngine::open(config, strategies, store, journal).unwrap(),
            symbol,
            dir,
        )
    }

    fn candles(indices: &[i64]) -> Vec<Candle> {
        indices
            .iter()
            .map(|&i| {
                let price = 100.0;
                Candle::new(
                    Timestamp::from_millis(1_700_000_000_000 + i * 14_400_000).unwrap(),
                    Money::new(price).unwrap(),
                    Money::new(price + 1.0).unwrap(),
                    Money::new(price - 1.0).unwrap(),
                    Money::new(price).unwrap(),
                    1.0,
                )
                .unwrap()
            })
            .collect()
    }

    fn today() -> NaiveDate {
        NaiveDate::from_ymd_opt(2024, 1, 1).unwrap()
    }

    #[test]
    fn entry_pause_kill_dedup_and_protective_exit_policy() {
        for (kill, pause) in [(false, true), (true, false)] {
            let (mut engine, symbol, dir) =
                fixture_engine(true, false, &format!("blocked-{kill}-{pause}"));
            let series = candles(&[1]);
            assert!(matches!(
                engine
                    .step(&symbol, &series, 100.0, today(), kill, pause)
                    .unwrap(),
                StepResult::EntryBlocked { .. }
            ));
            assert!(engine.state().positions.is_empty());
            fs::remove_dir_all(dir).unwrap();
        }
        let (mut engine, symbol, dir) = fixture_engine(true, false, "entry-exit");
        let series = candles(&[1]);
        let entered = engine
            .step(&symbol, &series, 100.0, today(), false, false)
            .unwrap();
        assert!(matches!(entered, StepResult::Entered { .. }));
        assert!(matches!(
            engine
                .step(&symbol, &series, 100.0, today(), false, false)
                .unwrap(),
            StepResult::DuplicateCandle
        ));
        assert_eq!(engine.state().positions.len(), 1);
        let (mut exit_engine, exit_symbol, exit_dir) = fixture_engine(false, true, "exit-gated");
        exit_engine.store.state = engine.store.state.clone();
        exit_engine.store.save().unwrap();
        let exit_series = candles(&[2]);
        assert!(matches!(
            exit_engine
                .step(&exit_symbol, &exit_series, 105.0, today(), true, true)
                .unwrap(),
            StepResult::Exited { .. }
        ));
        assert!(exit_engine.state().positions.is_empty());
        assert_eq!(exit_engine.state().closed.len(), 1);
        assert_eq!(
            Journal::open(exit_dir.join("journal.csv"))
                .unwrap()
                .read_all()
                .unwrap()
                .len(),
            1
        );
        let restored = StateStore::open(dir.join("state.json")).unwrap();
        assert_eq!(restored.state.positions.len(), 1);
        let strategies = HashMap::from([(
            symbol.clone(),
            Box::new(FixedStrategy {
                enter: true,
                exit: false,
            }) as Box<dyn Strategy>,
        )]);
        let mut restarted = SimEngine::open(
            engine.config.clone(),
            strategies,
            restored,
            Journal::open(dir.join("journal.csv")).unwrap(),
        )
        .unwrap();
        assert!(matches!(
            restarted
                .step(&symbol, &series, 100.0, today(), false, false)
                .unwrap(),
            StepResult::DuplicateCandle
        ));
        let rows = Journal::open(dir.join("journal.csv"))
            .unwrap()
            .read_all()
            .unwrap();
        assert_eq!(rows[0].ts_utc, "2023-11-15T02:13:20Z");
        assert_eq!(rows.len(), 1);
        fs::remove_dir_all(dir).unwrap();
        fs::remove_dir_all(exit_dir).unwrap();
    }

    #[test]
    fn stop_loss_closes_and_records_deterministically() {
        let (mut engine, symbol, dir) = fixture_engine(true, false, "stop");
        engine
            .step(&symbol, &candles(&[1]), 100.0, today(), false, false)
            .unwrap();
        let res = engine
            .step(&symbol, &candles(&[2]), 79.0, today(), true, true)
            .unwrap();
        assert!(matches!(res, StepResult::Exited { ref reason, .. } if reason == "stop"));
        assert!(engine.state().closed[0].pnl < 0.0);
        fs::remove_dir_all(dir).unwrap();
    }
}
