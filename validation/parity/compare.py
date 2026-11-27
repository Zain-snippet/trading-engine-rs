"""Compare the Python and Rust long-only backtest grids on pinned fixtures.

Usage:
  python validation/parity/compare.py --python-ref /path/to/binance-trading-bot

Requires Python 3.11+, NumPy, pandas, and Cargo. The reference checkout is read-only.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import subprocess
import sys
import types
from pathlib import Path

ABS_TOL = 1e-9
REL_TOL = 1e-10
TRADE_FIELDS = [
    "entry_time", "exit_time", "entry", "exit", "quantity", "entry_fee",
    "exit_fee", "gross_pnl", "pnl", "reason",
]
METRIC_MAP = {
    "initial_capital": "initial_capital", "final_equity": "final_equity",
    "total_return": "total_return", "cagr": "cagr", "trades": "trades",
    "win_rate": "win_rate", "profit_factor": "profit_factor",
    "max_drawdown": "max_drawdown", "sharpe": "sharpe",
    "average_winner": "average_winner", "average_loser": "average_loser",
    "total_fees": "total_fees",
}


def rust_dump(root: Path, fixture: Path, strategy: str, params: tuple) -> dict:
    values = tuple(value for _, value in params)
    command = ["cargo", "run", "--quiet", "--locked", "-p", "trading-engine-backtest",
               "--example", "parity_dump", "--", str(fixture), strategy,
               *(format(value, ".15g") if isinstance(value, float) else str(value) for value in values)]
    proc = subprocess.run(command, cwd=root, check=False, capture_output=True, text=True)
    if proc.returncode:
        raise RuntimeError(f"Rust dump failed for {strategy} {params}: {proc.stderr}")
    return json.loads(proc.stdout)


def python_dump(reference: Path, fixture: Path, strategy_name: str, params: tuple) -> dict:
    import numpy as np
    import pandas as pd

    if str(reference) not in sys.path:
        sys.path.insert(0, str(reference))
    from tradebot.strategies import Donchian, EmaCross, RsiReversion
    from tradebot.indicators import atr, ema, rsi

    classes = {"donchian": Donchian, "ema_cross": EmaCross, "rsi_mr": RsiReversion}
    settings = dict(params)
    strategy = classes[strategy_name](**settings)
    df = pd.read_csv(fixture, parse_dates=["time"]).set_index("time")
    df.index = df.index.tz_localize(None)
    signals = strategy.signals(df)
    source_path = reference / "tradebot" / "backtest.py"
    source = source_path.read_text(encoding="utf-8")
    original_append = 'trades.append((entry_t, d.index[i], "long" if side > 0 else "short", entry_px, fill, pnl, reason))'
    instrumented_append = 'trades.append((entry_t, d.index[i], "long" if side > 0 else "short", entry_px, fill, pnl, reason, abs(qty), entry_fee, f))'
    original_columns = 'columns=["entry_time", "exit_time", "side", "entry", "exit", "pnl", "reason"]'
    instrumented_columns = 'columns=["entry_time", "exit_time", "side", "entry", "exit", "pnl", "reason", "qty", "entry_fee", "exit_fee"]'
    if source.count(original_append) != 1 or source.count(original_columns) != 2:
        raise RuntimeError("Python backtest source changed; review and update in-memory instrumentation")
    source = source.replace(original_append, instrumented_append).replace(original_columns, instrumented_columns)
    module = types.ModuleType("reference_backtest_instrumented")
    namespace = module.__dict__
    namespace.update({"np": np, "pd": pd})
    sys.modules[module.__name__] = module
    exec(compile(source, str(source_path), "exec"), namespace)
    result = namespace["run"](df, signals, strategy.stop_mult, strategy.trailing, namespace["Config"]())
    perf = namespace["metrics"](result, initial=10_000.0)

    highs, lows, closes = df.high, df.low, df.close
    tr = pd.concat([highs - lows, (highs - closes.shift()).abs(),
                    (lows - closes.shift()).abs()], axis=1).max(axis=1)
    atr14, ema20, rsi14 = atr(df), ema(closes, 20), rsi(closes, 14)
    indicator_series = {"true_range": tr, "atr14": atr14, "ema20": ema20, "rsi14": rsi14}
    if strategy_name == "donchian":
        entry_n, exit_n = settings["entry_n"], settings["exit_n"]
        indicator_series.update({
            "entry_upper": highs.rolling(entry_n).max().shift(),
            "entry_lower": lows.rolling(entry_n).min().shift(),
            "exit_lower": lows.rolling(exit_n).min().shift(),
            "exit_upper": highs.rolling(exit_n).max().shift(),
        })
    elif strategy_name == "ema_cross":
        fast, slow = settings["fast"], settings["slow"]
        indicator_series.update({"ema_fast": ema(closes, fast), "ema_slow": ema(closes, slow)})
    else:
        period, trend_n = settings["rsi_n"], settings["trend_n"]
        indicator_series.update({"rsi_strategy": rsi(closes, period), "ema_trend": ema(closes, trend_n)})

    def number(series, i):
        value = series.iloc[i]
        return None if pd.isna(value) else float(value)

    bars = []
    for i, ts in enumerate(df.index):
        bars.append({
            "timestamp": int(ts.value // 1_000_000),
            "enter_long": bool(signals.enter_long.iloc[i]),
            "exit_long": bool(signals.exit_long.iloc[i]),
            "atr": number(signals.atr, i),
            "indicators": {key: number(series, i) for key, series in indicator_series.items()},
        })
    trades = [{
        "entry_time": int(t.entry_time.value // 1_000_000),
        "exit_time": int(t.exit_time.value // 1_000_000),
        "entry": float(t.entry), "exit": float(t.exit), "quantity": float(t.qty),
        "entry_fee": float(t.entry_fee), "exit_fee": float(t.exit_fee),
        "gross_pnl": float(t.qty * (t.exit - t.entry)), "pnl": float(t.pnl),
        "reason": str(t.reason),
    } for _, t in result.trades.iterrows()]
    equity = [{
        "timestamp": int(ts.value // 1_000_000), "equity": float(value),
        "in_position": bool(result.in_pos.iloc[i]),
        "notional_fraction": float(result.notional_frac.iloc[i]),
    } for i, (ts, value) in enumerate(result.equity.items())]
    daily = result.equity.resample("1D").last().ffill()

    def metric(name):
        value = perf[name]
        return None if pd.isna(value) else float(value)

    winners = result.trades.loc[result.trades.pnl > 0, "pnl"]
    losers = result.trades.loc[result.trades.pnl <= 0, "pnl"]

    return {
        "bars": bars, "trades": trades, "equity": equity,
        "metrics": {
            "initial_capital": 10_000.0, "final_equity": float(result.equity.iloc[-1]),
            "total_return": metric("total_return"), "cagr": metric("cagr"),
            "trades": int(perf["trades"]), "win_rate": metric("win_rate"),
            "profit_factor": metric("profit_factor"), "max_drawdown": metric("max_dd"),
            "sharpe": metric("sharpe"),
            "average_winner": float(winners.mean()) if not winners.empty else None,
            "average_loser": float(-losers.mean()) if not losers.empty else None,
            "total_fees": sum(t["entry_fee"] + t["exit_fee"] for t in trades),
            "daily_equity": [[d.strftime("%Y-%m-%d"), float(v)] for d, v in daily.items()],
        },
    }


def numeric_equal(a, b):
    if a is None or b is None:
        return a is None and b is None, 0.0, 0.0
    error = abs(float(a) - float(b))
    relative = error / max(abs(float(b)), 1e-300)
    return error <= ABS_TOL + REL_TOL * abs(float(b)), error, relative


def compare_series(label, rust, python, field, nested_key, differences):
    if len(rust) != len(python):
        differences[label] = {"lengths": {"rust": len(rust), "python": len(python)}}
        return {"count": 1, "max_abs": None, "max_rel": None}
    checked = mismatches = 0
    max_abs = max_rel = 0.0
    first = None
    for i, (a, b) in enumerate(zip(rust, python)):
        left, right = (a[nested_key][field], b[nested_key][field]) if nested_key else (a[field], b[field])
        if isinstance(left, (str, bool)) or isinstance(right, (str, bool)):
            equal, error, relative = left == right, 0.0, 0.0
        else:
            equal, error, relative = numeric_equal(left, right)
        checked += 1
        max_abs, max_rel = max(max_abs, error), max(max_rel, relative)
        if not equal:
            mismatches += 1
            if first is None:
                first = {
                    "index": i,
                    "timestamp": a.get("timestamp", a.get("entry_time")),
                    "rust": left, "python": right, "absolute_error": error,
                }
    if mismatches:
        differences[label] = {"mismatch_count": mismatches, "first": first,
                              "max_absolute_error": max_abs, "max_relative_error": max_rel}
    return {"count": checked, "mismatches": mismatches, "max_abs": max_abs, "max_rel": max_rel}


def compare_case(root: Path, reference: Path, fixture: Path, strategy: str, params: tuple) -> dict:
    rust = rust_dump(root, fixture, strategy, params)
    python = python_dump(reference, fixture, strategy, params)
    differences: dict = {}
    stats: dict[str, dict] = {}
    for field in ("timestamp", "enter_long", "exit_long", "atr"):
        stats[f"bar.{field}"] = compare_series(f"bar.{field}", rust["bars"], python["bars"], field, None, differences)
    indicator_names = list(rust["bars"][0]["indicators"]) if rust["bars"] else []
    for field in indicator_names:
        stats[f"indicator.{field}"] = compare_series(
            f"indicator.{field}", rust["bars"], python["bars"], field, "indicators", differences
        )
    if len(rust["trades"]) != len(python["trades"]):
        differences["trade.count"] = {"rust": len(rust["trades"]), "python": len(python["trades"])}
    for field in TRADE_FIELDS:
        stats[f"trade.{field}"] = compare_series(
            f"trade.{field}", rust["trades"], python["trades"], field, None, differences
        )
    for point, trades in ((rust["equity"], rust["trades"]), (python["equity"], python["trades"])):
        terminal_liquidation = bool(
            trades and trades[-1]["reason"] == "end"
            and trades[-1]["exit_time"] == point[-1]["timestamp"]
        ) if point else False
        for index, row in enumerate(point):
            row["balance"] = (
                row["equity"] if terminal_liquidation and index == len(point) - 1
                else row["equity"] * (1.0 - row["notional_fraction"])
            )
    for field in ("timestamp", "equity", "balance", "in_position", "notional_fraction"):
        stats[f"equity.{field}"] = compare_series(
            f"equity.{field}", rust["equity"], python["equity"], field, None, differences
        )

    timestamps = [bar["timestamp"] for bar in rust["bars"]]
    previous_timestamp = {timestamps[index]: timestamps[index - 1] for index in range(1, len(timestamps))}
    for trade_list in (rust["trades"], python["trades"]):
        for trade in trade_list:
            trade["entry_signal_time"] = previous_timestamp.get(trade["entry_time"])
            trade["exit_signal_time"] = (
                previous_timestamp.get(trade["exit_time"]) if trade["reason"] == "signal" else None
            )
            trade["holding_ms"] = trade["exit_time"] - trade["entry_time"]
    for field in ("entry_signal_time", "exit_signal_time", "holding_ms"):
        stats[f"trade.{field}"] = compare_series(
            f"trade.{field}", rust["trades"], python["trades"], field, None, differences
        )
    for rust_name, python_name in METRIC_MAP.items():
        a, b = rust["metrics"][rust_name], python["metrics"][python_name]
        equal, error, relative = numeric_equal(a, b)
        stats[f"metric.{rust_name}"] = {"mismatches": int(not equal), "max_abs": error,
                                          "max_rel": relative, "rust": a, "python": b}
        if not equal:
            differences[f"metric.{rust_name}"] = {"rust": a, "python": b, "absolute_error": error}
    for index in range(max(len(rust["metrics"]["daily_equity"]), len(python["metrics"]["daily_equity"]))):
        if index >= len(rust["metrics"]["daily_equity"]) or index >= len(python["metrics"]["daily_equity"]):
            differences["daily_equity.length"] = {
                "rust": len(rust["metrics"]["daily_equity"]),
                "python": len(python["metrics"]["daily_equity"]),
            }
            break
        day_a, value_a = rust["metrics"]["daily_equity"][index]
        day_b, value_b = python["metrics"]["daily_equity"][index]
        if day_a != day_b:
            differences.setdefault("daily_equity.date", {"index": index, "rust": day_a, "python": day_b})
        equal, error, relative = numeric_equal(value_a, value_b)
        stats.setdefault("daily_equity", {"count": 0, "mismatches": 0, "max_abs": 0.0, "max_rel": 0.0})
        daily = stats["daily_equity"]
        daily["count"] += 1
        daily["max_abs"] = max(daily["max_abs"], error)
        daily["max_rel"] = max(daily["max_rel"], relative)
        if not equal:
            daily["mismatches"] += 1
            differences.setdefault("daily_equity.value", {
                "index": index, "date": day_a, "rust": value_a, "python": value_b,
                "absolute_error": error,
            })
    rust_metrics = {key: value for key, value in rust["metrics"].items() if key != "daily_equity"}
    python_metrics = {key: value for key, value in python["metrics"].items() if key != "daily_equity"}
    trade_samples = {}
    for sample_name, index in (("first", 0), ("last", -1)):
        if rust["trades"] and python["trades"]:
            trade_samples[sample_name] = {"rust": rust["trades"][index], "python": python["trades"][index]}
    return {
        "dataset": fixture.name,
        "strategy": strategy,
        "parameters": dict(params),
        "bars": len(rust["bars"]),
        "trades": len(rust["trades"]),
        "comparisons": stats,
        "differences": differences,
        "rust_metrics": rust_metrics,
        "python_metrics": python_metrics,
        "daily_equity": {
            "rows_compared": len(rust["metrics"]["daily_equity"]),
            "first": rust["metrics"]["daily_equity"][0] if rust["metrics"]["daily_equity"] else None,
            "last": rust["metrics"]["daily_equity"][-1] if rust["metrics"]["daily_equity"] else None,
            "comparisons": stats.get("daily_equity", {}),
        },
        "trade_samples": trade_samples,
    }


def parameter_grid(reference: Path):
    if str(reference) not in sys.path:
        sys.path.insert(0, str(reference))
    from tradebot.strategies import Donchian, EmaCross, RsiReversion
    cases = []
    for strat in Donchian.grid():
        cases.append(("donchian", ("entry_n", strat.entry_n), ("exit_n", strat.exit_n),
                      ("stop_mult", strat.stop_mult)))
    for strat in EmaCross.grid():
        cases.append(("ema_cross", ("fast", strat.fast), ("slow", strat.slow),
                      ("stop_mult", strat.stop_mult)))
    for strat in RsiReversion.grid():
        cases.append(("rsi_mr", ("rsi_n", strat.rsi_n), ("entry", strat.entry),
                      ("exit", strat.exit), ("trend_n", strat.trend_n), ("stop_mult", strat.stop_mult)))
    return cases


def run_all(root: Path, reference: Path, output: Path) -> int:
    eth = root / "fixtures" / "ETHUSDT_4h_2023-09_2024-12.csv"
    btc = root / "fixtures" / "BTCUSDT_4h_2020-01_2026-11-25.csv"
    verify_fixtures(eth, btc)
    cases = parameter_grid(reference)
    results = []
    for case in cases:
        strategy, *items = case
        params = tuple(items)
        results.append(compare_case(root, reference, eth, strategy, params))
    # BTC 4h's research-selected Donchian (100/20, 2 ATR) is included in its
    # exact Python grid; validate all 14 grid variants on this source dataset.
    for case in cases:
        if case[0] != "donchian":
            continue
        strategy, *items = case
        results.append(compare_case(root, reference, btc, strategy, tuple(items)))

    failures = sum(bool(result["differences"]) for result in results)
    summary = {
        "tolerances": {"absolute": ABS_TOL, "relative": REL_TOL},
        "case_count": len(results),
        "datasets": {
            eth.name: {"cases": len(cases), "bars_per_case": 2_928},
            btc.name: {"cases": sum(1 for case in cases if case[0] == "donchian"),
                       "bars_per_case": 15_125},
        },
        "scalar_comparisons": sum(
            stat.get("count", 0) for result in results for stat in result["comparisons"].values()
        ),
        "aggregate_metric_comparisons": len(results) * len(METRIC_MAP),
        "total_trade_records_compared": sum(result["trades"] for result in results),
        "cases_with_differences": failures,
        "results": results,
    }
    output.write_text(json.dumps(summary, indent=2, allow_nan=False), encoding="utf-8")
    print(json.dumps({key: value for key, value in summary.items() if key != "results"}, indent=2))
    return int(failures > 0)


def verify_fixtures(eth: Path, btc: Path) -> None:
    eth_digest = hashlib.sha256(eth.read_bytes()).hexdigest()
    if eth_digest != "017e927c8c681e7b2e812a5ed77cb4f2a0509d5b947f92bff247fd307f225c23":
        raise ValueError(f"ETH fixture checksum changed: {eth_digest}")
    provenance_path = btc.with_suffix(btc.suffix + ".provenance.json")
    provenance = json.loads(provenance_path.read_text(encoding="utf-8"))
    btc_digest = hashlib.sha256(btc.read_bytes()).hexdigest()
    if btc_digest != "19d5d1fa75b83859cfd00160df13450888330eaba919041a1b063cc48857b898":
        raise ValueError(f"BTC fixture checksum changed: {btc_digest}")
    if btc_digest != provenance["csv_sha256"]:
        raise ValueError(f"BTC fixture checksum differs from provenance: {btc_digest}")
    if provenance["symbol"] != "BTCUSDT" or provenance["interval"] != "4h":
        raise ValueError("BTC provenance symbol or interval differs")
    if provenance["row_count"] != 15_125 or provenance["gap_count"] != 1:
        raise ValueError("BTC fixture row count or documented source gap differs")


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--python-ref", type=Path, required=True)
    parser.add_argument("--output", type=Path, default=Path("parity-report-expanded.json"))
    args = parser.parse_args()
    repo = Path(__file__).resolve().parents[2]
    raise SystemExit(run_all(repo, args.python_ref.resolve(), args.output.resolve()))
