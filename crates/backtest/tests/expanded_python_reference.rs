use sha2::{Digest, Sha256};
use trading_engine_backtest::{metrics, run, BacktestConfig};
use trading_engine_core::{Candle, Money, Timestamp};
use trading_engine_strategies::{Donchian, EmaCross, RsiReversion, Strategy};

fn parse_candles(input: &str) -> Vec<Candle> {
    input
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
        .collect()
}

fn assert_near(actual: f64, expected: f64) {
    assert!(
        (actual - expected).abs() < 1e-9,
        "actual={actual:.15}, expected={expected:.15}"
    );
}

struct Golden {
    trades: usize,
    final_equity: f64,
    total_return: f64,
    cagr: f64,
    drawdown: f64,
    sharpe: f64,
    win_rate: f64,
    profit_factor: f64,
    total_fees: f64,
}

fn assert_golden<S: Strategy>(candles: &[Candle], strategy: &S, expected: Golden) {
    let result = run(candles, strategy, BacktestConfig::default()).unwrap();
    let performance = metrics(&result, 10_000.0);
    assert_eq!(performance.trades, expected.trades);
    assert_near(performance.final_equity, expected.final_equity);
    assert_near(performance.total_return, expected.total_return);
    assert_near(performance.cagr.unwrap(), expected.cagr);
    assert_near(performance.max_drawdown, expected.drawdown);
    assert_near(performance.sharpe, expected.sharpe);
    assert_near(performance.win_rate.unwrap(), expected.win_rate);
    assert_near(performance.profit_factor.unwrap(), expected.profit_factor);
    assert_near(performance.total_fees, expected.total_fees);
}

#[test]
fn btc_donchian_research_configuration_matches_python_golden() {
    const BTC_FIXTURE: &[u8] =
        include_bytes!("../../../fixtures/BTCUSDT_4h_2020-01_2026-11-25.csv");
    let hash = format!("{:x}", Sha256::digest(BTC_FIXTURE));
    assert_eq!(
        hash,
        "19d5d1fa75b83859cfd00160df13450888330eaba919041a1b063cc48857b898"
    );
    let candles = parse_candles(std::str::from_utf8(BTC_FIXTURE).unwrap());
    assert_eq!(candles.len(), 15_125);
    let strategy = Donchian::new(100, 20, 2.0).unwrap();
    let result = run(&candles, &strategy, BacktestConfig::default()).unwrap();
    let performance = metrics(&result, 10_000.0);
    assert_eq!(performance.trades, 83);
    assert_near(performance.final_equity, 28_448.33747610918);
    assert_near(performance.total_return, 1.844833747610918);
    assert_near(performance.cagr.unwrap(), 0.16356183064608687);
    assert_near(performance.max_drawdown, -0.1483045851127096);
    assert_near(performance.sharpe, 1.2515916674649485);
    assert_near(performance.win_rate.unwrap(), 0.3132530120481928);
    assert_near(performance.profit_factor.unwrap(), 2.2703828000032553);
    assert_near(performance.total_fees, 1_575.752044484218);

    let first = &result.trades[0];
    assert_eq!(first.entry_time, 1_579_406_400_000);
    assert_eq!(first.exit_time, 1_579_420_800_000);
    assert_near(first.quantity, 0.33943810234025673);
    assert_near(first.pnl, -107.62359530560752);
}

#[test]
fn eth_python_tested_grid_cases_match_golden_summaries() {
    let candles = parse_candles(include_str!(
        "../../../fixtures/ETHUSDT_4h_2023-09_2024-12.csv"
    ));

    let alternative_donchian = Donchian::new(100, 50, 3.0).unwrap();
    assert_golden(
        &candles,
        &alternative_donchian,
        Golden {
            trades: 11,
            final_equity: 12_313.414665053671,
            total_return: 0.23134146650536724,
            cagr: 0.16860593604383256,
            drawdown: -0.058822208828100186,
            sharpe: 1.6400602499883539,
            win_rate: 0.36363636363636365,
            profit_factor: 4.508560761661933,
            total_fees: 57.37223699122504,
        },
    );

    let ema = EmaCross::new(10, 50, 4.0).unwrap();
    assert_golden(
        &candles,
        &ema,
        Golden {
            trades: 41,
            final_equity: 11_679.487690306702,
            total_return: 0.16794876903067024,
            cagr: 0.12326302506963116,
            drawdown: -0.04745789040575321,
            sharpe: 1.7845701677132568,
            win_rate: 0.3170731707317073,
            profit_factor: 2.4066143531618684,
            total_fees: 146.7780396694969,
        },
    );

    let rsi = RsiReversion::new(14, 35.0, 65.0, 200, 3.0).unwrap();
    assert_golden(
        &candles,
        &rsi,
        Golden {
            trades: 12,
            final_equity: 10_059.634390638796,
            total_return: 0.005963439063879639,
            cagr: 0.004461602024855571,
            drawdown: -0.060499060287062645,
            sharpe: 0.14811948559076676,
            win_rate: 0.5,
            profit_factor: 1.092182318262279,
            total_fees: 45.20977372638099,
        },
    );
}
