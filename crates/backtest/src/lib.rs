//! Historical long-only backtesting. Signals use candle closes; orders execute
//! at the following candle's open. This is independent from ticker-price sim.

use chrono::{DateTime, NaiveDate, Utc};
use trading_engine_core::Candle;
use trading_engine_risk::size_backtest_position;
use trading_engine_strategies::{Strategy, StrategyBar, StrategyError};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BacktestConfig {
    pub initial: f64,
    pub fee: f64,
    pub slippage: f64,
    pub risk: f64,
    pub max_leverage: f64,
}

impl Default for BacktestConfig {
    fn default() -> Self {
        Self {
            initial: 10_000.0,
            fee: 0.001,
            slippage: 0.0005,
            risk: 0.01,
            max_leverage: 1.0,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Trade {
    pub entry_time: i64,
    pub exit_time: i64,
    pub entry: f64,
    pub exit: f64,
    pub quantity: f64,
    pub pnl: f64,
    pub entry_fee: f64,
    pub exit_fee: f64,
    pub reason: ExitReason,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExitReason {
    Signal,
    Stop,
    End,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EquityPoint {
    pub timestamp: i64,
    pub equity: f64,
    pub in_position: bool,
    pub notional_fraction: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BacktestResult {
    pub equity_curve: Vec<EquityPoint>,
    pub trades: Vec<Trade>,
    pub total_fees: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Metrics {
    pub initial_capital: f64,
    pub final_equity: f64,
    pub total_return: f64,
    pub cagr: Option<f64>,
    pub trades: usize,
    pub win_rate: Option<f64>,
    pub profit_factor: Option<f64>,
    pub max_drawdown: f64,
    pub sharpe: f64,
    pub average_winner: Option<f64>,
    pub average_loser: Option<f64>,
    pub total_fees: f64,
    pub daily_equity: Vec<(NaiveDate, f64)>,
}

#[derive(Debug)]
pub enum BacktestError {
    Strategy(StrategyError),
    InvalidConfig(&'static str),
    InvalidCandleOrder,
}
impl From<StrategyError> for BacktestError {
    fn from(value: StrategyError) -> Self {
        Self::Strategy(value)
    }
}

#[derive(Clone, Copy)]
struct Position {
    qty: f64,
    entry: f64,
    entry_fee: f64,
    stop: f64,
    extreme: f64,
    entry_time: i64,
}

/// Runs one symbol. Match the Python reference contract: calculate signals on
/// full history, schedule at close, fill at next open, stop intrabar, mark at
/// close, update trailing stops after marking, then schedule the next signal.
pub fn run<S: Strategy>(
    candles: &[Candle],
    strategy: &S,
    cfg: BacktestConfig,
) -> Result<BacktestResult, BacktestError> {
    validate_config(cfg)?;
    if candles
        .windows(2)
        .any(|w| w[0].open_time() >= w[1].open_time())
    {
        return Err(BacktestError::InvalidCandleOrder);
    }
    let signals = strategy.signals(candles)?;
    run_with_signals(
        candles,
        &signals,
        strategy.stop_mult(),
        strategy.trailing(),
        cfg,
    )
}

fn validate_config(cfg: BacktestConfig) -> Result<(), BacktestError> {
    if !cfg.initial.is_finite() || cfg.initial <= 0.0 {
        return Err(BacktestError::InvalidConfig(
            "initial must be positive and finite",
        ));
    }
    if !cfg.fee.is_finite() || !(0.0..1.0).contains(&cfg.fee) {
        return Err(BacktestError::InvalidConfig("fee must be in [0,1)"));
    }
    if !cfg.slippage.is_finite() || !(0.0..1.0).contains(&cfg.slippage) {
        return Err(BacktestError::InvalidConfig("slippage must be in [0,1)"));
    }
    if !cfg.risk.is_finite() || cfg.risk <= 0.0 || cfg.risk > 1.0 {
        return Err(BacktestError::InvalidConfig("risk must be in (0,1]"));
    }
    if !cfg.max_leverage.is_finite() || cfg.max_leverage <= 0.0 {
        return Err(BacktestError::InvalidConfig(
            "max_leverage must be positive and finite",
        ));
    }
    Ok(())
}

fn run_with_signals(
    c: &[Candle],
    sig: &[StrategyBar],
    stop_mult: f64,
    trailing: bool,
    cfg: BacktestConfig,
) -> Result<BacktestResult, BacktestError> {
    if c.is_empty() {
        return Ok(BacktestResult {
            equity_curve: vec![],
            trades: vec![],
            total_fees: 0.0,
        });
    }
    if c.len() != sig.len() {
        return Err(BacktestError::InvalidConfig(
            "signal count must match candle count",
        ));
    }
    let (mut cash, mut pos, mut pending, mut pending_atr) =
        (cfg.initial, None::<Position>, 0u8, 0.0f64);
    let (mut equity_curve, mut trades, mut fees) = (Vec::with_capacity(c.len()), Vec::new(), 0.0);
    for i in 0..c.len() {
        let candle = &c[i];
        if pending == 2 {
            if let Some(p) = pos.take() {
                close(
                    &mut cash,
                    &mut fees,
                    &mut trades,
                    p,
                    (
                        candle.open().as_f64(),
                        candle.open_time().as_millis(),
                        ExitReason::Signal,
                    ),
                    cfg,
                );
            }
        } else if pending == 1 && pos.is_none() {
            let fill = candle.open().as_f64() * (1.0 + cfg.slippage);
            let stop = fill - stop_mult * pending_atr;
            let dist = (fill - stop).abs();
            let qty = size_backtest_position(cash, dist, fill, cfg.risk, cfg.max_leverage, cfg.fee)
                .map_err(|_| BacktestError::InvalidConfig("invalid inputs to position sizing"))?;
            if qty > 0.0 && stop > 0.0 {
                let entry_fee = qty * fill * cfg.fee;
                cash -= qty * fill + entry_fee;
                fees += entry_fee;
                pos = Some(Position {
                    qty,
                    entry: fill,
                    entry_fee,
                    stop,
                    extreme: candle.high().as_f64(),
                    entry_time: candle.open_time().as_millis(),
                });
            }
        }
        pending = 0;
        if let Some(p) = pos {
            if candle.low().as_f64() <= p.stop {
                let raw = candle.open().as_f64().min(p.stop);
                pos = None;
                close(
                    &mut cash,
                    &mut fees,
                    &mut trades,
                    p,
                    (raw, candle.open_time().as_millis(), ExitReason::Stop),
                    cfg,
                );
            }
        }
        let mark_equity = cash + pos.map_or(0.0, |p| p.qty * candle.close().as_f64());
        let notional_fraction = pos.map_or(0.0, |p| p.qty * candle.close().as_f64() / mark_equity);
        equity_curve.push(EquityPoint {
            timestamp: candle.open_time().as_millis(),
            equity: mark_equity,
            in_position: pos.is_some(),
            notional_fraction,
        });
        if let Some(mut p) = pos {
            if trailing && sig[i].atr.is_some() {
                p.extreme = p.extreme.max(candle.high().as_f64());
                p.stop = p.stop.max(p.extreme - stop_mult * sig[i].atr.unwrap());
                pos = Some(p);
            }
        }
        if i + 1 == c.len() || sig[i].atr.is_none() {
            continue;
        }
        if pos.is_some() && sig[i].exit_long {
            pending = 2;
        } else if pos.is_none() && sig[i].enter_long {
            pending = 1;
            pending_atr = sig[i].atr.unwrap();
        }
    }
    if let Some(p) = pos {
        let last = c.last().expect("nonempty");
        close(
            &mut cash,
            &mut fees,
            &mut trades,
            p,
            (
                last.close().as_f64(),
                last.open_time().as_millis(),
                ExitReason::End,
            ),
            cfg,
        );
        if let Some(point) = equity_curve.last_mut() {
            point.equity = cash;
        }
    }
    Ok(BacktestResult {
        equity_curve,
        trades,
        total_fees: fees,
    })
}

fn close(
    cash: &mut f64,
    fees: &mut f64,
    trades: &mut Vec<Trade>,
    p: Position,
    execution: (f64, i64, ExitReason),
    cfg: BacktestConfig,
) {
    let (raw, time, reason) = execution;
    let fill = raw * (1.0 - cfg.slippage);
    let exit_fee = p.qty * fill * cfg.fee;
    *cash += p.qty * fill - exit_fee;
    *fees += exit_fee;
    let pnl = p.qty * (fill - p.entry) - p.entry_fee - exit_fee;
    trades.push(Trade {
        entry_time: p.entry_time,
        exit_time: time,
        entry: p.entry,
        exit: fill,
        quantity: p.qty,
        pnl,
        entry_fee: p.entry_fee,
        exit_fee,
        reason,
    });
}

fn date(ts: i64) -> NaiveDate {
    DateTime::<Utc>::from_timestamp_millis(ts)
        .expect("candle timestamps are range checked")
        .date_naive()
}

/// Computes metrics using explicit initial capital. Daily returns prepend that
/// initial value one day before the first daily observation, matching Python.
pub fn metrics(result: &BacktestResult, initial: f64) -> Metrics {
    let final_eq = result.equity_curve.last().map_or(initial, |p| p.equity);
    let total_return = final_eq / initial - 1.0;
    let years = match (result.equity_curve.first(), result.equity_curve.last()) {
        (Some(a), Some(b)) => (b.timestamp - a.timestamp) as f64 / (365.25 * 86_400_000.0),
        _ => 0.0,
    };
    let cagr =
        (years > 0.0 && final_eq > 0.0).then(|| (final_eq / initial).powf(1.0 / years) - 1.0);
    let mut peak = f64::NEG_INFINITY;
    let mut max_dd: f64 = 0.0;
    for p in &result.equity_curve {
        peak = peak.max(p.equity);
        if peak > 0.0 {
            max_dd = max_dd.min(p.equity / peak - 1.0);
        }
    }
    let mut daily = std::collections::BTreeMap::new();
    for p in &result.equity_curve {
        daily.insert(date(p.timestamp), p.equity);
    }
    let mut returns = Vec::new();
    let mut daily_equity = Vec::new();
    let mut previous = initial;
    if let (Some((&first_day, _)), Some((&last_day, _))) =
        (daily.first_key_value(), daily.last_key_value())
    {
        let mut day = first_day;
        while day <= last_day {
            let current = daily.get(&day).copied().unwrap_or(previous);
            returns.push(current / previous - 1.0);
            previous = current;
            daily_equity.push((day, current));
            day = day.succ_opt().expect("bounded date range");
        }
    }
    let avg = if returns.is_empty() {
        0.0
    } else {
        returns.iter().sum::<f64>() / returns.len() as f64
    };
    let sd = if returns.len() > 1 {
        (returns.iter().map(|r| (r - avg).powi(2)).sum::<f64>() / (returns.len() - 1) as f64).sqrt()
    } else {
        0.0
    };
    let wins: Vec<f64> = result
        .trades
        .iter()
        .filter(|t| t.pnl > 0.0)
        .map(|t| t.pnl)
        .collect();
    let losses: Vec<f64> = result
        .trades
        .iter()
        .filter(|t| t.pnl <= 0.0)
        .map(|t| -t.pnl)
        .collect();
    let win_sum: f64 = wins.iter().sum();
    let loss_sum: f64 = losses.iter().sum();
    Metrics {
        initial_capital: initial,
        final_equity: final_eq,
        total_return,
        cagr,
        trades: result.trades.len(),
        win_rate: (!result.trades.is_empty())
            .then(|| wins.len() as f64 / result.trades.len() as f64),
        profit_factor: (loss_sum > 0.0).then_some(win_sum / loss_sum),
        max_drawdown: max_dd,
        sharpe: if sd > 0.0 {
            avg / sd * 365f64.sqrt()
        } else {
            0.0
        },
        average_winner: (!wins.is_empty()).then(|| win_sum / wins.len() as f64),
        average_loser: (!losses.is_empty()).then(|| loss_sum / losses.len() as f64),
        total_fees: result.total_fees,
        daily_equity,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use trading_engine_core::{Money, Timestamp};
    struct Script {
        bars: Vec<StrategyBar>,
        stop: f64,
        trail: bool,
    }
    impl Strategy for Script {
        fn name(&self) -> &'static str {
            "script"
        }
        fn stop_mult(&self) -> f64 {
            self.stop
        }
        fn trailing(&self) -> bool {
            self.trail
        }
        fn signals(&self, c: &[Candle]) -> Result<Vec<StrategyBar>, StrategyError> {
            Ok((0..c.len())
                .map(|i| {
                    self.bars.get(i).copied().unwrap_or(StrategyBar {
                        enter_long: false,
                        exit_long: false,
                        atr: Some(2.0),
                    })
                })
                .collect())
        }
    }
    fn candle(i: i64, o: f64, h: f64, l: f64, c: f64) -> Candle {
        Candle::new(
            Timestamp::from_millis(1_700_000_000_000 + i * 86_400_000).unwrap(),
            Money::new(o).unwrap(),
            Money::new(h).unwrap(),
            Money::new(l).unwrap(),
            Money::new(c).unwrap(),
            1.0,
        )
        .unwrap()
    }
    fn cfg() -> BacktestConfig {
        BacktestConfig {
            initial: 1000.0,
            fee: 0.01,
            slippage: 0.01,
            risk: 0.1,
            max_leverage: 1.0,
        }
    }
    #[test]
    fn entry_is_next_open_with_adverse_slippage_and_fee() {
        let c = vec![
            candle(0, 10.0, 11.0, 9.0, 10.0),
            candle(1, 20.0, 22.0, 19.0, 21.0),
            candle(2, 21.0, 22.0, 20.0, 21.0),
        ];
        let s = Script {
            bars: vec![
                StrategyBar {
                    enter_long: true,
                    exit_long: false,
                    atr: Some(2.0)
                };
                3
            ],
            stop: 2.0,
            trail: false,
        };
        let r = run(&c, &s, cfg()).unwrap();
        assert_eq!(r, run(&c, &s, cfg()).unwrap());
        assert_eq!(r.trades[0].entry_time, c[1].open_time().as_millis());
        assert!((r.trades[0].entry - 20.2).abs() < 1e-12);
        assert!(r.trades[0].entry_fee > 0.0);
    }

    #[test]
    fn spot_notional_cap_keeps_purchase_within_cash_including_fee() {
        let c = vec![
            candle(0, 10.0, 11.0, 9.0, 10.0),
            candle(1, 20.0, 22.0, 19.0, 21.0),
            candle(2, 21.0, 22.0, 20.0, 21.0),
        ];
        let s = Script {
            bars: vec![
                StrategyBar {
                    enter_long: true,
                    exit_long: false,
                    atr: Some(0.01)
                };
                3
            ],
            stop: 2.0,
            trail: false,
        };
        let mut config = cfg();
        config.risk = 1.0;
        let r = run(&c, &s, config).unwrap();
        let trade = &r.trades[0];
        assert!(trade.quantity * trade.entry * (1.0 + config.fee) <= config.initial + 1e-9);
    }
    #[test]
    fn stop_gaps_fill_at_worse_open_plus_slippage_and_finalizes() {
        let c = vec![
            candle(0, 10.0, 11.0, 9.0, 10.0),
            candle(1, 10.0, 10.5, 9.5, 10.0),
            candle(2, 5.0, 6.0, 4.0, 5.0),
        ];
        let mut bars = vec![
            StrategyBar {
                enter_long: true,
                exit_long: false,
                atr: Some(2.0)
            };
            3
        ];
        bars[1].atr = Some(2.0);
        let s = Script {
            bars,
            stop: 2.0,
            trail: false,
        };
        let r = run(&c, &s, cfg()).unwrap();
        assert_eq!(r.trades.len(), 1);
        assert_eq!(r.trades[0].reason, ExitReason::Stop);
        assert!(r.trades[0].exit < 5.0);
    }
    #[test]
    fn signal_exit_is_next_open_and_metrics_use_custom_initial() {
        let c = vec![
            candle(0, 10.0, 11.0, 9.0, 10.0),
            candle(1, 10.0, 11.0, 9.0, 10.0),
            candle(2, 12.0, 13.0, 11.0, 12.0),
            candle(3, 12.0, 13.0, 11.0, 12.0),
        ];
        let mut bars = vec![
            StrategyBar {
                enter_long: false,
                exit_long: false,
                atr: Some(2.0)
            };
            4
        ];
        bars[0].enter_long = true;
        bars[2].exit_long = true;
        let s = Script {
            bars,
            stop: 2.0,
            trail: false,
        };
        let r = run(&c, &s, cfg()).unwrap();
        assert_eq!(r.trades[0].exit_time, c[3].open_time().as_millis());
        let m = metrics(&r, 1000.0);
        assert_eq!(m.initial_capital, 1000.0);
        assert_eq!(m.trades, 1);
        assert!(m.total_fees > 0.0);
    }
    #[test]
    fn empty_and_no_trade_histories_are_defined() {
        let s = Script {
            bars: vec![],
            stop: 2.0,
            trail: false,
        };
        let r = run(&[], &s, cfg()).unwrap();
        assert_eq!(metrics(&r, 1000.0).final_equity, 1000.0);
        let c = vec![candle(0, 10.0, 11.0, 9.0, 10.0)];
        let r = run(&c, &s, cfg()).unwrap();
        let m = metrics(&r, 1000.0);
        assert_eq!(m.trades, 0);
        assert_eq!(m.total_return, 0.0);
        assert_eq!(m.win_rate, None);
    }

    #[test]
    fn performance_metrics_cover_drawdown_win_rate_profit_factor_and_daily_equity() {
        let base = 1_700_000_000_000_i64;
        let result = BacktestResult {
            equity_curve: vec![
                EquityPoint {
                    timestamp: base,
                    equity: 100.0,
                    in_position: false,
                    notional_fraction: 0.0,
                },
                EquityPoint {
                    timestamp: base + 86_400_000,
                    equity: 120.0,
                    in_position: true,
                    notional_fraction: 0.5,
                },
                EquityPoint {
                    timestamp: base + 2 * 86_400_000,
                    equity: 90.0,
                    in_position: true,
                    notional_fraction: 0.5,
                },
                EquityPoint {
                    timestamp: base + 3 * 86_400_000,
                    equity: 110.0,
                    in_position: false,
                    notional_fraction: 0.0,
                },
            ],
            trades: vec![
                Trade {
                    entry_time: base,
                    exit_time: base + 1,
                    entry: 1.0,
                    exit: 2.0,
                    quantity: 1.0,
                    pnl: 20.0,
                    entry_fee: 0.0,
                    exit_fee: 0.0,
                    reason: ExitReason::Signal,
                },
                Trade {
                    entry_time: base + 2,
                    exit_time: base + 3,
                    entry: 2.0,
                    exit: 1.0,
                    quantity: 1.0,
                    pnl: -10.0,
                    entry_fee: 0.0,
                    exit_fee: 0.0,
                    reason: ExitReason::Stop,
                },
            ],
            total_fees: 3.0,
        };
        let m = metrics(&result, 100.0);
        assert!((m.total_return - 0.1).abs() < 1e-12);
        assert_eq!(m.max_drawdown, -0.25);
        assert_eq!(m.win_rate, Some(0.5));
        assert_eq!(m.profit_factor, Some(2.0));
        assert_eq!(m.average_winner, Some(20.0));
        assert_eq!(m.average_loser, Some(10.0));
        assert_eq!(m.total_fees, 3.0);
        assert_eq!(m.daily_equity.len(), 4);
    }
}
