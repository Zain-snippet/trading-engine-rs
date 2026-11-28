//! Testnet-only orchestration. Market data and account snapshots are supplied
//! by the caller; strategy, risk, filter validation, execution and persistence
//! are coordinated here. This does not alter the historical backtester.

use std::collections::HashMap;

use chrono::NaiveDate;
use trading_engine_binance::{
    deterministic_client_order_id, BinanceTestnetClient, BinanceTrade, Decimal, ExecutionError,
    OrderLifecycle, OrderStatus, OrderSubmitError, OrderValidationError, OrderValidator,
};
use trading_engine_config::BotConfig;
use trading_engine_core::{Candle, Side, SymbolId};
use trading_engine_persistence::{
    BotState, ClosedTradeRecord, Journal, JournalEntry, PersistedExchangeFill,
    PersistedExchangeOrder, PersistedOrderStatus, PersistedPosition, ReconciliationDiscrepancy,
    StateStore,
};
use trading_engine_risk::{evaluate_risk_gate, size_paper_position, RiskError};
use trading_engine_strategies::{Strategy, StrategyError};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeEnvironment {
    SpotTestnet,
}

#[derive(Debug, Clone)]
pub struct RiskContext {
    pub reference_price: Decimal,
    /// Marked account equity computed from account balances and ticker marks.
    pub equity: f64,
    pub free_balances: HashMap<String, Decimal>,
    pub today_utc: NaiveDate,
    pub pause_active: bool,
    pub kill_active: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum LiveStepResult {
    Warmup,
    DuplicateCandle,
    NoAction,
    EntryBlocked(String),
    OrderSubmitted {
        client_order_id: String,
        status: OrderStatus,
        executed_qty: String,
    },
    AmbiguousSubmission {
        client_order_id: String,
    },
    NeedsReconciliation {
        client_order_id: String,
        reason: String,
    },
    PendingOrder {
        client_order_id: String,
    },
}

#[derive(Debug)]
pub enum LiveError {
    Config(String),
    Strategy(StrategyError),
    Risk(RiskError),
    Validation(OrderValidationError),
    Execution(ExecutionError),
    Persistence(trading_engine_persistence::PersistenceError),
    InvalidMarketData(&'static str),
    MissingStrategy(SymbolId),
    UnsupportedEnvironment,
}

impl std::fmt::Display for LiveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Config(message) => write!(f, "invalid configuration: {message}"),
            Self::Strategy(error) => write!(f, "strategy failure: {error}"),
            Self::Risk(error) => write!(f, "risk check failed: {error}"),
            Self::Validation(error) => write!(f, "order validation failed: {error}"),
            Self::Execution(error) => write!(f, "order lifecycle failure: {error}"),
            Self::Persistence(error) => write!(f, "persistence failure: {error}"),
            Self::InvalidMarketData(message) => f.write_str(message),
            Self::MissingStrategy(symbol) => write!(f, "missing strategy for {symbol}"),
            Self::UnsupportedEnvironment => {
                f.write_str("only explicitly selected Spot Testnet is supported")
            }
        }
    }
}

impl std::error::Error for LiveError {}

impl From<StrategyError> for LiveError {
    fn from(e: StrategyError) -> Self {
        Self::Strategy(e)
    }
}
impl From<RiskError> for LiveError {
    fn from(e: RiskError) -> Self {
        Self::Risk(e)
    }
}
impl From<OrderValidationError> for LiveError {
    fn from(e: OrderValidationError) -> Self {
        Self::Validation(e)
    }
}
impl From<ExecutionError> for LiveError {
    fn from(e: ExecutionError) -> Self {
        Self::Execution(e)
    }
}
impl From<trading_engine_persistence::PersistenceError> for LiveError {
    fn from(e: trading_engine_persistence::PersistenceError) -> Self {
        Self::Persistence(e)
    }
}

#[derive(Debug)]
pub struct SubmitFailure {
    pub ambiguous: bool,
    pub message: String,
}

pub trait SpotTestnetExecution {
    fn account(&mut self) -> Result<trading_engine_binance::AccountInfo, String>;
    #[allow(clippy::too_many_arguments)]
    fn submit_market(
        &mut self,
        validator: &OrderValidator,
        symbol: &SymbolId,
        side: Side,
        quantity: &Decimal,
        reference_price: &Decimal,
        free_balances: &HashMap<String, Decimal>,
        client_order_id: &str,
    ) -> Result<trading_engine_binance::OrderResponse, SubmitFailure>;
    fn query_order(
        &mut self,
        symbol: &SymbolId,
        client_order_id: &str,
    ) -> Result<trading_engine_binance::OrderResponse, String>;
    fn cancel_order(
        &mut self,
        symbol: &SymbolId,
        client_order_id: &str,
    ) -> Result<trading_engine_binance::OrderResponse, String>;
    fn open_orders(&mut self) -> Result<Vec<trading_engine_binance::OrderResponse>, String>;
    fn my_trades(&mut self, symbol: &SymbolId, order_id: u64) -> Result<Vec<BinanceTrade>, String>;
}

impl SpotTestnetExecution for BinanceTestnetClient {
    fn account(&mut self) -> Result<trading_engine_binance::AccountInfo, String> {
        BinanceTestnetClient::account(self).map_err(|error| error.to_string())
    }

    fn submit_market(
        &mut self,
        validator: &OrderValidator,
        symbol: &SymbolId,
        side: Side,
        quantity: &Decimal,
        reference_price: &Decimal,
        free_balances: &HashMap<String, Decimal>,
        client_order_id: &str,
    ) -> Result<trading_engine_binance::OrderResponse, SubmitFailure> {
        self.market_order_validated(
            validator,
            symbol,
            side,
            quantity,
            reference_price,
            free_balances,
            client_order_id,
        )
        .map_err(|error| {
            let (ambiguous, message) = match &error {
                OrderSubmitError::Validation(_) => (false, error.to_string()),
                OrderSubmitError::Exchange(exchange) => {
                    let ambiguous = match exchange {
                        trading_engine_binance::TestnetError::Transport(_) => true,
                        trading_engine_binance::TestnetError::Http { status, code, .. } => {
                            *status >= 500 || *code == Some(-1007)
                        }
                        _ => false,
                    };
                    (ambiguous, error.to_string())
                }
            };
            SubmitFailure { ambiguous, message }
        })
    }

    fn query_order(
        &mut self,
        symbol: &SymbolId,
        client_order_id: &str,
    ) -> Result<trading_engine_binance::OrderResponse, String> {
        BinanceTestnetClient::query_order(self, symbol, None, Some(client_order_id))
            .map_err(|error| error.to_string())
    }

    fn cancel_order(
        &mut self,
        symbol: &SymbolId,
        client_order_id: &str,
    ) -> Result<trading_engine_binance::OrderResponse, String> {
        BinanceTestnetClient::cancel_order(self, symbol, client_order_id)
            .map_err(|error| error.to_string())
    }

    fn open_orders(&mut self) -> Result<Vec<trading_engine_binance::OrderResponse>, String> {
        BinanceTestnetClient::open_orders(self, None).map_err(|error| error.to_string())
    }

    fn my_trades(&mut self, symbol: &SymbolId, order_id: u64) -> Result<Vec<BinanceTrade>, String> {
        BinanceTestnetClient::my_trades(self, symbol, order_id).map_err(|error| error.to_string())
    }
}

/// Single-writer Testnet engine. `RuntimeEnvironment::SpotTestnet` is required
/// at construction; no production endpoint or execution implementation exists.
pub struct TestnetEngine<B: SpotTestnetExecution> {
    config: BotConfig,
    strategies: HashMap<SymbolId, Box<dyn Strategy>>,
    validator: OrderValidator,
    broker: B,
    store: StateStore,
    journal: Journal,
}

impl<B: SpotTestnetExecution> TestnetEngine<B> {
    #[allow(clippy::too_many_arguments)]
    pub fn open(
        environment: RuntimeEnvironment,
        config: BotConfig,
        strategies: HashMap<SymbolId, Box<dyn Strategy>>,
        validator: OrderValidator,
        broker: B,
        store: StateStore,
        journal: Journal,
    ) -> Result<Self, LiveError> {
        if environment != RuntimeEnvironment::SpotTestnet {
            return Err(LiveError::UnsupportedEnvironment);
        }
        config
            .validate()
            .map_err(|e| LiveError::Config(e.to_string()))?;
        Ok(Self {
            config,
            strategies,
            validator,
            broker,
            store,
            journal,
        })
    }

    pub fn state(&self) -> &BotState {
        &self.store.state
    }

    /// Fetches the authenticated Spot Testnet account snapshot through the
    /// engine's configured execution adapter.
    pub fn account(&mut self) -> Result<trading_engine_binance::AccountInfo, String> {
        self.broker.account()
    }

    /// Resolves persisted in-flight orders by client ID, then compares tracked
    /// long quantities with exchange balances. Discrepancies are recorded and
    /// never silently overwrite local positions.
    pub fn reconcile_startup(
        &mut self,
        total_balances: &HashMap<String, Decimal>,
    ) -> Result<Vec<String>, LiveError> {
        let in_flight: Vec<_> = self
            .store
            .state
            .exchange
            .as_ref()
            .map(|exchange| {
                exchange
                    .orders
                    .values()
                    .filter(|order| order_needs_reconciliation(order))
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();
        let mut findings = Vec::new();
        for order in in_flight {
            let symbol = SymbolId::parse(&order.symbol)
                .map_err(|_| LiveError::InvalidMarketData("invalid persisted exchange symbol"))?;
            match self.broker.query_order(&symbol, &order.client_order_id) {
                Ok(response) => {
                    let stored_stop = order
                        .stop_price
                        .as_deref()
                        .and_then(|value| value.parse().ok());
                    let result = self.reconcile_response(
                        &symbol,
                        &response,
                        order.atr,
                        stored_stop,
                        order.reason.as_deref(),
                    )?;
                    if let LiveStepResult::NeedsReconciliation { reason, .. } = result {
                        self.store.state.halted = true;
                        findings.push(format!("{}: {reason}", order.client_order_id));
                    }
                }
                Err(reason) => {
                    self.store.state.halted = true;
                    self.store
                        .update_exchange_order(&order.client_order_id, |record| {
                            record.discrepancy = Some(reason.clone());
                        })?;
                    findings.push(format!(
                        "{} remains unresolved: {reason}",
                        order.client_order_id
                    ));
                }
            }
        }

        match self.broker.open_orders() {
            Ok(open_orders) => {
                let mut untracked = Vec::new();
                for open_order in open_orders {
                    let locally_active = self
                        .store
                        .state
                        .exchange
                        .as_ref()
                        .and_then(|exchange| exchange.orders.get(&open_order.client_order_id))
                        .is_some_and(|record| {
                            matches!(
                                record.status,
                                PersistedOrderStatus::Prepared
                                    | PersistedOrderStatus::Submitting
                                    | PersistedOrderStatus::Unknown
                                    | PersistedOrderStatus::New
                                    | PersistedOrderStatus::PartiallyFilled
                            )
                        });
                    if !locally_active {
                        untracked.push(format!(
                            "{}:{}:{:?}",
                            open_order.symbol, open_order.client_order_id, open_order.status
                        ));
                    }
                }
                untracked.sort();
                untracked.dedup();
                if !untracked.is_empty() {
                    self.store.state.halted = true;
                    findings.push(format!(
                        "untracked exchange open orders found: {}",
                        untracked.join(", ")
                    ));
                }
                self.store
                    .state
                    .exchange
                    .get_or_insert_with(Default::default)
                    .untracked_open_orders = untracked;
            }
            Err(error) => {
                self.store.state.halted = true;
                findings.push(format!("could not read exchange open orders: {error}"));
            }
        }

        for (symbol, position) in self.store.state.positions.clone() {
            let base = symbol.split('/').next().unwrap_or_default();
            let exchange_qty = total_balances.get(base).copied().unwrap_or_default();
            let exchange_qty_f64 = exchange_qty.to_string().parse::<f64>().unwrap_or(f64::NAN);
            if !exchange_qty_f64.is_finite() || (exchange_qty_f64 - position.qty).abs() > 1e-8 {
                self.store.state.halted = true;
                let reason =
                    "exchange base balance differs from tracked position; local position retained";
                let already_recorded = self.store.state.exchange.as_ref().is_some_and(|exchange| {
                    exchange.discrepancies.iter().any(|item| {
                        item.symbol == symbol
                            && item.local_quantity == position.qty.to_string()
                            && item.exchange_quantity == exchange_qty.to_string()
                            && item.reason == reason
                    })
                });
                if !already_recorded {
                    self.store
                        .state
                        .exchange
                        .get_or_insert_with(Default::default)
                        .discrepancies
                        .push(ReconciliationDiscrepancy {
                            symbol: symbol.clone(),
                            local_quantity: position.qty.to_string(),
                            exchange_quantity: exchange_qty.to_string(),
                            reason: reason.to_owned(),
                            observed_at_ms: chrono::Utc::now().timestamp_millis(),
                        });
                    self.journal.write(&JournalEntry {
                        ts_utc: chrono::Utc::now().to_rfc3339(),
                        event: "reconcile".to_owned(),
                        symbol: symbol.clone(),
                        qty: Some(position.qty.to_string()),
                        details: Some(format!("{reason}; exchange={exchange_qty}")),
                        ..JournalEntry::default()
                    })?;
                }
                findings.push(format!("{symbol}: {reason}"));
            } else if let Some(exchange) = self.store.state.exchange.as_mut() {
                exchange.discrepancies.retain(|item| {
                    !(item.symbol == symbol
                        && item.reason
                            == "exchange base balance differs from tracked position; local position retained")
                });
            }
        }
        let tracked_symbols: std::collections::HashSet<_> =
            self.store.state.positions.keys().cloned().collect();
        if let Some(exchange) = self.store.state.exchange.as_mut() {
            exchange.discrepancies.retain(|item| {
                item.reason
                    != "exchange base balance differs from tracked position; local position retained"
                    || tracked_symbols.contains(&item.symbol)
            });
        }
        self.store.save()?;
        Ok(findings)
    }

    pub fn step(
        &mut self,
        symbol: &SymbolId,
        candles: &[Candle],
        context: &RiskContext,
    ) -> Result<LiveStepResult, LiveError> {
        validate_candles(candles)?;
        let Some(last) = candles.last() else {
            return Ok(LiveStepResult::Warmup);
        };
        let strategy = self
            .strategies
            .get(symbol)
            .ok_or_else(|| LiveError::MissingStrategy(symbol.clone()))?;
        let bars = strategy.signals(candles)?;
        let Some(bar) = bars.last() else {
            return Ok(LiveStepResult::Warmup);
        };
        let Some(atr) = bar.atr else {
            return Ok(LiveStepResult::Warmup);
        };
        let candle_id = last.open_time().as_millis().to_string();
        let key = symbol.to_string();
        let price = context
            .reference_price
            .to_string()
            .parse::<f64>()
            .map_err(|_| LiveError::InvalidMarketData("invalid reference price"))?;
        if !price.is_finite() || price <= 0.0 {
            return Err(LiveError::InvalidMarketData(
                "reference price must be positive and finite",
            ));
        }

        let risk = evaluate_risk_gate(
            &mut self.store.state.risk,
            context.today_utc,
            context.equity,
            self.config.daily_loss_cap,
            context.kill_active || self.store.state.halted,
            context.pause_active,
        )?;
        let position = self.store.state.positions.get(&key).cloned();
        let is_duplicate = self.store.state.last_candle.get(&key) == Some(&candle_id);

        let result = if let Some(position) = position {
            let stop_hit = price <= position.stop;
            if stop_hit || bar.exit_long {
                let reason = if stop_hit { "stop" } else { "signal" };
                if let Some(active) = self.active_order_record_for(&key) {
                    if active.side != "BUY" {
                        return Ok(LiveStepResult::PendingOrder {
                            client_order_id: active.client_order_id,
                        });
                    }
                    let cancel_response = match self
                        .broker
                        .cancel_order(symbol, &active.client_order_id)
                    {
                        Ok(response) => response,
                        Err(message) => {
                            self.store
                                .update_exchange_order(&active.client_order_id, |order| {
                                    order.discrepancy =
                                        Some(format!("entry cancellation unresolved: {message}"));
                                })?;
                            return Ok(LiveStepResult::PendingOrder {
                                client_order_id: active.client_order_id,
                            });
                        }
                    };
                    let reconciled = self.reconcile_response(
                        symbol,
                        &cancel_response,
                        active.atr,
                        active
                            .stop_price
                            .as_deref()
                            .and_then(|value| value.parse().ok()),
                        active.reason.as_deref(),
                    )?;
                    if matches!(reconciled, LiveStepResult::NeedsReconciliation { .. }) {
                        return Ok(LiveStepResult::PendingOrder {
                            client_order_id: active.client_order_id,
                        });
                    }
                    if !self.store.state.positions.contains_key(&key) {
                        return Ok(LiveStepResult::NoAction);
                    }
                } else if self.active_order_for(&key) {
                    return Ok(LiveStepResult::PendingOrder {
                        client_order_id: self.active_client_for(&key).unwrap_or_default(),
                    });
                }
                let position = self
                    .store
                    .state
                    .positions
                    .get(&key)
                    .cloned()
                    .unwrap_or(position);
                let quantity = Decimal::parse(&position.qty.to_string())?;
                let id = deterministic_client_order_id(symbol, Side::Sell, last.open_time());
                self.submit(
                    symbol,
                    Side::Sell,
                    quantity,
                    context,
                    &id,
                    &candle_id,
                    None,
                    Some(reason),
                    None,
                )?
            } else if is_duplicate {
                LiveStepResult::DuplicateCandle
            } else {
                LiveStepResult::NoAction
            }
        } else if is_duplicate {
            LiveStepResult::DuplicateCandle
        } else if !bar.enter_long {
            LiveStepResult::NoAction
        } else if let Some(blocked) = risk.blocked {
            LiveStepResult::EntryBlocked(blocked.as_str().to_owned())
        } else {
            if self.active_order_for(&key) {
                return Ok(LiveStepResult::PendingOrder {
                    client_order_id: self.active_client_for(&key).unwrap_or_default(),
                });
            }
            let free_quote = context
                .free_balances
                .get(&self.config.quote)
                .copied()
                .unwrap_or_default()
                .to_string()
                .parse::<f64>()
                .map_err(|_| LiveError::InvalidMarketData("invalid free quote balance"))?;
            let qty_f64 = size_paper_position(
                context.equity,
                atr,
                strategy.stop_mult(),
                price,
                free_quote,
                self.config.risk_per_trade,
                self.config.sim_fee,
            )?;
            if qty_f64 <= 0.0 {
                LiveStepResult::NoAction
            } else {
                let raw_qty = Decimal::parse(&qty_f64.to_string())?;
                let rules = self.validator.rules_for_symbol(symbol)?;
                let quantity = raw_qty.floor_to_step(rules.market_quantity_step())?;
                let stop = price - strategy.stop_mult() * atr;
                if !stop.is_finite() || stop <= 0.0 {
                    LiveStepResult::NoAction
                } else {
                    let id = deterministic_client_order_id(symbol, Side::Buy, last.open_time());
                    self.submit(
                        symbol,
                        Side::Buy,
                        quantity,
                        context,
                        &id,
                        &candle_id,
                        Some(atr),
                        Some("entry"),
                        Some(stop),
                    )?
                }
            }
        };

        self.store.state.last_candle.insert(key, candle_id);
        self.store.save()?;
        Ok(result)
    }

    fn active_order_for(&self, symbol: &str) -> bool {
        self.store.state.exchange.as_ref().is_some_and(|exchange| {
            exchange.orders.values().any(|order| {
                order.symbol == symbol
                    && matches!(
                        order.status,
                        PersistedOrderStatus::Prepared
                            | PersistedOrderStatus::Submitting
                            | PersistedOrderStatus::Unknown
                            | PersistedOrderStatus::New
                            | PersistedOrderStatus::PartiallyFilled
                    )
            })
        })
    }

    fn active_client_for(&self, symbol: &str) -> Option<String> {
        self.active_order_record_for(symbol)
            .map(|order| order.client_order_id)
    }

    fn active_order_record_for(&self, symbol: &str) -> Option<PersistedExchangeOrder> {
        self.store
            .state
            .exchange
            .as_ref()?
            .orders
            .values()
            .find(|order| {
                order.symbol == symbol
                    && matches!(
                        order.status,
                        PersistedOrderStatus::Prepared
                            | PersistedOrderStatus::Submitting
                            | PersistedOrderStatus::Unknown
                            | PersistedOrderStatus::New
                            | PersistedOrderStatus::PartiallyFilled
                    )
            })
            .cloned()
    }

    #[allow(clippy::too_many_arguments)]
    fn submit(
        &mut self,
        symbol: &SymbolId,
        side: Side,
        quantity: Decimal,
        context: &RiskContext,
        client_id: &str,
        candle_id: &str,
        atr: Option<f64>,
        reason: Option<&str>,
        stop: Option<f64>,
    ) -> Result<LiveStepResult, LiveError> {
        if !quantity.is_positive() {
            return Ok(LiveStepResult::NoAction);
        }
        self.validator.validate_market_order(
            symbol,
            side,
            quantity,
            context.reference_price,
            &context.free_balances,
        )?;
        let order = PersistedExchangeOrder {
            symbol: symbol.to_string(),
            side: side.as_str().to_ascii_uppercase(),
            client_order_id: client_id.to_owned(),
            requested_qty: quantity.to_string(),
            status: PersistedOrderStatus::Prepared,
            exchange_order_id: None,
            executed_qty: "0".to_owned(),
            cumulative_quote_qty: "0".to_owned(),
            fills: HashMap::new(),
            signal_candle: Some(candle_id.to_owned()),
            atr,
            stop_price: stop.map(|value| value.to_string()),
            reason: reason.map(str::to_owned),
            created_at_ms: chrono::Utc::now().timestamp_millis(),
            last_reconciled_at_ms: None,
            discrepancy: None,
        };
        if !self.store.record_exchange_intent(order)? {
            return Ok(LiveStepResult::PendingOrder {
                client_order_id: client_id.to_owned(),
            });
        }
        self.store.update_exchange_order(client_id, |record| {
            record.status = PersistedOrderStatus::Submitting
        })?;
        let response = match self.broker.submit_market(
            &self.validator,
            symbol,
            side,
            &quantity,
            &context.reference_price,
            &context.free_balances,
            client_id,
        ) {
            Ok(response) => response,
            Err(failure) => {
                self.store.update_exchange_order(client_id, |record| {
                    record.status = if failure.ambiguous {
                        PersistedOrderStatus::Unknown
                    } else {
                        PersistedOrderStatus::Rejected
                    };
                    record.discrepancy = Some(failure.message.clone());
                })?;
                return Ok(if failure.ambiguous {
                    LiveStepResult::AmbiguousSubmission {
                        client_order_id: client_id.to_owned(),
                    }
                } else {
                    LiveStepResult::NeedsReconciliation {
                        client_order_id: client_id.to_owned(),
                        reason: failure.message,
                    }
                });
            }
        };
        self.reconcile_response(symbol, &response, atr, stop, reason)
    }

    fn reconcile_response(
        &mut self,
        symbol: &SymbolId,
        response: &trading_engine_binance::OrderResponse,
        atr: Option<f64>,
        stop: Option<f64>,
        reason: Option<&str>,
    ) -> Result<LiveStepResult, LiveError> {
        let client_id = response.client_order_id.clone();
        let requested = self
            .store
            .state
            .exchange
            .as_ref()
            .and_then(|x| x.orders.get(&client_id))
            .map(|o| o.requested_qty.clone())
            .ok_or(LiveError::InvalidMarketData(
                "exchange order record disappeared",
            ))?;
        let is_buy = self
            .store
            .state
            .exchange
            .as_ref()
            .and_then(|x| x.orders.get(&client_id))
            .is_some_and(|order| order.side == "BUY");
        let mut lifecycle = OrderLifecycle::new(
            &client_id,
            symbol.binance_symbol(),
            if is_buy { Side::Buy } else { Side::Sell },
            Decimal::parse(&requested)?,
        );
        lifecycle.begin_submission()?;
        lifecycle.reconcile_order(response)?;
        self.store.update_exchange_order(&client_id, |record| {
            record.status = persist_status(response.status);
            record.exchange_order_id = Some(response.order_id.to_string());
            record.executed_qty = response.executed_qty.clone();
            record.cumulative_quote_qty = response.cumulative_quote_qty.clone();
            record.atr = atr.or(record.atr);
            record.stop_price = stop
                .map(|value| value.to_string())
                .or_else(|| record.stop_price.clone());
            record.reason = reason.map(str::to_owned).or_else(|| record.reason.clone());
        })?;
        let trades = match self.broker.my_trades(symbol, response.order_id) {
            Ok(trades) => trades,
            Err(reason) => {
                self.store.update_exchange_order(&client_id, |record| {
                    record.discrepancy = Some(reason.clone())
                })?;
                return Ok(LiveStepResult::NeedsReconciliation {
                    client_order_id: client_id,
                    reason,
                });
            }
        };
        let order_is_buy = self
            .store
            .state
            .exchange
            .as_ref()
            .and_then(|exchange| exchange.orders.get(&client_id))
            .is_some_and(|order| order.side == "BUY");
        if trades.iter().any(|trade| trade.is_buyer != order_is_buy) {
            let reason = "exchange trade side differs from persisted order side".to_owned();
            self.store.update_exchange_order(&client_id, |record| {
                record.discrepancy = Some(reason.clone())
            })?;
            return Ok(LiveStepResult::NeedsReconciliation {
                client_order_id: client_id,
                reason,
            });
        }
        let ledger =
            match trading_engine_binance::TradeLedger::from_trades(response.order_id, &trades) {
                Ok(ledger) => ledger,
                Err(error) => {
                    let reason = error.to_string();
                    self.store.update_exchange_order(&client_id, |record| {
                        record.discrepancy = Some(reason.clone())
                    })?;
                    return Ok(LiveStepResult::NeedsReconciliation {
                        client_order_id: client_id,
                        reason,
                    });
                }
            };
        if let Err(error) = ledger.verify_executed_qty(Decimal::parse(&response.executed_qty)?) {
            let reason = error.to_string();
            self.store.update_exchange_order(&client_id, |record| {
                record.discrepancy = Some(reason.clone())
            })?;
            return Ok(LiveStepResult::NeedsReconciliation {
                client_order_id: client_id,
                reason,
            });
        }
        let old_fills = self
            .store
            .state
            .exchange
            .as_ref()
            .and_then(|x| x.orders.get(&client_id))
            .map(|o| o.fills.clone())
            .unwrap_or_default();
        let new_trades: Vec<_> = trades
            .iter()
            .filter(|trade| !old_fills.contains_key(&trade.id.to_string()))
            .cloned()
            .collect();
        // Persist position mutations and the deduplication ledger together.
        // A crash can therefore neither lose applied fills nor apply them twice.
        let mut journal_entries = Vec::with_capacity(new_trades.len());
        for trade in new_trades {
            journal_entries.push(self.apply_trade(symbol, response, &trade, atr, reason)?);
        }
        if let Some(record) = self
            .store
            .state
            .exchange
            .as_mut()
            .and_then(|exchange| exchange.orders.get_mut(&client_id))
        {
            record.discrepancy = None;
            record.last_reconciled_at_ms = Some(chrono::Utc::now().timestamp_millis());
            for trade in &trades {
                record.fills.insert(
                    trade.id.to_string(),
                    PersistedExchangeFill {
                        trade_id: trade.id.to_string(),
                        qty: trade.qty.clone(),
                        price: trade.price.clone(),
                        quote_qty: trade.quote_qty.clone(),
                        commission: trade.commission.clone(),
                        commission_asset: trade.commission_asset.clone(),
                        time_ms: trade.time,
                    },
                );
            }
        }
        self.store.save()?;
        for entry in journal_entries {
            self.journal.write(&entry)?;
        }
        Ok(LiveStepResult::OrderSubmitted {
            client_order_id: client_id,
            status: response.status,
            executed_qty: response.executed_qty.clone(),
        })
    }

    fn apply_trade(
        &mut self,
        symbol: &SymbolId,
        response: &trading_engine_binance::OrderResponse,
        trade: &BinanceTrade,
        atr: Option<f64>,
        reason: Option<&str>,
    ) -> Result<JournalEntry, LiveError> {
        let qty = Decimal::parse(&trade.qty)?
            .to_string()
            .parse::<f64>()
            .map_err(|_| LiveError::InvalidMarketData("invalid trade quantity"))?;
        let price = Decimal::parse(&trade.price)?
            .to_string()
            .parse::<f64>()
            .map_err(|_| LiveError::InvalidMarketData("invalid trade price"))?;
        let commission = Decimal::parse(&trade.commission)?
            .to_string()
            .parse::<f64>()
            .map_err(|_| LiveError::InvalidMarketData("invalid commission"))?;
        let quote_qty = Decimal::parse(&trade.quote_qty)?
            .to_string()
            .parse::<f64>()
            .map_err(|_| LiveError::InvalidMarketData("invalid quote quantity"))?;
        let time_ms = i64::try_from(trade.time)
            .map_err(|_| LiveError::InvalidMarketData("trade timestamp out of range"))?;
        let fill_time = chrono::DateTime::from_timestamp_millis(time_ms)
            .ok_or(LiveError::InvalidMarketData("trade timestamp out of range"))?;
        let key = symbol.to_string();
        let base_fee = if trade.commission_asset == symbol.base() {
            commission
        } else {
            0.0
        };
        let quote_fee = if trade.commission_asset == symbol.quote() {
            commission
        } else {
            0.0
        };
        let order_side = if self
            .store
            .state
            .exchange
            .as_ref()
            .and_then(|x| x.orders.get(&response.client_order_id))
            .is_some_and(|order| order.side == "BUY")
        {
            Side::Buy
        } else {
            Side::Sell
        };
        if order_side == Side::Buy {
            let net_qty = qty - base_fee;
            // `quote_qty` already includes the gross executed base quantity;
            // a base-asset commission reduces received quantity but does not
            // add a second quote-denominated purchase cost.
            let actual_cost = quote_qty + quote_fee;
            if net_qty <= 0.0 || !actual_cost.is_finite() {
                return Err(LiveError::InvalidMarketData(
                    "entry fill commissions exceed fill quantity",
                ));
            }
            let previous = self.store.state.positions.get(&key).cloned();
            let (total_qty, average_entry, stop_price, entry_time, candle, risk_quote) =
                if let Some(position) = previous {
                    let total = position.qty + net_qty;
                    let cost = position.qty * position.entry + actual_cost;
                    let average = cost / total;
                    let stop = average
                        - self.strategy_for(symbol)?.stop_mult()
                            * atr.or(Some(position.atr)).unwrap_or(position.atr);
                    (
                        total,
                        average,
                        stop,
                        position.entry_time,
                        position.candle,
                        total * (average - stop),
                    )
                } else {
                    let average = actual_cost / net_qty;
                    let atr = atr.ok_or(LiveError::InvalidMarketData(
                        "entry fill has no recorded ATR",
                    ))?;
                    let stop = average - self.strategy_for(symbol)?.stop_mult() * atr;
                    (
                        net_qty,
                        average,
                        stop,
                        fill_time.to_rfc3339(),
                        trade.time.to_string(),
                        net_qty * (average - stop),
                    )
                };
            if stop_price <= 0.0 {
                return Err(LiveError::InvalidMarketData(
                    "computed protective stop is non-positive",
                ));
            }
            self.store.state.positions.insert(
                key.clone(),
                PersistedPosition {
                    qty: total_qty,
                    entry: average_entry,
                    stop: stop_price,
                    atr: atr.unwrap_or_else(|| {
                        self.store
                            .state
                            .positions
                            .get(&key)
                            .map_or(0.0, |position| position.atr)
                    }),
                    entry_time,
                    candle,
                    entry_fee: 0.0,
                    client_id: response.client_order_id.clone(),
                    order_id: response.order_id.to_string(),
                    risk_quote,
                },
            );
        } else {
            let mut position = self.store.state.positions.get(&key).cloned().ok_or(
                LiveError::InvalidMarketData("sell fill has no tracked long position"),
            )?;
            let consumed_qty = qty + base_fee;
            if consumed_qty > position.qty + f64::EPSILON {
                return Err(LiveError::InvalidMarketData(
                    "sell fill exceeds tracked position",
                ));
            }
            let proceeds = quote_qty - quote_fee;
            let pnl = proceeds - consumed_qty * position.entry;
            let average_exit = proceeds / qty;
            let remaining = (position.qty - consumed_qty).max(0.0);
            self.store.state.closed.push(ClosedTradeRecord {
                symbol: key.clone(),
                entry_time: position.entry_time.clone(),
                exit_time: fill_time.to_rfc3339(),
                entry: position.entry,
                exit: average_exit,
                qty,
                pnl,
                r: (position.risk_quote > 0.0).then_some(pnl / position.risk_quote),
                reason: reason.unwrap_or("signal").to_owned(),
            });
            if remaining <= f64::EPSILON {
                self.store.state.positions.remove(&key);
            } else {
                position.qty = remaining;
                position.risk_quote = remaining * (position.entry - position.stop);
                self.store.state.positions.insert(key.clone(), position);
            }
        }
        let journal = JournalEntry {
            ts_utc: fill_time.to_rfc3339(),
            event: "fill".to_owned(),
            symbol: key,
            side: Some(
                if order_side == Side::Buy {
                    "buy"
                } else {
                    "sell"
                }
                .to_owned(),
            ),
            qty: Some(qty.to_string()),
            price: Some(price.to_string()),
            pnl: None,
            order_id: Some(response.order_id.to_string()),
            client_id: Some(response.client_order_id.clone()),
            details: Some(format!(
                "trade_id={} commission={} {}",
                trade.id, commission, trade.commission_asset
            )),
            ..JournalEntry::default()
        };
        Ok(journal)
    }

    fn strategy_for(&self, symbol: &SymbolId) -> Result<&dyn Strategy, LiveError> {
        self.strategies
            .get(symbol)
            .map(Box::as_ref)
            .ok_or_else(|| LiveError::MissingStrategy(symbol.clone()))
    }
}

fn validate_candles(candles: &[Candle]) -> Result<(), LiveError> {
    if candles
        .windows(2)
        .any(|window| window[0].open_time() >= window[1].open_time())
    {
        return Err(LiveError::InvalidMarketData(
            "candles must be strictly chronological",
        ));
    }
    Ok(())
}

fn persist_status(status: OrderStatus) -> PersistedOrderStatus {
    match status {
        OrderStatus::New => PersistedOrderStatus::New,
        OrderStatus::PartiallyFilled => PersistedOrderStatus::PartiallyFilled,
        OrderStatus::Filled => PersistedOrderStatus::Filled,
        OrderStatus::Canceled => PersistedOrderStatus::Canceled,
        OrderStatus::Rejected => PersistedOrderStatus::Rejected,
        OrderStatus::Expired => PersistedOrderStatus::Expired,
        OrderStatus::ExpiredInMatch => PersistedOrderStatus::ExpiredInMatch,
    }
}

fn order_needs_reconciliation(order: &PersistedExchangeOrder) -> bool {
    let is_active = matches!(
        order.status,
        PersistedOrderStatus::Prepared
            | PersistedOrderStatus::Submitting
            | PersistedOrderStatus::Unknown
            | PersistedOrderStatus::New
            | PersistedOrderStatus::PartiallyFilled
    );
    let executed = match Decimal::parse(&order.executed_qty) {
        Ok(value) => value,
        Err(_) => return true,
    };
    let mut applied = Decimal::zero();
    for fill in order.fills.values() {
        let Ok(quantity) = Decimal::parse(&fill.qty) else {
            return true;
        };
        let Ok(total) = applied.checked_add(quantity) else {
            return true;
        };
        applied = total;
    }
    is_active || applied != executed
}

#[cfg(test)]
mod tests;
