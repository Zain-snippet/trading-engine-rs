use crate::{
    error::DomainError,
    money::Money,
    timestamp::Timestamp,
    validation::{ensure_non_negative, ensure_positive_price},
};

/// One OHLCV candle whose `open_time` is the candle's opening instant in UTC.
#[derive(Debug, Clone, PartialEq)]
pub struct Candle {
    open_time: Timestamp,
    open: Money,
    high: Money,
    low: Money,
    close: Money,
    volume: f64,
}

impl Candle {
    /// Creates and validates an OHLCV candle.
    pub fn new(
        open_time: Timestamp,
        open: Money,
        high: Money,
        low: Money,
        close: Money,
        volume: f64,
    ) -> Result<Self, DomainError> {
        ensure_positive_price("open", open.as_f64())?;
        ensure_positive_price("high", high.as_f64())?;
        ensure_positive_price("low", low.as_f64())?;
        ensure_positive_price("close", close.as_f64())?;
        ensure_non_negative("volume", volume)?;

        let open_value = open.as_f64();
        let high_value = high.as_f64();
        let low_value = low.as_f64();
        let close_value = close.as_f64();

        if high_value < low_value {
            return Err(DomainError::InvalidCandle(
                "high must be greater than or equal to low",
            ));
        }
        if high_value < open_value {
            return Err(DomainError::InvalidCandle(
                "high must be greater than or equal to open",
            ));
        }
        if high_value < close_value {
            return Err(DomainError::InvalidCandle(
                "high must be greater than or equal to close",
            ));
        }
        if low_value > open_value {
            return Err(DomainError::InvalidCandle(
                "low must be less than or equal to open",
            ));
        }
        if low_value > close_value {
            return Err(DomainError::InvalidCandle(
                "low must be less than or equal to close",
            ));
        }

        Ok(Self {
            open_time,
            open,
            high,
            low,
            close,
            volume,
        })
    }

    #[must_use]
    pub const fn open_time(&self) -> Timestamp {
        self.open_time
    }

    #[must_use]
    pub const fn open(&self) -> Money {
        self.open
    }

    #[must_use]
    pub const fn high(&self) -> Money {
        self.high
    }

    #[must_use]
    pub const fn low(&self) -> Money {
        self.low
    }

    #[must_use]
    pub const fn close(&self) -> Money {
        self.close
    }

    #[must_use]
    pub const fn volume(&self) -> f64 {
        self.volume
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn money(value: f64) -> Money {
        Money::new(value).unwrap()
    }

    fn timestamp() -> Timestamp {
        Timestamp::parse_fixture("2023-09-01 00:00:00").unwrap()
    }

    #[test]
    fn accepts_a_consistent_candle() {
        let candle = Candle::new(
            timestamp(),
            money(1_645.77),
            money(1_654.53),
            money(1_645.23),
            money(1_652.2),
            22943.8273,
        )
        .unwrap();

        assert_eq!(candle.open_time(), timestamp());
        assert_eq!(candle.open().as_f64(), 1_645.77);
        assert_eq!(candle.high().as_f64(), 1_654.53);
        assert_eq!(candle.low().as_f64(), 1_645.23);
        assert_eq!(candle.close().as_f64(), 1_652.2);
        assert_eq!(candle.volume(), 22943.8273);
    }

    #[test]
    fn rejects_inconsistent_high_low_relationships() {
        assert!(matches!(
            Candle::new(
                timestamp(),
                money(10.0),
                money(9.0),
                money(10.0),
                money(10.0),
                1.0
            ),
            Err(DomainError::InvalidCandle(_))
        ));
        assert!(matches!(
            Candle::new(
                timestamp(),
                money(10.0),
                money(11.0),
                money(9.0),
                money(12.0),
                1.0
            ),
            Err(DomainError::InvalidCandle(_))
        ));
        assert!(matches!(
            Candle::new(
                timestamp(),
                money(8.0),
                money(11.0),
                money(9.0),
                money(10.0),
                1.0
            ),
            Err(DomainError::InvalidCandle(_))
        ));
    }

    #[test]
    fn rejects_non_positive_prices() {
        assert!(matches!(
            Candle::new(
                timestamp(),
                money(0.0),
                money(1.0),
                money(0.0),
                money(1.0),
                1.0
            ),
            Err(DomainError::InvalidPrice { field: "open", .. })
        ));
    }

    #[test]
    fn rejects_negative_or_non_finite_volume() {
        assert!(matches!(
            Candle::new(
                timestamp(),
                money(10.0),
                money(11.0),
                money(9.0),
                money(10.0),
                -1.0
            ),
            Err(DomainError::Negative {
                field: "volume",
                ..
            })
        ));
        assert!(matches!(
            Candle::new(
                timestamp(),
                money(10.0),
                money(11.0),
                money(9.0),
                money(10.0),
                f64::NAN
            ),
            Err(DomainError::NonFinite {
                field: "volume",
                ..
            })
        ));
    }

    #[test]
    fn allows_zero_volume() {
        let candle = Candle::new(
            timestamp(),
            money(10.0),
            money(11.0),
            money(9.0),
            money(10.0),
            0.0,
        )
        .unwrap();
        assert_eq!(candle.volume(), 0.0);
    }
}
