use std::collections::VecDeque;

use trading_engine_core::Candle;

use crate::error::IndicatorError;

/// Donchian channel levels for a single candle, shifted by 1 bar so the current
/// candle's high and low are strictly excluded.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DonchianLevel {
    /// Rolling max of `high[i - entry_n .. i]`.
    pub entry_upper: Option<f64>,
    /// Rolling min of `low[i - entry_n .. i]`.
    pub entry_lower: Option<f64>,
    /// Rolling min of `low[i - exit_n .. i]`.
    pub exit_lower: Option<f64>,
    /// Rolling max of `high[i - exit_n .. i]`.
    pub exit_upper: Option<f64>,
}

fn shifted_rolling_extrema(
    indicator: &'static str,
    values: &[f64],
    period: usize,
    replaces_back: fn(candidate: f64, existing: f64) -> bool,
) -> Result<Vec<Option<f64>>, IndicatorError> {
    if period == 0 {
        return Err(IndicatorError::InvalidPeriod { indicator, period });
    }
    for (index, value) in values.iter().enumerate() {
        if !value.is_finite() {
            return Err(IndicatorError::NonFiniteInput { indicator, index });
        }
    }

    let mut out = Vec::with_capacity(values.len());
    let mut deque: VecDeque<usize> = VecDeque::with_capacity(period.min(values.len()) + 1);

    for i in 0..values.len() {
        if i >= period {
            out.push(deque.front().map(|&idx| values[idx]));
        } else {
            out.push(None);
        }

        // Advance the unshifted window to cover `[i + 1 - period ..= i]` for index `i + 1`.
        while let Some(&front) = deque.front() {
            if front + period <= i {
                deque.pop_front();
            } else {
                break;
            }
        }
        let val = values[i];
        while let Some(&back) = deque.back() {
            if replaces_back(val, values[back]) {
                deque.pop_back();
            } else {
                break;
            }
        }
        deque.push_back(i);
    }

    Ok(out)
}

/// Shifted rolling maximum matching `series.rolling(period).max().shift()`.
///
/// At index `i`, returns `Some(max(values[i - period .. i]))` when `i >= period`,
/// strictly excluding `values[i]`.
pub fn shifted_rolling_max(
    values: &[f64],
    period: usize,
) -> Result<Vec<Option<f64>>, IndicatorError> {
    shifted_rolling_extrema("shifted_rolling_max", values, period, |cand, back| {
        cand >= back
    })
}

/// Shifted rolling minimum matching `series.rolling(period).min().shift()`.
///
/// At index `i`, returns `Some(min(values[i - period .. i]))` when `i >= period`,
/// strictly excluding `values[i]`.
pub fn shifted_rolling_min(
    values: &[f64],
    period: usize,
) -> Result<Vec<Option<f64>>, IndicatorError> {
    shifted_rolling_extrema("shifted_rolling_min", values, period, |cand, back| {
        cand <= back
    })
}

/// Computes Donchian entry and exit channel bounds matching `tradebot.strategies.Donchian`.
///
/// Every level excludes the current candle (`rolling(n).max/min().shift()`).
pub fn donchian_channels(
    candles: &[Candle],
    entry_n: usize,
    exit_n: usize,
) -> Result<Vec<DonchianLevel>, IndicatorError> {
    let highs: Vec<f64> = candles.iter().map(|c| c.high().as_f64()).collect();
    let lows: Vec<f64> = candles.iter().map(|c| c.low().as_f64()).collect();

    let entry_upper = shifted_rolling_max(&highs, entry_n)?;
    let entry_lower = shifted_rolling_min(&lows, entry_n)?;
    let exit_lower = shifted_rolling_min(&lows, exit_n)?;
    let exit_upper = shifted_rolling_max(&highs, exit_n)?;

    let levels = (0..candles.len())
        .map(|i| DonchianLevel {
            entry_upper: entry_upper[i],
            entry_lower: entry_lower[i],
            exit_lower: exit_lower[i],
            exit_upper: exit_upper[i],
        })
        .collect();

    Ok(levels)
}

#[cfg(test)]
mod tests {
    use trading_engine_core::{Money, Timestamp};

    use super::*;

    fn candle(idx: i64, high: f64, low: f64) -> Candle {
        let mid = (high + low) * 0.5;
        Candle::new(
            Timestamp::from_millis(1_700_000_000_000 + idx * 14_400_000).unwrap(),
            Money::new(mid).unwrap(),
            Money::new(high).unwrap(),
            Money::new(low).unwrap(),
            Money::new(mid).unwrap(),
            10.0,
        )
        .unwrap()
    }

    #[test]
    fn shifted_rolling_extrema_match_naive_windows_across_periods() {
        let values = [10.0, 12.0, 11.0, 15.0, 13.0, 9.0, 14.0, 14.0, 8.0, 16.0];
        for period in [1, 2, 3, 5, 10, 12] {
            let maxs = shifted_rolling_max(&values, period).unwrap();
            let mins = shifted_rolling_min(&values, period).unwrap();

            for i in 0..values.len() {
                if i < period {
                    assert_eq!(maxs[i], None, "period={period} i={i}");
                    assert_eq!(mins[i], None, "period={period} i={i}");
                } else {
                    let window = &values[i - period..i];
                    let expected_max = window.iter().copied().fold(f64::NEG_INFINITY, f64::max);
                    let expected_min = window.iter().copied().fold(f64::INFINITY, f64::min);
                    assert_eq!(maxs[i], Some(expected_max), "max period={period} i={i}");
                    assert_eq!(mins[i], Some(expected_min), "min period={period} i={i}");
                }
            }
        }
    }

    #[test]
    fn donchian_excludes_current_candle() {
        // Candle 2 has an extreme high (999.0), which must NOT affect candle 2's
        // entry_upper, and MUST appear starting on candle 3.
        let candles = vec![
            candle(0, 100.0, 90.0),
            candle(1, 105.0, 95.0),
            candle(2, 999.0, 50.0),
            candle(3, 110.0, 80.0),
        ];
        let levels = donchian_channels(&candles, 2, 1).unwrap();

        assert_eq!(levels[0].entry_upper, None);
        assert_eq!(levels[1].entry_upper, None);
        assert_eq!(levels[2].entry_upper, Some(105.0)); // max(100, 105), excludes 999
        assert_eq!(levels[2].entry_lower, Some(90.0)); // min(90, 95), excludes 50
        assert_eq!(levels[2].exit_lower, Some(95.0)); // min(95), excludes 50
        assert_eq!(levels[2].exit_upper, Some(105.0)); // max(105), excludes 999

        assert_eq!(levels[3].entry_upper, Some(999.0)); // max(105, 999)
        assert_eq!(levels[3].entry_lower, Some(50.0)); // min(95, 50)
        assert_eq!(levels[3].exit_lower, Some(50.0)); // min(50)
        assert_eq!(levels[3].exit_upper, Some(999.0)); // max(999)
    }

    #[test]
    fn rejects_zero_period_and_non_finite() {
        assert_eq!(
            shifted_rolling_max(&[1.0], 0),
            Err(IndicatorError::InvalidPeriod {
                indicator: "shifted_rolling_max",
                period: 0
            })
        );
        assert_eq!(
            shifted_rolling_min(&[1.0], 0),
            Err(IndicatorError::InvalidPeriod {
                indicator: "shifted_rolling_min",
                period: 0
            })
        );
        assert_eq!(
            shifted_rolling_max(&[1.0, f64::NAN], 1),
            Err(IndicatorError::NonFiniteInput {
                indicator: "shifted_rolling_max",
                index: 1
            })
        );
    }
}
