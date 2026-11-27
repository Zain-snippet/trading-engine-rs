use std::{env, fs};

use trading_engine_backtest::{metrics, run, BacktestConfig};
use trading_engine_core::{Candle, Money, Timestamp};
use trading_engine_indicators::{atr, donchian_channels, ema, rsi, true_range};
use trading_engine_strategies::{Donchian, Strategy};

fn main() -> Result<(), String> {
    let path = env::args()
        .nth(1)
        .ok_or("usage: parity_dump <fixture.csv>")?;
    let input = fs::read_to_string(path).map_err(|e| e.to_string())?;
    let candles: Vec<Candle> = input
        .lines()
        .skip(1)
        .map(|line| -> Result<Candle, String> {
            let f: Vec<_> = line.split(',').collect();
            Candle::new(
                Timestamp::parse_fixture(f[0]).map_err(|e| e.to_string())?,
                Money::new(
                    f[1].parse()
                        .map_err(|e: std::num::ParseFloatError| e.to_string())?,
                )
                .map_err(|e| e.to_string())?,
                Money::new(
                    f[2].parse()
                        .map_err(|e: std::num::ParseFloatError| e.to_string())?,
                )
                .map_err(|e| e.to_string())?,
                Money::new(
                    f[3].parse()
                        .map_err(|e: std::num::ParseFloatError| e.to_string())?,
                )
                .map_err(|e| e.to_string())?,
                Money::new(
                    f[4].parse()
                        .map_err(|e: std::num::ParseFloatError| e.to_string())?,
                )
                .map_err(|e| e.to_string())?,
                f[5].parse()
                    .map_err(|e: std::num::ParseFloatError| e.to_string())?,
            )
            .map_err(|e| e.to_string())
        })
        .collect::<Result<_, String>>()?;
    let closes: Vec<f64> = candles.iter().map(|c| c.close().as_f64()).collect();
    let channels = donchian_channels(&candles, 55, 50).map_err(|e| format!("{e:?}"))?;
    let atrs = atr(&candles, 14).map_err(|e| format!("{e:?}"))?;
    let emas = ema(&closes, 20).map_err(|e| format!("{e:?}"))?;
    let rsis = rsi(&closes, 14).map_err(|e| format!("{e:?}"))?;
    let tr = true_range(&candles);
    let strategy = Donchian::new(55, 50, 2.0).map_err(|e| format!("{e:?}"))?;
    let signals = strategy.signals(&candles).map_err(|e| format!("{e:?}"))?;
    let config = BacktestConfig::default();
    let result = run(&candles, &strategy, config).map_err(|e| format!("{e:?}"))?;
    let perf = metrics(&result, config.initial);
    let bars: Vec<_> = (0..candles.len()).map(|i| serde_json::json!({
        "timestamp": candles[i].open_time().as_millis(), "true_range": tr[i], "atr": atrs[i],
        "donchian_entry_upper": channels[i].entry_upper, "donchian_entry_lower": channels[i].entry_lower,
        "donchian_exit_lower": channels[i].exit_lower, "donchian_exit_upper": channels[i].exit_upper,
        "ema20": emas[i], "rsi14": rsis[i], "enter_long": signals[i].enter_long,
        "exit_long": signals[i].exit_long,
    })).collect();
    let equity: Vec<_> = result
        .equity_curve
        .iter()
        .map(|p| {
            serde_json::json!({
                "timestamp": p.timestamp, "equity": p.equity, "in_position": p.in_position,
                "notional_fraction": p.notional_fraction,
            })
        })
        .collect();
    let trades: Vec<_> = result
        .trades
        .iter()
        .map(|t| {
            serde_json::json!({
                "entry_time": t.entry_time, "exit_time": t.exit_time, "entry": t.entry,
                "exit": t.exit, "quantity": t.quantity, "entry_fee": t.entry_fee,
                "exit_fee": t.exit_fee, "gross_pnl": t.quantity * (t.exit - t.entry),
                "pnl": t.pnl, "reason": format!("{:?}", t.reason).to_lowercase(),
            })
        })
        .collect();
    println!(
        "{}",
        serde_json::json!({
            "bars": bars, "equity": equity, "trades": trades, "total_fees": result.total_fees,
            "metrics": {
                "initial_capital": perf.initial_capital, "final_equity": perf.final_equity,
                "total_return": perf.total_return, "cagr": perf.cagr, "trades": perf.trades,
                "win_rate": perf.win_rate, "profit_factor": perf.profit_factor,
                "max_drawdown": perf.max_drawdown, "sharpe": perf.sharpe,
                "average_winner": perf.average_winner, "average_loser": perf.average_loser,
                "total_fees": perf.total_fees,
                "daily_equity": perf.daily_equity.iter().map(|(d,v)| serde_json::json!([d.to_string(),v])).collect::<Vec<_>>(),
            }
        })
    );
    Ok(())
}
