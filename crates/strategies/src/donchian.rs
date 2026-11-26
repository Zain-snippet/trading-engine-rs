use trading_engine_config::{SymbolConfig, ATR_PERIOD};
use trading_engine_core::{Candle, Money, Signal, SymbolId};
use trading_engine_indicators::{atr, donchian_channels};

use crate::{Strategy, StrategyBar, StrategyError};

/// Turtle-style Donchian breakout strategy matching `tradebot.strategies.Donchian`.
///
/// Uses shifted rolling extrema so the current candle's high and low are excluded
/// from the channel levels. Defaults to `trailing = false` (fixed initial stop).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Donchian {
    pub entry_n: usize,
    pub exit_n: usize,
    pub stop_mult: f64,
    pub trailing: bool,
}

/// Evaluation of the latest closed candle for live/paper trading, matching
/// `paperbot.engine.compute_signal`.
#[derive(Debug, Clone, PartialEq)]
pub struct DonchianEvaluation {
    pub signal: Signal,
    pub entry_level: Option<f64>,
    pub exit_level: Option<f64>,
}

impl Default for Donchian {
    fn default() -> Self {
        Self {
            entry_n: 55,
            exit_n: 20,
            stop_mult: 2.0,
            trailing: false,
        }
    }
}

impl Donchian {
    /// Creates a validated `Donchian` strategy with `trailing = false`.
    pub fn new(entry_n: usize, exit_n: usize, stop_mult: f64) -> Result<Self, StrategyError> {
        if entry_n == 0 {
            return Err(StrategyError::InvalidParameter {
                strategy: "donchian",
                reason: "entry_n must be >= 1".to_owned(),
            });
        }
        if exit_n == 0 {
            return Err(StrategyError::InvalidParameter {
                strategy: "donchian",
                reason: "exit_n must be >= 1".to_owned(),
            });
        }
        if !stop_mult.is_finite() || stop_mult <= 0.0 {
            return Err(StrategyError::InvalidParameter {
                strategy: "donchian",
                reason: format!("stop_mult must be positive and finite, got {stop_mult}"),
            });
        }

        Ok(Self {
            entry_n,
            exit_n,
            stop_mult,
            trailing: false,
        })
    }

    /// Constructs a `Donchian` strategy from a validated `SymbolConfig`.
    pub fn from_symbol_config(config: &SymbolConfig) -> Result<Self, StrategyError> {
        Self::new(config.entry_n, config.exit_n, config.stop_mult)
    }

    /// Parameter grid matching `Donchian.grid()` in `tradebot/strategies.py`.
    #[must_use]
    pub fn grid() -> Vec<Self> {
        let mut out = Vec::new();
        for entry_n in [20, 55, 100] {
            for exit_n in [10, 20, 50] {
                if exit_n < entry_n {
                    for stop_mult in [2.0, 3.0] {
                        out.push(Self {
                            entry_n,
                            exit_n,
                            stop_mult,
                            trailing: false,
                        });
                    }
                }
            }
        }
        out
    }

    /// Evaluates the latest closed candle, matching `paperbot.engine.compute_signal`.
    pub fn latest_signal(
        &self,
        symbol: &SymbolId,
        candles: &[Candle],
    ) -> Result<DonchianEvaluation, StrategyError> {
        let Some(last_candle) = candles.last() else {
            return Err(StrategyError::EmptyCandles);
        };

        let levels = donchian_channels(candles, self.entry_n, self.exit_n)?;
        let atrs = atr(candles, ATR_PERIOD)?;

        let idx = candles.len() - 1;
        let last_level = levels[idx];
        let last_atr = atrs[idx].ok_or(StrategyError::MissingAtr { candle_index: idx })?;
        let close = last_candle.close().as_f64();

        let enter_long = last_level.entry_upper.is_some_and(|hh| close > hh);
        let exit_long = last_level.exit_lower.is_some_and(|xl| close < xl);

        let signal = Signal::new(
            symbol.clone(),
            last_candle.open_time(),
            last_candle.close(),
            Money::new(last_atr)?,
            enter_long,
            exit_long,
        )?;

        Ok(DonchianEvaluation {
            signal,
            entry_level: last_level.entry_upper,
            exit_level: last_level.exit_lower,
        })
    }
}

impl Strategy for Donchian {
    fn name(&self) -> &'static str {
        "donchian"
    }

    fn stop_mult(&self) -> f64 {
        self.stop_mult
    }

    fn trailing(&self) -> bool {
        self.trailing
    }

    fn signals(&self, candles: &[Candle]) -> Result<Vec<StrategyBar>, StrategyError> {
        let levels = donchian_channels(candles, self.entry_n, self.exit_n)?;
        let atrs = atr(candles, ATR_PERIOD)?;

        let bars = candles
            .iter()
            .zip(levels)
            .zip(atrs)
            .map(|((candle, level), atr_val)| {
                let close = candle.close().as_f64();
                StrategyBar {
                    enter_long: level.entry_upper.is_some_and(|hh| close > hh),
                    exit_long: level.exit_lower.is_some_and(|xl| close < xl),
                    enter_short: level.entry_lower.is_some_and(|ll| close < ll),
                    exit_short: level.exit_upper.is_some_and(|xh| close > xh),
                    atr: atr_val,
                }
            })
            .collect();

        Ok(bars)
    }
}

#[cfg(test)]
mod tests {
    use trading_engine_core::Timestamp;

    use super::*;

    fn candle(idx: i64, high: f64, low: f64, close: f64) -> Candle {
        Candle::new(
            Timestamp::from_millis(1_700_000_000_000 + idx * 14_400_000).unwrap(),
            Money::new(close).unwrap(),
            Money::new(high).unwrap(),
            Money::new(low).unwrap(),
            Money::new(close).unwrap(),
            100.0,
        )
        .unwrap()
    }

    #[test]
    fn generates_breakout_and_exit_signals_excluding_current_bar() {
        // 15 candles so ATR(14) is warmed up at index 13 and 14.
        let mut candles: Vec<Candle> = (0..13).map(|i| candle(i, 100.0, 90.0, 95.0)).collect();
        // Bar 13 closes at 105 (above prior 3-bar high of 100) -> enter_long = true
        candles.push(candle(13, 106.0, 95.0, 105.0));
        // Bar 14 closes at 88 (below prior 2-bar low of 90) -> exit_long = true
        candles.push(candle(14, 104.0, 87.0, 88.0));

        let strat = Donchian::new(3, 2, 2.0).unwrap();
        let bars = strat.signals(&candles).unwrap();

        assert!(bars[13].enter_long);
        assert!(!bars[13].exit_long);
        assert!(bars[13].atr.is_some());

        assert!(!bars[14].enter_long);
        assert!(bars[14].exit_long);

        let sym = SymbolId::parse("ETH/USDT").unwrap();
        let eval = strat.latest_signal(&sym, &candles).unwrap();
        assert!(!eval.signal.enter_long());
        assert!(eval.signal.exit_long());
        assert_eq!(eval.entry_level, Some(106.0));
        assert_eq!(eval.exit_level, Some(90.0));
    }

    #[test]
    fn grid_matches_python_combinations() {
        let g = Donchian::grid();
        // (20,10), (55,10), (55,20), (55,50), (100,10), (100,20), (100,50) => 7 * 2 = 14
        assert_eq!(g.len(), 14);
    }

    #[test]
    fn rejects_invalid_parameters_and_insufficient_atr_history() {
        assert!(Donchian::new(0, 10, 2.0).is_err());
        assert!(Donchian::new(10, 0, 2.0).is_err());
        assert!(Donchian::new(10, 5, 0.0).is_err());
        assert!(Donchian::new(10, 5, f64::NAN).is_err());

        let strat = Donchian::new(3, 2, 2.0).unwrap();
        let sym = SymbolId::parse("ETH/USDT").unwrap();
        assert!(matches!(
            strat.latest_signal(&sym, &[]),
            Err(StrategyError::EmptyCandles)
        ));
        let short_candles: Vec<Candle> = (0..13).map(|i| candle(i, 100.0, 90.0, 95.0)).collect();
        assert!(matches!(
            strat.latest_signal(&sym, &short_candles),
            Err(StrategyError::MissingAtr { candle_index: 12 })
        ));
    }
}
