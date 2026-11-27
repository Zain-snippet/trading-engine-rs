use trading_engine_config::ATR_PERIOD;
use trading_engine_core::Candle;
use trading_engine_indicators::{atr, ema, rsi};

use crate::{Strategy, StrategyBar, StrategyError};

/// RSI mean-reversion strategy with EMA trend filter matching `tradebot.strategies.RsiReversion`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RsiReversion {
    pub rsi_n: usize,
    pub entry: f64,
    pub exit: f64,
    pub trend_n: usize,
    pub stop_mult: f64,
    pub trailing: bool,
}

impl Default for RsiReversion {
    fn default() -> Self {
        Self {
            rsi_n: 14,
            entry: 30.0,
            exit: 55.0,
            trend_n: 200,
            stop_mult: 2.0,
            trailing: false,
        }
    }
}

impl RsiReversion {
    /// Creates a validated `RsiReversion` strategy with `trailing = false`.
    pub fn new(
        rsi_n: usize,
        entry: f64,
        exit: f64,
        trend_n: usize,
        stop_mult: f64,
    ) -> Result<Self, StrategyError> {
        if rsi_n == 0 || trend_n == 0 {
            return Err(StrategyError::InvalidParameter {
                strategy: "rsi_mr",
                reason: "rsi_n and trend_n must be >= 1".to_owned(),
            });
        }
        if !entry.is_finite() || !exit.is_finite() || !stop_mult.is_finite() || stop_mult <= 0.0 {
            return Err(StrategyError::InvalidParameter {
                strategy: "rsi_mr",
                reason: "entry, exit, and positive stop_mult must be finite".to_owned(),
            });
        }

        Ok(Self {
            rsi_n,
            entry,
            exit,
            trend_n,
            stop_mult,
            trailing: false,
        })
    }

    /// Parameter grid matching `RsiReversion.grid()` in `tradebot/strategies.py`.
    #[must_use]
    pub fn grid() -> Vec<Self> {
        let mut out = Vec::new();
        for rsi_n in [7, 14] {
            for entry in [25.0, 30.0, 35.0] {
                for exit in [50.0, 65.0] {
                    for stop_mult in [2.0, 3.0] {
                        out.push(Self {
                            rsi_n,
                            entry,
                            exit,
                            trend_n: 200,
                            stop_mult,
                            trailing: false,
                        });
                    }
                }
            }
        }
        out
    }
}

impl Strategy for RsiReversion {
    fn name(&self) -> &'static str {
        "rsi_mr"
    }

    fn stop_mult(&self) -> f64 {
        self.stop_mult
    }

    fn trailing(&self) -> bool {
        self.trailing
    }

    fn signals(&self, candles: &[Candle]) -> Result<Vec<StrategyBar>, StrategyError> {
        let closes: Vec<f64> = candles.iter().map(|c| c.close().as_f64()).collect();
        let rsi_vals = rsi(&closes, self.rsi_n)?;
        let trend = ema(&closes, self.trend_n)?;
        let atrs = atr(candles, ATR_PERIOD)?;

        let bars = (0..candles.len())
            .map(|i| {
                let close = closes[i];
                let enter_long = matches!(
                    (rsi_vals[i], trend[i]),
                    (Some(r), Some(t)) if r < self.entry && close > t
                );
                let exit_long = rsi_vals[i].is_some_and(|r| r > self.exit);
                StrategyBar {
                    enter_long,
                    exit_long,
                    atr: atrs[i],
                }
            })
            .collect();

        Ok(bars)
    }
}

#[cfg(test)]
mod tests {
    use trading_engine_core::{Money, Timestamp};

    use super::*;

    fn candle(idx: i64, close: f64) -> Candle {
        Candle::new(
            Timestamp::from_millis(1_700_000_000_000 + idx * 14_400_000).unwrap(),
            Money::new(close).unwrap(),
            Money::new(close + 1.0).unwrap(),
            Money::new(close - 1.0).unwrap(),
            Money::new(close).unwrap(),
            10.0,
        )
        .unwrap()
    }

    #[test]
    fn generates_mean_reversion_entry_above_trend() {
        // Strong uptrend followed by a sharp pullback so close > trend EMA while RSI drops < 40.
        let prices = [100.0, 102.0, 105.0, 110.0, 120.0, 130.0, 117.0];
        let candles: Vec<Candle> = prices
            .iter()
            .enumerate()
            .map(|(i, &p)| candle(i as i64, p))
            .collect();

        let strat = RsiReversion::new(2, 40.0, 60.0, 6, 2.0).unwrap();
        let bars = strat.signals(&candles).unwrap();

        assert!(bars[6].enter_long);
    }

    #[test]
    fn grid_matches_python_combinations() {
        // 2 * 3 * 2 * 2 = 24
        assert_eq!(RsiReversion::grid().len(), 24);
    }

    #[test]
    fn rejects_invalid_parameters() {
        assert!(RsiReversion::new(0, 30.0, 55.0, 200, 2.0).is_err());
        assert!(RsiReversion::new(14, 30.0, 55.0, 0, 2.0).is_err());
        assert!(RsiReversion::new(14, f64::NAN, 55.0, 200, 2.0).is_err());
    }
}
