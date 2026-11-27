use std::{env, fs};

use trading_engine_backtest::{metrics, run, BacktestConfig};
use trading_engine_core::{Candle, Money, Timestamp};
use trading_engine_indicators::{atr, donchian_channels, ema, rsi, true_range};
use trading_engine_strategies::{Donchian, EmaCross, RsiReversion, Strategy};

fn parse<T: std::str::FromStr>(value: Option<String>, name: &str) -> Result<T, String> {
    value
        .ok_or_else(|| format!("missing {name}"))?
        .parse()
        .map_err(|_| format!("invalid {name}"))
}

fn dump<S: Strategy>(
    candles: &[Candle],
    strategy: S,
    indicators: Vec<serde_json::Value>,
) -> Result<(), String> {
    let signals = strategy.signals(candles).map_err(|e| format!("{e:?}"))?;
    let config = BacktestConfig::default();
    let result = run(candles, &strategy, config).map_err(|e| format!("{e:?}"))?;
    let perf = metrics(&result, config.initial);
    let bars: Vec<_> = candles
        .iter()
        .enumerate()
        .map(|(i, candle)| {
            serde_json::json!({
                "timestamp": candle.open_time().as_millis(),
                "enter_long": signals[i].enter_long,
                "exit_long": signals[i].exit_long,
                "atr": signals[i].atr,
                "indicators": indicators[i],
            })
        })
        .collect();
    let equity: Vec<_> = result
        .equity_curve
        .iter()
        .map(|point| {
            serde_json::json!({
                "timestamp": point.timestamp,
                "equity": point.equity,
                "in_position": point.in_position,
                "notional_fraction": point.notional_fraction,
            })
        })
        .collect();
    let trades: Vec<_> = result
        .trades
        .iter()
        .map(|trade| {
            serde_json::json!({
                "entry_time": trade.entry_time,
                "exit_time": trade.exit_time,
                "entry": trade.entry,
                "exit": trade.exit,
                "quantity": trade.quantity,
                "entry_fee": trade.entry_fee,
                "exit_fee": trade.exit_fee,
                "gross_pnl": trade.quantity * (trade.exit - trade.entry),
                "pnl": trade.pnl,
                "reason": format!("{:?}", trade.reason).to_lowercase(),
            })
        })
        .collect();
    println!(
        "{}",
        serde_json::json!({
            "strategy": strategy.name(),
            "bars": bars,
            "equity": equity,
            "trades": trades,
            "metrics": {
                "initial_capital": perf.initial_capital,
                "final_equity": perf.final_equity,
                "total_return": perf.total_return,
                "cagr": perf.cagr,
                "trades": perf.trades,
                "win_rate": perf.win_rate,
                "profit_factor": perf.profit_factor,
                "max_drawdown": perf.max_drawdown,
                "sharpe": perf.sharpe,
                "average_winner": perf.average_winner,
                "average_loser": perf.average_loser,
                "total_fees": perf.total_fees,
                "daily_equity": perf.daily_equity.iter()
                    .map(|(date, value)| serde_json::json!([date.to_string(), value]))
                    .collect::<Vec<_>>(),
            }
        })
    );
    Ok(())
}

fn load_candles(path: &str) -> Result<Vec<Candle>, String> {
    let input = fs::read_to_string(path).map_err(|e| e.to_string())?;
    input
        .lines()
        .skip(1)
        .map(|line| -> Result<Candle, String> {
            let fields: Vec<_> = line.split(',').collect();
            if fields.len() < 6 {
                return Err(format!("invalid CSV row: {line}"));
            }
            Candle::new(
                Timestamp::parse_fixture(fields[0]).map_err(|e| e.to_string())?,
                Money::new(
                    fields[1]
                        .parse()
                        .map_err(|e: std::num::ParseFloatError| e.to_string())?,
                )
                .map_err(|e| e.to_string())?,
                Money::new(
                    fields[2]
                        .parse()
                        .map_err(|e: std::num::ParseFloatError| e.to_string())?,
                )
                .map_err(|e| e.to_string())?,
                Money::new(
                    fields[3]
                        .parse()
                        .map_err(|e: std::num::ParseFloatError| e.to_string())?,
                )
                .map_err(|e| e.to_string())?,
                Money::new(
                    fields[4]
                        .parse()
                        .map_err(|e: std::num::ParseFloatError| e.to_string())?,
                )
                .map_err(|e| e.to_string())?,
                fields[5]
                    .parse()
                    .map_err(|e: std::num::ParseFloatError| e.to_string())?,
            )
            .map_err(|e| e.to_string())
        })
        .collect()
}

fn main() -> Result<(), String> {
    let mut args = env::args().skip(1);
    let path = args.next().ok_or(
        "usage: parity_dump <fixture.csv> <donchian|ema_cross|rsi_mr> <strategy parameters>",
    )?;
    let kind = args.next().ok_or("missing strategy")?;
    let candles = load_candles(&path)?;
    let closes: Vec<f64> = candles.iter().map(|c| c.close().as_f64()).collect();
    let ranges = true_range(&candles);
    let atr14 = atr(&candles, 14).map_err(|e| format!("{e:?}"))?;
    let ema20 = ema(&closes, 20).map_err(|e| format!("{e:?}"))?;
    let rsi14 = rsi(&closes, 14).map_err(|e| format!("{e:?}"))?;

    match kind.as_str() {
        "donchian" => {
            let entry_n: usize = parse(args.next(), "entry_n")?;
            let exit_n: usize = parse(args.next(), "exit_n")?;
            let stop_mult: f64 = parse(args.next(), "stop_mult")?;
            let channels =
                donchian_channels(&candles, entry_n, exit_n).map_err(|e| format!("{e:?}"))?;
            let indicators = (0..candles.len())
                .map(|i| serde_json::json!({
                    "true_range": ranges[i], "atr14": atr14[i], "ema20": ema20[i], "rsi14": rsi14[i],
                    "entry_upper": channels[i].entry_upper, "entry_lower": channels[i].entry_lower,
                    "exit_lower": channels[i].exit_lower, "exit_upper": channels[i].exit_upper,
                }))
                .collect();
            dump(
                &candles,
                Donchian::new(entry_n, exit_n, stop_mult).map_err(|e| format!("{e:?}"))?,
                indicators,
            )
        }
        "ema_cross" => {
            let fast: usize = parse(args.next(), "fast")?;
            let slow: usize = parse(args.next(), "slow")?;
            let stop_mult: f64 = parse(args.next(), "stop_mult")?;
            let fast_ema = ema(&closes, fast).map_err(|e| format!("{e:?}"))?;
            let slow_ema = ema(&closes, slow).map_err(|e| format!("{e:?}"))?;
            let indicators = (0..candles.len())
                .map(|i| serde_json::json!({
                    "true_range": ranges[i], "atr14": atr14[i], "ema20": ema20[i], "rsi14": rsi14[i],
                    "ema_fast": fast_ema[i], "ema_slow": slow_ema[i],
                }))
                .collect();
            dump(
                &candles,
                EmaCross::new(fast, slow, stop_mult).map_err(|e| format!("{e:?}"))?,
                indicators,
            )
        }
        "rsi_mr" => {
            let rsi_n: usize = parse(args.next(), "rsi_n")?;
            let entry: f64 = parse(args.next(), "entry")?;
            let exit: f64 = parse(args.next(), "exit")?;
            let trend_n: usize = parse(args.next(), "trend_n")?;
            let stop_mult: f64 = parse(args.next(), "stop_mult")?;
            let rsi_n_values = rsi(&closes, rsi_n).map_err(|e| format!("{e:?}"))?;
            let trend_ema = ema(&closes, trend_n).map_err(|e| format!("{e:?}"))?;
            let indicators = (0..candles.len())
                .map(|i| serde_json::json!({
                    "true_range": ranges[i], "atr14": atr14[i], "ema20": ema20[i], "rsi14": rsi14[i],
                    "rsi_strategy": rsi_n_values[i], "ema_trend": trend_ema[i],
                }))
                .collect();
            dump(
                &candles,
                RsiReversion::new(rsi_n, entry, exit, trend_n, stop_mult)
                    .map_err(|e| format!("{e:?}"))?,
                indicators,
            )
        }
        _ => Err(format!("unknown strategy {kind:?}")),
    }
}
