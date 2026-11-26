use trading_engine_core::Candle;

use crate::error::IndicatorError;

/// Validates that `period >= 1` and all values in `values` are finite.
fn validate_series(
    indicator: &'static str,
    values: &[f64],
    period: usize,
) -> Result<(), IndicatorError> {
    if period == 0 {
        return Err(IndicatorError::InvalidPeriod { indicator, period });
    }
    for (index, value) in values.iter().enumerate() {
        if !value.is_finite() {
            return Err(IndicatorError::NonFiniteInput { indicator, index });
        }
    }
    Ok(())
}

/// Core pandas-compatible EWM recurrence: `ewm(alpha=alpha, adjust=False, min_periods=min_periods)`.
///
/// Assumes all values in `values` are finite. When `values` is non-empty, the
/// state initializes at `values[0]` and emits `Some(state)` once at least
/// `min_periods` observations have been processed.
fn ewma_finite(values: &[f64], alpha: f64, min_periods: usize) -> Vec<Option<f64>> {
    let mut out = Vec::with_capacity(values.len());
    let mut state: Option<f64> = None;

    for (idx, &value) in values.iter().enumerate() {
        let next = match state {
            None => value,
            Some(prev) => (1.0 - alpha) * prev + alpha * value,
        };
        state = Some(next);
        let nobs = idx + 1;
        out.push(if nobs >= min_periods {
            Some(next)
        } else {
            None
        });
    }

    out
}

/// Exponential moving average matching `s.ewm(span=period, adjust=False, min_periods=period).mean()`.
pub fn ema(values: &[f64], period: usize) -> Result<Vec<Option<f64>>, IndicatorError> {
    validate_series("ema", values, period)?;
    let alpha = 2.0 / (period as f64 + 1.0);
    Ok(ewma_finite(values, alpha, period))
}

/// True range per candle matching `tradebot/indicators.py`:
/// - bar 0: `high - low` (since `close.shift()` is `NaN` and `max(axis=1)` skips `NaN`)
/// - bar i >= 1: `max(high - low, |high - prev_close|, |low - prev_close|)`
#[must_use]
pub fn true_range(candles: &[Candle]) -> Vec<f64> {
    let mut tr = Vec::with_capacity(candles.len());
    for (i, candle) in candles.iter().enumerate() {
        let high = candle.high().as_f64();
        let low = candle.low().as_f64();
        let hl = high - low;
        let range = if i == 0 {
            hl
        } else {
            let prev_close = candles[i - 1].close().as_f64();
            hl.max((high - prev_close).abs())
                .max((low - prev_close).abs())
        };
        tr.push(range);
    }
    tr
}

/// Average True Range matching `tradebot.indicators.atr(df, n=period)`:
/// `tr.ewm(alpha=1 / period, adjust=False, min_periods=period).mean()`.
///
/// The first valid output is at index `period - 1` (e.g., index 13 for `period = 14`).
pub fn atr(candles: &[Candle], period: usize) -> Result<Vec<Option<f64>>, IndicatorError> {
    if period == 0 {
        return Err(IndicatorError::InvalidPeriod {
            indicator: "atr",
            period,
        });
    }
    let tr = true_range(candles);
    let alpha = 1.0 / (period as f64);
    Ok(ewma_finite(&tr, alpha, period))
}

/// Relative Strength Index matching `tradebot.indicators.rsi(s, n=period)`:
/// `d = s.diff()`, `up = d.clip(lower=0).ewm(alpha=1/n, adjust=False, min_periods=n).mean()`,
/// `dn = (-d.clip(upper=0)).ewm(alpha=1/n, adjust=False, min_periods=n).mean()`,
/// `100 - 100 / (1 + up / dn)`.
///
/// Index 0 has no prior close (`diff()` is `NaN`), so `n` diffs are observed at
/// index `period`, making index `period` the first non-`None` output when prices
/// are non-flat. Flat regions (`up == 0 && dn == 0`) yield `None` (matching `NaN`).
pub fn rsi(values: &[f64], period: usize) -> Result<Vec<Option<f64>>, IndicatorError> {
    validate_series("rsi", values, period)?;
    if values.is_empty() {
        return Ok(Vec::new());
    }

    let mut ups = Vec::with_capacity(values.len() - 1);
    let mut dns = Vec::with_capacity(values.len() - 1);
    for pair in values.windows(2) {
        let diff = pair[1] - pair[0];
        ups.push(diff.max(0.0));
        dns.push((-diff).max(0.0));
    }

    let alpha = 1.0 / (period as f64);
    let up_ewm = ewma_finite(&ups, alpha, period);
    let dn_ewm = ewma_finite(&dns, alpha, period);

    let mut out = Vec::with_capacity(values.len());
    out.push(None); // index 0 has NaN diff
    for (up_opt, dn_opt) in up_ewm.into_iter().zip(dn_ewm) {
        let value = match (up_opt, dn_opt) {
            (Some(up), Some(dn)) => {
                let rsi_val = 100.0 - 100.0 / (1.0 + up / dn);
                if rsi_val.is_finite() {
                    Some(rsi_val)
                } else {
                    None
                }
            }
            _ => None,
        };
        out.push(value);
    }

    Ok(out)
}

#[cfg(test)]
mod tests {
    use trading_engine_core::{Money, Timestamp};

    use super::*;

    fn candle(idx: i64, open: f64, high: f64, low: f64, close: f64) -> Candle {
        Candle::new(
            Timestamp::from_millis(1_700_000_000_000 + idx * 14_400_000).unwrap(),
            Money::new(open).unwrap(),
            Money::new(high).unwrap(),
            Money::new(low).unwrap(),
            Money::new(close).unwrap(),
            100.0,
        )
        .unwrap()
    }

    #[test]
    fn ema_matches_pandas_ewm_adjust_false() {
        let values = [10.0, 12.0, 14.0, 13.0, 15.0];
        let result = ema(&values, 3).unwrap();
        // alpha = 2 / (3 + 1) = 0.5
        // i=0: state = 10.0 (<3 -> None)
        // i=1: state = 0.5*10 + 0.5*12 = 11.0 (<3 -> None)
        // i=2: state = 0.5*11 + 0.5*14 = 12.5
        // i=3: state = 0.5*12.5 + 0.5*13 = 12.75
        // i=4: state = 0.5*12.75 + 0.5*15 = 13.875
        assert_eq!(
            result,
            vec![None, None, Some(12.5), Some(12.75), Some(13.875)]
        );
    }

    #[test]
    fn atr_matches_pandas_ewm_initialization() {
        let candles = vec![
            candle(0, 11.0, 12.0, 10.0, 11.0), // TR = 2.0
            candle(1, 12.0, 14.0, 11.0, 13.0), // TR = max(3, |14-11|, |11-11|) = 3.0
            candle(2, 12.0, 13.0, 9.0, 10.0),  // TR = max(4, |13-13|, |9-13|) = 4.0
            candle(3, 11.0, 15.0, 10.0, 14.0), // TR = max(5, |15-10|, |10-10|) = 5.0
        ];

        assert_eq!(true_range(&candles), vec![2.0, 3.0, 4.0, 5.0]);

        let result = atr(&candles, 3).unwrap();
        assert_eq!(result[0], None);
        assert_eq!(result[1], None);

        // EWM with alpha = 1/3, initialized at TR[0] = 2.0:
        // i=1: (2/3)*2 + (1/3)*3 = 7/3
        // i=2: (2/3)*(7/3) + (1/3)*4 = 26/9
        // i=3: (2/3)*(26/9) + (1/3)*5 = 97/27
        let expected_2 = (2.0 / 3.0) * ((2.0 / 3.0) * 2.0 + (1.0 / 3.0) * 3.0) + (1.0 / 3.0) * 4.0;
        let expected_3 = (2.0 / 3.0) * expected_2 + (1.0 / 3.0) * 5.0;
        assert!((result[2].unwrap() - expected_2).abs() < 1e-12);
        assert!((result[3].unwrap() - expected_3).abs() < 1e-12);
    }

    #[test]
    fn atr_differs_from_sma_seeded_wilder_atr() {
        // Verify explicitly that ATR does NOT use SMA seeding on the first n bars.
        // For TR = [2, 3, 4], SMA(3) = 3.0, whereas pandas ewm(alpha=1/3, adjust=False) = 26/9.
        let candles = vec![
            candle(0, 11.0, 12.0, 10.0, 11.0),
            candle(1, 12.0, 14.0, 11.0, 13.0),
            candle(2, 12.0, 13.0, 9.0, 10.0),
        ];
        let result = atr(&candles, 3).unwrap();
        assert!((result[2].unwrap() - 26.0 / 9.0).abs() < 1e-12);
        assert!((result[2].unwrap() - 3.0).abs() > 0.1);
    }

    #[test]
    fn rsi_matches_pandas_diff_and_ewm() {
        // closes: [10, 12, 11, 14, 14]
        // diffs (from i=1): [+2, -1, +3, 0]
        // ups: [2, 0, 3, 0]
        // dns: [0, 1, 0, 0]
        // period = 2, alpha = 0.5:
        // diff i=0 (close i=1): up=2.0, dn=0.0 -> None (min_periods=2)
        // diff i=1 (close i=2): up=0.5*2 + 0.5*0 = 1.0, dn=0.5*0 + 0.5*1 = 0.5 -> rs=2.0 -> 100 - 100/3
        // diff i=2 (close i=3): up=0.5*1 + 0.5*3 = 2.0, dn=0.5*0.5 + 0 = 0.25 -> rs=8.0 -> 100 - 100/9
        // diff i=3 (close i=4): up=1.0, dn=0.125 -> rs=8.0 -> 100 - 100/9
        let closes = [10.0, 12.0, 11.0, 14.0, 14.0];
        let result = rsi(&closes, 2).unwrap();
        assert_eq!(result[0], None);
        assert_eq!(result[1], None);
        assert!((result[2].unwrap() - (100.0 - 100.0 / 3.0)).abs() < 1e-12);
        assert!((result[3].unwrap() - (100.0 - 100.0 / 9.0)).abs() < 1e-12);
        assert!((result[4].unwrap() - (100.0 - 100.0 / 9.0)).abs() < 1e-12);
    }

    #[test]
    fn rsi_handles_all_up_and_flat_edge_cases() {
        let rising = [10.0, 11.0, 12.0, 13.0];
        let r = rsi(&rising, 2).unwrap();
        assert_eq!(r[2], Some(100.0));
        assert_eq!(r[3], Some(100.0));

        let flat = [10.0, 10.0, 10.0, 10.0];
        let f = rsi(&flat, 2).unwrap();
        assert_eq!(f, vec![None, None, None, None]);
    }

    #[test]
    fn rejects_zero_periods_and_non_finite_inputs() {
        assert_eq!(
            ema(&[1.0, 2.0], 0),
            Err(IndicatorError::InvalidPeriod {
                indicator: "ema",
                period: 0
            })
        );
        assert_eq!(
            atr(&[], 0),
            Err(IndicatorError::InvalidPeriod {
                indicator: "atr",
                period: 0
            })
        );
        assert_eq!(
            rsi(&[1.0, 2.0], 0),
            Err(IndicatorError::InvalidPeriod {
                indicator: "rsi",
                period: 0
            })
        );
        assert_eq!(
            ema(&[1.0, f64::NAN], 2),
            Err(IndicatorError::NonFiniteInput {
                indicator: "ema",
                index: 1
            })
        );
        assert_eq!(
            rsi(&[f64::INFINITY, 2.0], 2),
            Err(IndicatorError::NonFiniteInput {
                indicator: "rsi",
                index: 0
            })
        );
    }

    #[test]
    fn handles_empty_and_short_series() {
        assert_eq!(ema(&[], 14).unwrap(), Vec::<Option<f64>>::new());
        assert_eq!(atr(&[], 14).unwrap(), Vec::<Option<f64>>::new());
        assert_eq!(rsi(&[], 14).unwrap(), Vec::<Option<f64>>::new());

        assert_eq!(ema(&[1.0, 2.0], 5).unwrap(), vec![None, None]);
        assert_eq!(rsi(&[1.0, 2.0], 2).unwrap(), vec![None, None]);
    }
}
