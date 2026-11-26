use crate::{
    error::DomainError,
    money::Money,
    order::{Fill, Side},
    symbol::SymbolId,
    timestamp::Timestamp,
    validation::{ensure_non_negative, ensure_positive_price},
};

/// An open long spot position.
///
/// The original bot is long-only and one position per symbol, so this type
/// intentionally has no short side. It also stores the fixed stop and the
/// entry-to-stop risk in quote currency, matching the Python position record.
#[derive(Debug, Clone, PartialEq)]
pub struct Position {
    symbol: SymbolId,
    quantity: f64,
    entry_price: Money,
    entry_time: Timestamp,
    entry_fee: Money,
    stop_price: Money,
    atr: Money,
    risk_quote: Money,
}

impl Position {
    /// Opens a long position from a buy fill.
    pub fn from_entry_fill(
        fill: &Fill,
        stop_price: Money,
        atr: Money,
    ) -> Result<Self, DomainError> {
        if fill.side() != Side::Buy {
            return Err(DomainError::InvalidPosition(
                "entry fill must be a buy fill",
            ));
        }
        ensure_positive_price("stop_price", stop_price.as_f64())?;
        if stop_price.as_f64() >= fill.price().as_f64() {
            return Err(DomainError::InvalidPosition(
                "stop price must be below the entry price for a long position",
            ));
        }
        ensure_non_negative("atr", atr.as_f64())?;

        // The Python engine stores `qty * (entry - stop)` as the quote risk.
        let risk_quote =
            Money::new(fill.quantity() * (fill.price().as_f64() - stop_price.as_f64()))?;
        if !risk_quote.is_positive() {
            return Err(DomainError::InvalidPosition(
                "risk_quote must be positive for an open position",
            ));
        }

        Ok(Self {
            symbol: fill.symbol().clone(),
            quantity: fill.quantity(),
            entry_price: fill.price(),
            entry_time: fill.time(),
            entry_fee: fill.fee_quote(),
            stop_price,
            atr,
            risk_quote,
        })
    }

    #[must_use]
    pub fn symbol(&self) -> &SymbolId {
        &self.symbol
    }

    #[must_use]
    pub const fn quantity(&self) -> f64 {
        self.quantity
    }

    #[must_use]
    pub const fn entry_price(&self) -> Money {
        self.entry_price
    }

    #[must_use]
    pub const fn entry_time(&self) -> Timestamp {
        self.entry_time
    }

    #[must_use]
    pub const fn entry_fee(&self) -> Money {
        self.entry_fee
    }

    #[must_use]
    pub const fn stop_price(&self) -> Money {
        self.stop_price
    }

    #[must_use]
    pub const fn atr(&self) -> Money {
        self.atr
    }

    #[must_use]
    pub const fn risk_quote(&self) -> Money {
        self.risk_quote
    }

    /// Returns true when the supplied price has reached or crossed the stop.
    #[must_use]
    pub fn is_stop_hit(&self, price: Money) -> bool {
        price.as_f64() <= self.stop_price.as_f64()
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

    fn buy_fill() -> Fill {
        Fill::new(
            symbol(),
            Side::Buy,
            1.25,
            money(2_000.0),
            money(2.5),
            timestamp(),
        )
        .unwrap()
    }

    #[test]
    fn opens_a_long_position_from_a_buy_fill() {
        let position = Position::from_entry_fill(&buy_fill(), money(1_920.0), money(40.0)).unwrap();

        assert_eq!(position.symbol().base(), "ETH");
        assert_eq!(position.quantity(), 1.25);
        assert_eq!(position.entry_price().as_f64(), 2_000.0);
        assert_eq!(position.entry_time(), timestamp());
        assert_eq!(position.entry_fee().as_f64(), 2.5);
        assert_eq!(position.stop_price().as_f64(), 1_920.0);
        assert_eq!(position.atr().as_f64(), 40.0);
        assert_eq!(position.risk_quote().as_f64(), 100.0);
    }

    #[test]
    fn rejects_a_sell_fill_as_an_entry() {
        let fill = Fill::new(
            symbol(),
            Side::Sell,
            1.0,
            money(2_000.0),
            money(1.0),
            timestamp(),
        )
        .unwrap();

        assert!(matches!(
            Position::from_entry_fill(&fill, money(1_920.0), money(40.0)),
            Err(DomainError::InvalidPosition(_))
        ));
    }

    #[test]
    fn rejects_stop_at_or_above_entry() {
        assert!(matches!(
            Position::from_entry_fill(&buy_fill(), money(2_000.0), money(40.0)),
            Err(DomainError::InvalidPosition(_))
        ));
        assert!(matches!(
            Position::from_entry_fill(&buy_fill(), money(2_100.0), money(40.0)),
            Err(DomainError::InvalidPosition(_))
        ));
    }

    #[test]
    fn rejects_negative_atr() {
        assert!(matches!(
            Position::from_entry_fill(&buy_fill(), money(1_920.0), money(-1.0)),
            Err(DomainError::Negative { field: "atr", .. })
        ));
    }

    #[test]
    fn detects_stop_hits() {
        let position = Position::from_entry_fill(&buy_fill(), money(1_920.0), money(40.0)).unwrap();

        assert!(!position.is_stop_hit(money(1_920.01)));
        assert!(position.is_stop_hit(money(1_920.0)));
        assert!(position.is_stop_hit(money(1_800.0)));
    }
}
