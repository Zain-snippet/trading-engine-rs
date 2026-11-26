use trading_engine_core::{Candle, Money, Timestamp};
use trading_engine_indicators::{atr, donchian_channels, ema, rsi, true_range};

const FIXTURE: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../fixtures/ETHUSDT_4h_2023-09_2024-12.csv"
));

fn load_fixture_candles() -> Vec<Candle> {
    FIXTURE
        .lines()
        .skip(1)
        .map(|line| {
            let fields: Vec<&str> = line.split(',').collect();
            assert_eq!(fields.len(), 6);
            Candle::new(
                Timestamp::parse_fixture(fields[0]).unwrap(),
                Money::new(fields[1].parse().unwrap()).unwrap(),
                Money::new(fields[2].parse().unwrap()).unwrap(),
                Money::new(fields[3].parse().unwrap()).unwrap(),
                Money::new(fields[4].parse().unwrap()).unwrap(),
                fields[5].parse().unwrap(),
            )
            .unwrap()
        })
        .collect()
}

#[test]
fn indicators_on_eth_fixture_have_expected_warmup_and_causality() {
    let candles = load_fixture_candles();
    assert_eq!(candles.len(), 2_928);

    let closes: Vec<f64> = candles.iter().map(|c| c.close().as_f64()).collect();
    let atr_series = atr(&candles, 14).unwrap();
    let ema_series = ema(&closes, 20).unwrap();
    let rsi_series = rsi(&closes, 14).unwrap();
    let dc_series = donchian_channels(&candles, 55, 50).unwrap();

    // ATR(14) warmup: None for 0..13, Some positive finite from 13 onward.
    for (i, val) in atr_series.iter().enumerate() {
        if i < 13 {
            assert_eq!(*val, None, "atr[{i}] must be None during warmup");
        } else {
            let v = val.expect("atr must be Some after warmup");
            assert!(v.is_finite() && v > 0.0, "atr[{i}] = {v}");
        }
    }

    // EMA(20) warmup: None for 0..19, Some from 19 onward.
    for (i, val) in ema_series.iter().enumerate() {
        assert_eq!(val.is_some(), i >= 19, "ema[{i}]");
    }

    // RSI(14) warmup: diff() drops index 0, so first valid is at index 14.
    for (i, val) in rsi_series.iter().enumerate() {
        if i < 14 {
            assert_eq!(*val, None, "rsi[{i}] must be None during warmup");
        } else {
            let v = val.expect("rsi must be Some after warmup");
            assert!((0.0..=100.0).contains(&v), "rsi[{i}] = {v}");
        }
    }

    // Donchian(55, 50) matches naive exclusive window slices across the entire fixture.
    for (i, level) in dc_series.iter().enumerate() {
        if i < 55 {
            assert_eq!(level.entry_upper, None);
            assert_eq!(level.entry_lower, None);
        } else {
            let expected_upper = candles[i - 55..i]
                .iter()
                .map(|c| c.high().as_f64())
                .fold(f64::NEG_INFINITY, f64::max);
            let expected_lower = candles[i - 55..i]
                .iter()
                .map(|c| c.low().as_f64())
                .fold(f64::INFINITY, f64::min);
            assert_eq!(level.entry_upper, Some(expected_upper), "entry_upper[{i}]");
            assert_eq!(level.entry_lower, Some(expected_lower), "entry_lower[{i}]");
        }

        if i < 50 {
            assert_eq!(level.exit_lower, None);
            assert_eq!(level.exit_upper, None);
        } else {
            let expected_exit_lower = candles[i - 50..i]
                .iter()
                .map(|c| c.low().as_f64())
                .fold(f64::INFINITY, f64::min);
            let expected_exit_upper = candles[i - 50..i]
                .iter()
                .map(|c| c.high().as_f64())
                .fold(f64::NEG_INFINITY, f64::max);
            assert_eq!(
                level.exit_lower,
                Some(expected_exit_lower),
                "exit_lower[{i}]"
            );
            assert_eq!(
                level.exit_upper,
                Some(expected_exit_upper),
                "exit_upper[{i}]"
            );
        }
    }

    // Causality check: truncating history at bar i matches full history at bar i.
    for i in [55, 100, 300, 500, 1500, 2927] {
        let prefix = &candles[..=i];
        let p_atr = atr(prefix, 14).unwrap();
        let p_ema = ema(&closes[..=i], 20).unwrap();
        let p_rsi = rsi(&closes[..=i], 14).unwrap();
        let p_dc = donchian_channels(prefix, 55, 50).unwrap();

        assert_eq!(p_atr[i], atr_series[i]);
        assert_eq!(p_ema[i], ema_series[i]);
        assert_eq!(p_rsi[i], rsi_series[i]);
        assert_eq!(p_dc[i], dc_series[i]);
    }

    // Verify true_range matches bar 0 = high - low and bar i >= 1 max(H-L, |H-PC|, |L-PC|).
    let tr = true_range(&candles);
    assert_eq!(
        tr[0],
        candles[0].high().as_f64() - candles[0].low().as_f64()
    );
    let pc1 = candles[0].close().as_f64();
    let expected_tr1 = (candles[1].high().as_f64() - candles[1].low().as_f64())
        .max((candles[1].high().as_f64() - pc1).abs())
        .max((candles[1].low().as_f64() - pc1).abs());
    assert_eq!(tr[1], expected_tr1);
}
