use trading_engine_config::SymbolConfig;
use trading_engine_core::{Candle, Money, SymbolId, Timestamp};
use trading_engine_strategies::{Donchian, Strategy};

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
fn donchian_latest_signal_matches_full_history_without_lookahead() {
    let candles = load_fixture_candles();
    let sc = SymbolConfig::new(SymbolId::parse("ETH/USDT").unwrap(), 55, 50, 2.0);
    let strat = Donchian::from_symbol_config(&sc).unwrap();
    let full = strat.signals(&candles).unwrap();

    // Mirrors `tests/test_paperbot.py::test_signal_matches_backtest_without_lookahead`.
    let mut indices: Vec<usize> = (300..candles.len())
        .filter(|&i| full[i].enter_long || full[i].exit_long)
        .take(40)
        .collect();
    indices.extend(300..340);

    for i in indices {
        let eval = strat.latest_signal(&sc.symbol, &candles[..=i]).unwrap();
        assert_eq!(eval.signal.enter_long(), full[i].enter_long, "bar {i}");
        assert_eq!(eval.signal.exit_long(), full[i].exit_long, "bar {i}");
        assert!(
            (eval.signal.atr().as_f64() - full[i].atr.unwrap()).abs() <= 1e-12,
            "atr bar {i}"
        );

        let expected_entry_level = candles[i - 55..i]
            .iter()
            .map(|c| c.high().as_f64())
            .fold(f64::NEG_INFINITY, f64::max);
        let expected_exit_level = candles[i - 50..i]
            .iter()
            .map(|c| c.low().as_f64())
            .fold(f64::INFINITY, f64::min);

        assert_eq!(eval.entry_level, Some(expected_entry_level));
        assert_eq!(eval.exit_level, Some(expected_exit_level));
        assert_eq!(
            eval.signal.enter_long(),
            candles[i].close().as_f64() > expected_entry_level
        );
        assert_eq!(
            eval.signal.exit_long(),
            candles[i].close().as_f64() < expected_exit_level
        );
    }
}
