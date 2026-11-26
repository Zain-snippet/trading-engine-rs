use crate::{
    error::DomainError,
    money::Money,
    symbol::SymbolId,
    timestamp::Timestamp,
    validation::{ensure_non_negative, ensure_positive_price},
};

/// A strategy signal evaluated on one closed candle.
///
/// Channel levels and other indicator outputs are deliberately not stored here;
/// they belong to the indicator/strategy layer. This type mirrors the
/// decision-relevant part of the Python `compute_signal` result.
#[derive(Debug, Clone, PartialEq)]
pub struct Signal {
    symbol: SymbolId,
    candle: Timestamp,
    close: Money,
    atr: Money,
    enter_long: bool,
    exit_long: bool,
}

impl Signal {
    /// Creates a signal and validates its numerical fields.
    pub fn new(
        symbol: SymbolId,
        candle: Timestamp,
        close: Money,
        atr: Money,
        enter_long: bool,
        exit_long: bool,
    ) -> Result<Self, DomainError> {
        ensure_positive_price("close", close.as_f64())?;
        ensure_non_negative("atr", atr.as_f64())?;

        Ok(Self {
            symbol,
            candle,
            close,
            atr,
            enter_long,
            exit_long,
        })
    }

    #[must_use]
    pub fn symbol(&self) -> &SymbolId {
        &self.symbol
    }

    #[must_use]
    pub const fn candle(&self) -> Timestamp {
        self.candle
    }

    #[must_use]
    pub const fn close(&self) -> Money {
        self.close
    }

    #[must_use]
    pub const fn atr(&self) -> Money {
        self.atr
    }

    #[must_use]
    pub const fn enter_long(&self) -> bool {
        self.enter_long
    }

    #[must_use]
    pub const fn exit_long(&self) -> bool {
        self.exit_long
    }

    #[must_use]
    pub const fn is_actionable(&self) -> bool {
        self.enter_long || self.exit_long
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
    fn preserves_signal_flags_and_values() {
        let signal = Signal::new(
            symbol(),
            timestamp(),
            money(2_000.0),
            money(40.0),
            true,
            false,
        )
        .unwrap();

        assert_eq!(signal.symbol().base(), "ETH");
        assert_eq!(signal.candle(), timestamp());
        assert_eq!(signal.close().as_f64(), 2_000.0);
        assert_eq!(signal.atr().as_f64(), 40.0);
        assert!(signal.enter_long());
        assert!(!signal.exit_long());
        assert!(signal.is_actionable());
    }

    #[test]
    fn allows_zero_atr_and_no_action() {
        let signal = Signal::new(
            symbol(),
            timestamp(),
            money(2_000.0),
            money(0.0),
            false,
            false,
        )
        .unwrap();
        assert!(!signal.is_actionable());
    }

    #[test]
    fn rejects_non_positive_close() {
        assert!(matches!(
            Signal::new(symbol(), timestamp(), money(0.0), money(40.0), true, false),
            Err(DomainError::InvalidPrice { field: "close", .. })
        ));
    }

    #[test]
    fn rejects_negative_atr() {
        assert!(matches!(
            Signal::new(
                symbol(),
                timestamp(),
                money(2_000.0),
                money(-1.0),
                true,
                false
            ),
            Err(DomainError::Negative { field: "atr", .. })
        ));
    }
}
