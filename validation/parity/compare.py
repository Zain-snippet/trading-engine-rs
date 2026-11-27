"""Compare Rust and Python long-only Donchian backtests on the frozen fixture.

Usage:
  python validation/parity/compare.py --python-ref C:/path/to/binance-trading-bot

Requires Python 3.11+, numpy, pandas, and cargo. The reference checkout is only read.
"""
from __future__ import annotations

import argparse
import json
import subprocess
import sys
import types
from pathlib import Path

ABS_TOL = 1e-9
REL_TOL = 1e-10


def rust_dump(root: Path, fixture: Path) -> dict:
    proc = subprocess.run(
        ["cargo", "run", "--quiet", "--locked", "-p", "trading-engine-backtest", "--example", "parity_dump", "--", str(fixture)],
        cwd=root, check=True, capture_output=True, text=True,
    )
    return json.loads(proc.stdout)


def python_dump(reference: Path, fixture: Path) -> dict:
    import numpy as np
    import pandas as pd

    sys.path.insert(0, str(reference))
    from tradebot.strategies import Donchian
    from tradebot.indicators import atr, ema, rsi

    source = (reference / "tradebot" / "backtest.py").read_text(encoding="utf-8")
    original_append = 'trades.append((entry_t, d.index[i], "long" if side > 0 else "short", entry_px, fill, pnl, reason))'
    instrumented_append = 'trades.append((entry_t, d.index[i], "long" if side > 0 else "short", entry_px, fill, pnl, reason, abs(qty), entry_fee, f))'
    original_columns = 'columns=["entry_time", "exit_time", "side", "entry", "exit", "pnl", "reason"]'
    instrumented_columns = 'columns=["entry_time", "exit_time", "side", "entry", "exit", "pnl", "reason", "qty", "entry_fee", "exit_fee"]'
    if source.count(original_append) != 1 or source.count(original_columns) != 2:
        raise RuntimeError("reference backtest changed; update instrumentation explicitly")
    source = source.replace(original_append, instrumented_append).replace(original_columns, instrumented_columns)
    module = types.ModuleType("reference_backtest_instrumented")
    namespace = module.__dict__
    namespace.update({"np": np, "pd": pd})
    sys.modules[module.__name__] = module
    exec(compile(source, str(reference / "tradebot" / "backtest.py"), "exec"), namespace)

    df = pd.read_csv(fixture, parse_dates=["time"]).set_index("time")
    df.index = df.index.tz_localize(None)
    sig = Donchian(entry_n=55, exit_n=50, stop_mult=2.0).signals(df)
    result = namespace["run"](df, sig, 2.0, False, namespace["Config"]())
    perf = namespace["metrics"](result, initial=10_000.0)

    hi = df.high.rolling(55).max().shift()
    lo = df.low.rolling(55).min().shift()
    exit_lo = df.low.rolling(50).min().shift()
    exit_hi = df.high.rolling(50).max().shift()
    pc = df.close.shift()
    tr = pd.concat([df.high-df.low, (df.high-pc).abs(), (df.low-pc).abs()], axis=1).max(axis=1)
    atr_values, ema_values, rsi_values = atr(df), ema(df.close, 20), rsi(df.close, 14)
    bars = []
    for i, ts in enumerate(df.index):
        millis = int(ts.value // 1_000_000)
        def number(series):
            value = series.iloc[i]
            return None if pd.isna(value) else float(value)
        bars.append({
            "timestamp": millis, "true_range": float(tr.iloc[i]), "atr": number(atr_values),
            "donchian_entry_upper": number(hi), "donchian_entry_lower": number(lo),
            "donchian_exit_lower": number(exit_lo), "donchian_exit_upper": number(exit_hi),
            "ema20": number(ema_values), "rsi14": number(rsi_values),
            "enter_long": bool(sig.enter_long.iloc[i]), "exit_long": bool(sig.exit_long.iloc[i]),
        })
    trades = []
    for _, t in result.trades.iterrows():
        trades.append({
            "entry_time": int(t.entry_time.value // 1_000_000), "exit_time": int(t.exit_time.value // 1_000_000),
            "entry": float(t.entry), "exit": float(t.exit), "quantity": float(t.qty),
            "entry_fee": float(t.entry_fee), "exit_fee": float(t.exit_fee),
            "gross_pnl": float(t.qty * (t.exit-t.entry)), "pnl": float(t.pnl), "reason": str(t.reason),
        })
    equity = [{"timestamp": int(ts.value // 1_000_000), "equity": float(eq),
               "in_position": bool(result.in_pos.iloc[i]), "notional_fraction": float(result.notional_frac.iloc[i])}
              for i, (ts, eq) in enumerate(result.equity.items())]
    daily = result.equity.resample("1D").last().ffill()
    daily_equity = [[d.strftime("%Y-%m-%d"), float(v)] for d, v in daily.items()]
    return {"bars": bars, "trades": trades, "equity": equity, "total_fees": sum(t["entry_fee"]+t["exit_fee"] for t in trades),
            "metrics": {"initial_capital": 10_000.0, "final_equity": float(result.equity.iloc[-1]),
                "total_return": float(perf["total_return"]), "cagr": float(perf["cagr"]),
                "trades": int(perf["trades"]), "win_rate": float(perf["win_rate"]),
                "profit_factor": float(perf["profit_factor"]), "max_drawdown": float(perf["max_dd"]),
                "sharpe": float(perf["sharpe"]), "total_fees": sum(t["entry_fee"]+t["exit_fee"] for t in trades),
                "daily_equity": daily_equity}}


def compare_series(label, left, right, key, issues, stats):
    if len(left) != len(right):
        issues.append(f"{label}: length differs Rust={len(left)} Python={len(right)}")
        return
    max_abs = max_rel = 0.0
    first = None
    for i, (a, b) in enumerate(zip(left, right)):
        x, y = a[key], b[key]
        if isinstance(x, str) or isinstance(y, str) or isinstance(x, bool) or isinstance(y, bool):
            mismatch = x != y
            err = rel = 0.0
            if mismatch and first is None:
                first = {"index": i, "timestamp": a.get("timestamp", a.get("entry_time")), "rust": x, "python": y}
            continue
        if x is None or y is None:
            mismatch = x is not None or y is not None
            err = rel = 0.0
        else:
            err = abs(float(x)-float(y))
            rel = err / max(abs(float(y)), 1e-300)
            mismatch = err > ABS_TOL + REL_TOL * abs(float(y))
        max_abs, max_rel = max(max_abs, err), max(max_rel, rel)
        if mismatch and first is None:
            first = {"index": i, "timestamp": a.get("timestamp"), "rust": x, "python": y, "abs_error": err}
    stats[label] = {"max_absolute_error": max_abs, "max_relative_error": max_rel, "first_mismatch": first}
    if first:
        issues.append(f"{label}: first mismatch at {first['timestamp']}: Rust={first['rust']} Python={first['python']}")


def compare(root: Path, reference: Path, output: Path) -> int:
    fixture = root / "fixtures" / "ETHUSDT_4h_2023-09_2024-12.csv"
    rust = rust_dump(root, fixture)
    py = python_dump(reference, fixture)
    issues, stats = [], {}
    fields = ["true_range", "atr", "donchian_entry_upper", "donchian_entry_lower", "donchian_exit_lower", "donchian_exit_upper", "ema20", "rsi14"]
    for field in fields:
        compare_series(f"bar.{field}", rust["bars"], py["bars"], field, issues, stats)
    for field in ["timestamp", "enter_long", "exit_long"]:
        compare_series(f"bar.{field}", rust["bars"], py["bars"], field, issues, stats)
    if len(rust["trades"]) != len(py["trades"]):
        issues.append(f"trade count differs Rust={len(rust['trades'])} Python={len(py['trades'])}")
    for field in ["entry_time", "exit_time", "entry", "exit", "quantity", "entry_fee", "exit_fee", "gross_pnl", "pnl"]:
        compare_series(f"trade.{field}", rust["trades"], py["trades"], field, issues, stats)
    compare_series("trade.reason", rust["trades"], py["trades"], "reason", issues, stats)
    for field in ["timestamp", "equity", "in_position", "notional_fraction"]:
        compare_series(f"equity.{field}", rust["equity"], py["equity"], field, issues, stats)
    for field in ["initial_capital", "final_equity", "total_return", "cagr", "trades", "win_rate", "profit_factor", "max_drawdown", "sharpe", "total_fees"]:
        a, b = rust["metrics"][field], py["metrics"][field]
        if isinstance(a, (int,float)) and isinstance(b, (int,float)):
            err = abs(float(a)-float(b)); rel=err/max(abs(float(b)),1e-300)
            stats[f"metric.{field}"]={"rust":a,"python":b,"absolute_error":err,"relative_error":rel}
            if err > ABS_TOL + REL_TOL*abs(float(b)): issues.append(f"metric.{field} differs: Rust={a} Python={b}")
    compare_series("daily.date", rust["metrics"]["daily_equity"], py["metrics"]["daily_equity"], 0, issues, stats)
    compare_series("daily.equity", rust["metrics"]["daily_equity"], py["metrics"]["daily_equity"], 1, issues, stats)
    report = {"fixture": fixture.name, "bars": len(rust["bars"]), "trades": len(rust["trades"]),
              "tolerances": {"absolute": ABS_TOL, "relative": REL_TOL}, "comparisons": stats,
              "issues": issues, "rust_metrics": rust["metrics"], "python_metrics": py["metrics"],
              "trade_comparison": [{"trade": i, "rust": a, "python": py["trades"][i]} for i,a in enumerate(rust["trades"])]}
    output.write_text(json.dumps(report, indent=2, allow_nan=True), encoding="utf-8")
    print(json.dumps({"bars": report["bars"], "rust_trades": len(rust["trades"]), "python_trades": len(py["trades"]), "issues": issues, "report": str(output)}, indent=2))
    return 1 if issues else 0


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--python-ref", type=Path, required=True)
    parser.add_argument("--output", type=Path, default=Path("parity-report.json"))
    args = parser.parse_args()
    repo = Path(__file__).resolve().parents[2]
    raise SystemExit(compare(repo, args.python_ref.resolve(), args.output.resolve()))
