use crate::{
    error::DomainError,
    money::Money,
    symbol::SymbolId,
    timestamp::Timestamp,
    validation::{ensure_non_negative, ensure_positive_price, ensure_positive_quantity},
};

/// The side of a spot order or fill.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Side {
    Buy,
    Sell,
}

impl Side {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Buy => "buy",
            Self::Sell => "sell",
        }
    }
}

/// A completed fill reported by a broker.
#[derive(Debug, Clone, PartialEq)]
pub struct Fill {
    symbol: SymbolId,
    side: Side,
    quantity: f64,
    price: Money,
    fee_quote: Money,
    time: Timestamp,
    client_order_id: Option<String>,
    order_id: Option<String>,
}

impl Fill {
    /// Creates a fill and validates its numerical fields.
    pub fn new(
        symbol: SymbolId,
        side: Side,
        quantity: f64,
        price: Money,
        fee_quote: Money,
        time: Timestamp,
    ) -> Result<Self, DomainError> {
        ensure_positive_quantity(quantity)?;
        ensure_positive_price("price", price.as_f64())?;
        ensure_non_negative("fee_quote", fee_quote.as_f64())?;

        Ok(Self {
            symbol,
            side,
            quantity,
            price,
            fee_quote,
            time,
            client_order_id: None,
            order_id: None,
        })
    }

    #[must_use]
    pub fn with_client_order_id(mut self, client_order_id: impl Into<String>) -> Self {
        self.client_order_id = Some(client_order_id.into());
        self
    }

    #[must_use]
    pub fn with_order_id(mut self, order_id: impl Into<String>) -> Self {
        self.order_id = Some(order_id.into());
        self
    }

    #[must_use]
    pub fn symbol(&self) -> &SymbolId {
        &self.symbol
    }

    #[must_use]
    pub const fn side(&self) -> Side {
        self.side
    }

    #[must_use]
    pub const fn quantity(&self) -> f64 {
        self.quantity
    }

    #[must_use]
    pub const fn price(&self) -> Money {
        self.price
    }

    #[must_use]
    pub const fn fee_quote(&self) -> Money {
        self.fee_quote
    }

    #[must_use]
    pub const fn time(&self) -> Timestamp {
        self.time
    }

    #[must_use]
    pub fn client_order_id(&self) -> Option<&str> {
        self.client_order_id.as_deref()
    }

    #[must_use]
    pub fn order_id(&self) -> Option<&str> {
        self.order_id.as_deref()
    }
}

/// A request to place a market order.
#[derive(Debug, Clone, PartialEq)]
pub struct OrderIntent {
    symbol: SymbolId,
    side: Side,
    quantity: f64,
    client_order_id: Option<String>,
    reason: Option<String>,
}

impl OrderIntent {
    /// Creates an order intent and validates its quantity.
    pub fn new(symbol: SymbolId, side: Side, quantity: f64) -> Result<Self, DomainError> {
        ensure_positive_quantity(quantity)?;

        Ok(Self {
            symbol,
            side,
            quantity,
            client_order_id: None,
            reason: None,
        })
    }

    #[must_use]
    pub fn with_client_order_id(mut self, client_order_id: impl Into<String>) -> Self {
        self.client_order_id = Some(client_order_id.into());
        self
    }

    #[must_use]
    pub fn with_reason(mut self, reason: impl Into<String>) -> Self {
        self.reason = Some(reason.into());
        self
    }

    #[must_use]
    pub fn symbol(&self) -> &SymbolId {
        &self.symbol
    }

    #[must_use]
    pub const fn side(&self) -> Side {
        self.side
    }

    #[must_use]
    pub const fn quantity(&self) -> f64 {
        self.quantity
    }

    #[must_use]
    pub fn client_order_id(&self) -> Option<&str> {
        self.client_order_id.as_deref()
    }

    #[must_use]
    pub fn reason(&self) -> Option<&str> {
        self.reason.as_deref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn symbol() -> SymbolId {
        SymbolId::parse("ETH/USDT").unwrap()
    }

    fn money(value: f64) -> Money {
        Money::new(value).unwrap()
    }

    fn timestamp() -> Timestamp {
        Timestamp::parse_fixture("2024-01-01 00:00:00").unwrap()
    }

    #[test]
    fn creates_a_fill_with_optional_identifiers() {
        let fill = Fill::new(
            symbol(),
            Side::Buy,
            0.5,
            money(2_000.0),
            money(1.0),
            timestamp(),
        )
        .unwrap()
        .with_client_order_id("dcETH123")
        .with_order_id("42");

        assert_eq!(fill.symbol().binance_symbol(), "ETHUSDT");
        assert_eq!(fill.side(), Side::Buy);
        assert_eq!(fill.quantity(), 0.5);
        assert_eq!(fill.price().as_f64(), 2_000.0);
        assert_eq!(fill.fee_quote().as_f64(), 1.0);
        assert_eq!(fill.time(), timestamp());
        assert_eq!(fill.client_order_id(), Some("dcETH123"));
        assert_eq!(fill.order_id(), Some("42"));
    }

    #[test]
    fn rejects_invalid_fill_values() {
        assert!(matches!(
            Fill::new(
                symbol(),
                Side::Buy,
                0.0,
                money(2_000.0),
                money(1.0),
                timestamp()
            ),
            Err(DomainError::InvalidQuantity { .. })
        ));
        assert!(matches!(
            Fill::new(
                symbol(),
                Side::Buy,
                -1.0,
                money(2_000.0),
                money(1.0),
                timestamp()
            ),
            Err(DomainError::InvalidQuantity { .. })
        ));
        assert!(matches!(
            Fill::new(
                symbol(),
                Side::Buy,
                f64::NAN,
                money(2_000.0),
                money(1.0),
                timestamp()
            ),
            Err(DomainError::InvalidQuantity { .. })
        ));
        assert!(matches!(
            Fill::new(
                symbol(),
                Side::Buy,
                1.0,
                money(0.0),
                money(1.0),
                timestamp()
            ),
            Err(DomainError::InvalidPrice { field: "price", .. })
        ));
        assert!(matches!(
            Fill::new(
                symbol(),
                Side::Buy,
                1.0,
                money(2_000.0),
                money(-1.0),
                timestamp()
            ),
            Err(DomainError::Negative {
                field: "fee_quote",
                ..
            })
        ));
    }

    #[test]
    fn creates_an_order_intent_with_metadata() {
        let intent = OrderIntent::new(symbol(), Side::Sell, 0.25)
            .unwrap()
            .with_client_order_id("dcETH456")
            .with_reason("stop");

        assert_eq!(intent.side(), Side::Sell);
        assert_eq!(intent.quantity(), 0.25);
        assert_eq!(intent.client_order_id(), Some("dcETH456"));
        assert_eq!(intent.reason(), Some("stop"));
    }

    #[test]
    fn rejects_invalid_order_quantity() {
        assert!(matches!(
            OrderIntent::new(symbol(), Side::Buy, 0.0),
            Err(DomainError::InvalidQuantity { .. })
        ));
        assert!(matches!(
            OrderIntent::new(symbol(), Side::Buy, f64::INFINITY),
            Err(DomainError::InvalidQuantity { .. })
        ));
    }

    #[test]
    fn renders_sides() {
        assert_eq!(Side::Buy.as_str(), "buy");
        assert_eq!(Side::Sell.as_str(), "sell");
    }
}
