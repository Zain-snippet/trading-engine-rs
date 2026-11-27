use std::{cmp::Ordering, collections::BTreeMap};

use trading_engine_core::{Side, SymbolId, Timestamp};

use crate::{BinanceTrade, Decimal, OrderResponse, OrderStatus, OrderValidationError};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrderLifecycleState {
    Prepared,
    Submitting,
    Ambiguous,
    Acknowledged,
    Terminal,
}

/// Stable client order ID for one symbol, side and completed signal candle.
#[must_use]
pub fn deterministic_client_order_id(symbol: &SymbolId, side: Side, candle: Timestamp) -> String {
    let side_code = match side {
        Side::Buy => 'B',
        Side::Sell => 'S',
    };
    format!(
        "te{}{}{}",
        symbol.binance_symbol(),
        side_code,
        candle.as_millis()
    )
}

/// In-memory order lifecycle guard. An ambiguous submission may be reconciled,
/// but this type will never permit it to be submitted a second time.
#[derive(Debug, Clone, PartialEq)]
pub struct OrderLifecycle {
    pub client_order_id: String,
    pub symbol: String,
    pub side: Side,
    pub requested_qty: Decimal,
    pub state: OrderLifecycleState,
    pub status: Option<OrderStatus>,
    pub order_id: Option<u64>,
    pub executed_qty: Decimal,
}

impl OrderLifecycle {
    pub fn new(
        client_order_id: impl Into<String>,
        symbol: impl Into<String>,
        side: Side,
        requested_qty: Decimal,
    ) -> Self {
        Self {
            client_order_id: client_order_id.into(),
            symbol: symbol.into(),
            side,
            requested_qty,
            state: OrderLifecycleState::Prepared,
            status: None,
            order_id: None,
            executed_qty: Decimal::zero(),
        }
    }

    pub fn begin_submission(&mut self) -> Result<(), ExecutionError> {
        if self.state != OrderLifecycleState::Prepared {
            return Err(ExecutionError::DuplicateSubmission);
        }
        self.state = OrderLifecycleState::Submitting;
        Ok(())
    }

    pub fn mark_ambiguous(&mut self) -> Result<(), ExecutionError> {
        if self.state != OrderLifecycleState::Submitting {
            return Err(ExecutionError::InvalidTransition);
        }
        self.state = OrderLifecycleState::Ambiguous;
        Ok(())
    }

    /// Accepts either a POST acknowledgement or an exchange reconciliation
    /// response. Callers fetch actual trades separately before booking fills.
    pub fn reconcile_order(&mut self, response: &OrderResponse) -> Result<(), ExecutionError> {
        if !matches!(
            self.state,
            OrderLifecycleState::Submitting
                | OrderLifecycleState::Ambiguous
                | OrderLifecycleState::Acknowledged
        ) {
            return Err(ExecutionError::InvalidTransition);
        }
        if response.client_order_id != self.client_order_id || response.symbol != self.symbol {
            return Err(ExecutionError::IdentityMismatch);
        }
        let executed = Decimal::parse(&response.executed_qty)?;
        if executed.checked_cmp(self.requested_qty)? == Ordering::Greater {
            return Err(ExecutionError::Overfill);
        }
        self.executed_qty = executed;
        self.order_id = Some(response.order_id);
        self.status = Some(response.status);
        self.state = if matches!(
            response.status,
            OrderStatus::New | OrderStatus::PartiallyFilled
        ) {
            OrderLifecycleState::Acknowledged
        } else {
            OrderLifecycleState::Terminal
        };
        Ok(())
    }

    /// True only before the first network submission. A timeout does not make
    /// this true again; looking up the deterministic client ID is next.
    #[must_use]
    pub const fn may_submit(&self) -> bool {
        matches!(self.state, OrderLifecycleState::Prepared)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct TradeLedger {
    pub order_id: u64,
    pub fills: BTreeMap<u64, TrackedTrade>,
    pub executed_qty: Decimal,
    pub quote_qty: Decimal,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TrackedTrade {
    pub qty: Decimal,
    pub price: Decimal,
    pub quote_qty: Decimal,
    pub commission: Decimal,
    pub commission_asset: String,
    pub time_ms: u64,
    pub is_buyer: bool,
}

impl TradeLedger {
    pub fn from_trades(order_id: u64, trades: &[BinanceTrade]) -> Result<Self, ExecutionError> {
        let mut ledger = Self {
            order_id,
            fills: BTreeMap::new(),
            executed_qty: Decimal::zero(),
            quote_qty: Decimal::zero(),
        };
        for trade in trades {
            if trade.order_id != order_id {
                return Err(ExecutionError::IdentityMismatch);
            }
            let fill = TrackedTrade {
                qty: Decimal::parse(&trade.qty)?,
                price: Decimal::parse(&trade.price)?,
                quote_qty: Decimal::parse(&trade.quote_qty)?,
                commission: Decimal::parse(&trade.commission)?,
                commission_asset: trade.commission_asset.clone(),
                time_ms: trade.time,
                is_buyer: trade.is_buyer,
            };
            if !fill.qty.is_positive() || !fill.price.is_positive() {
                return Err(ExecutionError::InvalidTrade);
            }
            if let Some(previous) = ledger.fills.get(&trade.id) {
                if previous != &fill {
                    return Err(ExecutionError::ConflictingDuplicateTrade(trade.id));
                }
                continue;
            }
            ledger.executed_qty = ledger.executed_qty.checked_add(fill.qty)?;
            ledger.quote_qty = ledger.quote_qty.checked_add(fill.quote_qty)?;
            ledger.fills.insert(trade.id, fill);
        }
        Ok(ledger)
    }

    /// Rebuilds from an exchange snapshot so repeat reads cannot double-count
    /// quantities or commissions.
    pub fn reconcile(
        &mut self,
        order_id: u64,
        trades: &[BinanceTrade],
    ) -> Result<(), ExecutionError> {
        *self = Self::from_trades(order_id, trades)?;
        Ok(())
    }

    pub fn verify_executed_qty(&self, expected: Decimal) -> Result<(), ExecutionError> {
        if self.executed_qty == expected {
            Ok(())
        } else {
            Err(ExecutionError::FillQuantityMismatch {
                expected: expected.to_string(),
                actual: self.executed_qty.to_string(),
            })
        }
    }
}

#[derive(Debug)]
pub enum ExecutionError {
    DuplicateSubmission,
    InvalidTransition,
    IdentityMismatch,
    Overfill,
    InvalidTrade,
    ConflictingDuplicateTrade(u64),
    FillQuantityMismatch { expected: String, actual: String },
    Validation(OrderValidationError),
}

impl std::fmt::Display for ExecutionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DuplicateSubmission => {
                f.write_str("order has already been submitted or is ambiguous")
            }
            Self::InvalidTransition => f.write_str("invalid order lifecycle transition"),
            Self::IdentityMismatch => {
                f.write_str("exchange response does not match local order identity")
            }
            Self::Overfill => {
                f.write_str("exchange executed quantity exceeds the requested quantity")
            }
            Self::InvalidTrade => {
                f.write_str("exchange trade has a non-positive quantity or price")
            }
            Self::ConflictingDuplicateTrade(id) => {
                write!(f, "trade ID {id} was returned with conflicting fill data")
            }
            Self::FillQuantityMismatch { expected, actual } => write!(
                f,
                "exchange order quantity is {expected}, while retrieved fills total {actual}"
            ),
            Self::Validation(error) => write!(f, "invalid exchange decimal: {error}"),
        }
    }
}

impl std::error::Error for ExecutionError {}

impl From<OrderValidationError> for ExecutionError {
    fn from(value: OrderValidationError) -> Self {
        Self::Validation(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use trading_engine_core::Timestamp;

    fn decimal(value: &str) -> Decimal {
        Decimal::parse(value).unwrap()
    }

    fn order_response(status: OrderStatus, executed: &str) -> OrderResponse {
        serde_json::from_value(serde_json::json!({
            "symbol":"ETHUSDT", "orderId":44, "clientOrderId":"entry-1", "status":status_name(status),
            "executedQty":executed, "cummulativeQuoteQty":"400", "fills":[]
        })).unwrap()
    }

    fn status_name(status: OrderStatus) -> &'static str {
        match status {
            OrderStatus::New => "NEW",
            OrderStatus::PartiallyFilled => "PARTIALLY_FILLED",
            OrderStatus::Filled => "FILLED",
            OrderStatus::Canceled => "CANCELED",
            OrderStatus::Rejected => "REJECTED",
            OrderStatus::Expired => "EXPIRED",
            OrderStatus::ExpiredInMatch => "EXPIRED_IN_MATCH",
        }
    }

    fn trade(id: u64, qty: &str) -> BinanceTrade {
        BinanceTrade {
            id,
            order_id: 44,
            price: "2000".to_owned(),
            qty: qty.to_owned(),
            quote_qty: (Decimal::parse(qty)
                .unwrap()
                .checked_mul(decimal("2000"))
                .unwrap())
            .to_string(),
            commission: "0.0002".to_owned(),
            commission_asset: "ETH".to_owned(),
            time: 123,
            is_buyer: true,
        }
    }

    #[test]
    fn ambiguous_submission_cannot_be_repeated_and_can_reconcile() {
        let mut lifecycle = OrderLifecycle::new("entry-1", "ETHUSDT", Side::Buy, decimal("0.3"));
        assert!(lifecycle.may_submit());
        lifecycle.begin_submission().unwrap();
        lifecycle.mark_ambiguous().unwrap();
        assert!(!lifecycle.may_submit());
        assert!(matches!(
            lifecycle.begin_submission(),
            Err(ExecutionError::DuplicateSubmission)
        ));
        lifecycle
            .reconcile_order(&order_response(OrderStatus::PartiallyFilled, "0.2"))
            .unwrap();
        assert_eq!(lifecycle.state, OrderLifecycleState::Acknowledged);
        assert_eq!(lifecycle.executed_qty, decimal("0.2"));
        assert_eq!(lifecycle.order_id, Some(44));
    }

    #[test]
    fn client_order_ids_are_repeatable_distinguish_side_and_fit_exchange_limit() {
        let symbol = SymbolId::parse("ETH/USDT").unwrap();
        let candle = Timestamp::from_millis(1_735_675_200_000).unwrap();
        let entry = deterministic_client_order_id(&symbol, Side::Buy, candle);
        assert_eq!(
            entry,
            deterministic_client_order_id(&symbol, Side::Buy, candle)
        );
        assert_ne!(
            entry,
            deterministic_client_order_id(&symbol, Side::Sell, candle)
        );
        assert!(entry.len() <= 36);
        assert!(entry.bytes().all(|b| b.is_ascii_alphanumeric()));
    }

    #[test]
    fn rejects_mismatched_order_and_overfill() {
        let mut lifecycle = OrderLifecycle::new("entry-1", "ETHUSDT", Side::Buy, decimal("0.3"));
        lifecycle.begin_submission().unwrap();
        let mut mismatched = order_response(OrderStatus::Filled, "0.3");
        mismatched.client_order_id = "other".to_owned();
        assert!(matches!(
            lifecycle.reconcile_order(&mismatched),
            Err(ExecutionError::IdentityMismatch)
        ));
        assert!(matches!(
            lifecycle.reconcile_order(&order_response(OrderStatus::Filled, "0.4")),
            Err(ExecutionError::Overfill)
        ));
    }

    #[test]
    fn trade_snapshots_deduplicate_by_trade_id_and_verify_cumulative_qty() {
        let trades = [trade(10, "0.1"), trade(11, "0.2"), trade(11, "0.2")];
        let ledger = TradeLedger::from_trades(44, &trades).unwrap();
        assert_eq!(ledger.fills.len(), 2);
        assert_eq!(ledger.executed_qty, decimal("0.3"));
        assert_eq!(ledger.quote_qty, decimal("600"));
        ledger.verify_executed_qty(decimal("0.3")).unwrap();
        assert!(matches!(
            ledger.verify_executed_qty(decimal("0.4")),
            Err(ExecutionError::FillQuantityMismatch { .. })
        ));
    }

    #[test]
    fn rejects_conflicting_or_wrong_order_trade_data() {
        let duplicate = [trade(10, "0.1"), trade(10, "0.2")];
        assert!(matches!(
            TradeLedger::from_trades(44, &duplicate),
            Err(ExecutionError::ConflictingDuplicateTrade(10))
        ));
        assert!(matches!(
            TradeLedger::from_trades(99, &[trade(10, "0.1")]),
            Err(ExecutionError::IdentityMismatch)
        ));
    }
}
