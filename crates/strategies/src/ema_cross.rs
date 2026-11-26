use trading_engine_config::ATR_PERIOD;
use trading_engine_core::Candle;
use trading_engine_indicators::{atr, ema};

use crate::{Strategy, StrategyBar, StrategyError};

/// Dual-EMA crossover strategy matching `tradebot.strategies.EmaCross`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EmaCross {
    pub fast: usize,
    pub slow: usize,
    pub stop_mult: f64,
    pub trailing: bool,
}

impl Default for EmaCross {
    fn default() -> Self {
        Self {
            fast: 20,
            slow: 100,
            stop_mult: 3.0,
            trailing: true,
        }
    }
}

impl EmaCross {
    /// Creates a validated `EmaCross` strategy with `trailing = true`.
    pub fn new(fast: usize, slow: usize, stop_mult: f64) -> Result<Self, StrategyError> {
        if fast == 0 || slow == 0 {
            return Err(StrategyError::InvalidParameter {
                strategy: "ema_cross",
                reason: "fast and slow periods must be >= 1".to_owned(),
            });
        }
        if fast >= slow {
            return Err(StrategyError::InvalidParameter {
                strategy: "ema_cross",
                reason: format!("fast ({fast}) must be < slow ({slow})"),
            });
        }
        if !stop_mult.is_finite() || stop_mult <= 0.0 {
            return Err(StrategyError::InvalidParameter {
                strategy: "ema_cross",
                reason: format!("stop_mult must be positive and finite, got {stop_mult}"),
            });
        }

        Ok(Self {
            fast,
            slow,
            stop_mult,
            trailing: true,
        })
    }

    /// Parameter grid matching `EmaCross.grid()` in `tradebot/strategies.py`.
    #[must_use]
    pub fn grid() -> Vec<Self> {
        let mut out = Vec::new();
        for fast in [10, 20, 50] {
            for slow in [50, 100, 200] {
                if fast < slow {
                    for stop_mult in [2.0, 3.0, 4.0] {
                        out.push(Self {
                            fast,
                            slow,
                            stop_mult,
                            trailing: true,
                        });
                    }
                }
            }
        }
        out
    }
}

fn cross_up(a: &[Option<f64>], b: &[Option<f64>], i: usize) -> bool {
    if i == 0 {
        return false;
    }
    matches!(
        (a[i], b[i], a[i - 1], b[i - 1]),
        (Some(ai), Some(bi), Some(ap), Some(bp)) if ai > bi && ap <= bp
    )
}

impl Strategy for EmaCross {
    fn name(&self) -> &'static str {
        "ema_cross"
    }

    fn stop_mult(&self) -> f64 {
        self.stop_mult
    }

    fn trailing(&self) -> bool {
        self.trailing
    }

    fn signals(&self, candles: &[Candle]) -> Result<Vec<StrategyBar>, StrategyError> {
        let closes: Vec<f64> = candles.iter().map(|c| c.close().as_f64()).collect();
        let fast_ema = ema(&closes, self.fast)?;
        let slow_ema = ema(&closes, self.slow)?;
        let atrs = atr(candles, ATR_PERIOD)?;

        let bars = (0..candles.len())
            .map(|i| {
                let (exit_long, exit_short) = match (fast_ema[i], slow_ema[i]) {
                    (Some(f), Some(s)) => (f < s, f > s),
                    _ => (false, false),
                };
                StrategyBar {
                    enter_long: cross_up(&fast_ema, &slow_ema, i),
                    exit_long,
                    enter_short: cross_up(&slow_ema, &fast_ema, i),
                    exit_short,
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
    fn detects_ema_crossover_and_ignores_warmup_boundaries() {
        // Declining then rising closes to trigger fast crossing above slow.
        let prices = [100.0, 95.0, 90.0, 85.0, 80.0, 95.0, 115.0, 130.0];
        let candles: Vec<Candle> = prices
            .iter()
            .enumerate()
            .map(|(i, &p)| candle(i as i64, p))
            .collect();

        let strat = EmaCross::new(2, 4, 3.0).unwrap();
        let bars = strat.signals(&candles).unwrap();

        // First bar where slow_ema(4) is Some is index 3, so index 3 cannot cross_up
        // because slow_ema[2] was None (matching pandas NaN shift behavior).
        assert!(!bars[3].enter_long);
        assert!(bars.iter().any(|b| b.enter_long));
    }

    #[test]
    fn grid_matches_python_combinations() {
        // fast < slow pairs from [10, 20, 50] x [50, 100, 200]:
        // 10 < (50,100,200) -> 3; 20 < (50,100,200) -> 3; 50 < (100,200) -> 2 => 8 * 3 = 24
        assert_eq!(EmaCross::grid().len(), 24);
    }

    #[test]
    fn rejects_invalid_periods() {
        assert!(EmaCross::new(0, 10, 3.0).is_err());
        assert!(EmaCross::new(10, 10, 3.0).is_err());
        assert!(EmaCross::new(20, 10, 3.0).is_err());
    }
}
