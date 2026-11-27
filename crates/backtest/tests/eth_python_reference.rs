use trading_engine_backtest::{metrics, run, BacktestConfig, ExitReason};
use trading_engine_core::{Candle, Money, Timestamp};
use trading_engine_strategies::Donchian;

#[test]
fn eth_fixture_matches_python_backtest_golden_summary_and_boundary_trades() {
    let candles: Vec<Candle> = include_str!("../../../fixtures/ETHUSDT_4h_2023-09_2024-12.csv")
        .lines()
        .skip(1)
        .map(|line| {
            let fields: Vec<_> = line.split(',').collect();
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
        .collect();
    let strategy = Donchian::new(55, 50, 2.0).unwrap();
    let result = run(&candles, &strategy, BacktestConfig::default()).unwrap();
    let summary = metrics(&result, 10_000.0);

    assert_eq!(candles.len(), 2_928);
    assert_eq!(result.trades.len(), 16);
    assert!((summary.final_equity - 13_943.096736739262).abs() < 1e-8);
    assert!((summary.total_return - 0.3943096736739262).abs() < 1e-12);
    assert!((summary.cagr.unwrap() - 0.2825800515439554).abs() < 1e-12);
    assert!((summary.max_drawdown - (-0.0831272393167658)).abs() < 1e-12);
    assert!((summary.sharpe - 1.66486849294924).abs() < 1e-12);
    assert!((summary.profit_factor.unwrap() - 4.055837985603487).abs() < 1e-12);
    assert_eq!(summary.win_rate, Some(0.3125));
    assert!((summary.total_fees - 133.51228545353374).abs() < 1e-10);

    let first = &result.trades[0];
    assert_eq!(first.entry_time, 1_695_038_400_000);
    assert_eq!(first.exit_time, 1_695_196_800_000);
    assert!((first.quantity - 3.285131986623404).abs() < 1e-12);
    assert_eq!(first.reason, ExitReason::Stop);

    let last = result.trades.last().unwrap();
    assert_eq!(last.entry_time, 1_730_952_000_000);
    assert_eq!(last.exit_time, 1_734_638_400_000);
    assert!((last.pnl - 574.3328474403149).abs() < 1e-10);
    assert_eq!(last.reason, ExitReason::Signal);
}
