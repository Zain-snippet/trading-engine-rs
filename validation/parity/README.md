# Python ↔ Rust backtest validation

## Verified scope

The Python reference source was read from a separate checkout and was not modified. Its `data/` directory documents that its parquet files are not committed. Therefore the BTC fixture below is a source-verified reconstruction from Binance's public Spot kline archive and REST API using the same monthly-plus-current-month collection approach as the original `tradebot/data.py`; it cannot be asserted byte-identical to the unavailable original parquet.

| Dataset | Python-tested configurations compared | Bars per run | Trade records compared |
|---|---:|---:|---:|
| ETHUSDT 4h, 2023-09 through 2024-12 | 14 Donchian + 24 EMA crossover + 24 RSI mean reversion | 2,928 | 1,151 |
| BTCUSDT 4h, 2020-01-01 through 2026-11-25 20:00 UTC | All 14 Python Donchian grid combinations | 15,125 | 1,517 |
| **Total** | **76 cases** | — | **2,668** |

The Python research script's BTC 4h selected Donchian configuration is entry/exit channels `100/20`, stop `2 ATR`. On the complete BTC reconstruction and starting capital 10,000, that case produced 83 trades, final equity 28,448.33747610918, total return 184.483374761%, CAGR 16.356183%, max drawdown −14.830459%, Sharpe 1.251592, win rate 31.325301%, profit factor 2.270383, and fees 1,575.752044. These are full-history validation outputs, not the research CSV's OOS-only performance.

The existing ETH `55/50`, `2 ATR` Donchian golden remains covered by `eth_python_reference.rs`. Additional Python-derived Rust goldens cover BTC `100/20/2`, ETH Donchian `100/50/3`, ETH EMA crossover `10/50/4`, and ETH RSI mean reversion `14/35/65/200/3` (RSI period / entry / exit / trend EMA period / stop multiplier).

## Python parameter grids exercised

The harness enumerates the original strategy `grid()` methods and validates long-only mode with the original default backtest configuration (10,000 initial capital, 0.1% fee per side, 0.05% adverse slippage per fill, 1% risk, 1× spot notional cap):

- Donchian, 14 cases: entry periods `[20, 55, 100]`, exit periods `[10, 20, 50]` where exit is less than entry, and stop multipliers `[2, 3]`.
- EMA crossover, 24 cases: fast periods `[10, 20, 50]`, slow periods `[50, 100, 200]` where fast is less than slow, and stop multipliers `[2, 3, 4]`. The Python strategy uses a trailing stop.
- RSI mean reversion, 24 cases: RSI periods `[7, 14]`, entry thresholds `[25, 30, 35]`, exit thresholds `[50, 65]`, trend EMA period `200`, and stop multipliers `[2, 3]`.

BTC validation uses all 14 Donchian grid cases. ETH validation uses all 62 cases across these three Python-equivalent strategies. Rust-only strategies, if added later, must remain labeled unvalidated until an equivalent reference exists.

## Compared behavior and result

`compare.py` uses the same candles, strategy signal implementation, default Python `Config`, and Rust `BacktestConfig`. It compares at every candle:

- True range, ATR, Donchian levels, EMA and RSI values, including warm-up `None`/NaN boundaries.
- Long entry and exit signals and their timestamps.
- Per-candle equity, in-position state, notional fraction, and cash balance derived from equity and marked position value.
- Daily equity and dates.
- Every trade's entry/exit signal time, entry/exit execution time, fill prices, quantity, fees, reason, gross/net PnL, and holding duration.
- Initial/final capital, return, CAGR, drawdown, Sharpe, win rate, profit factor, average win/loss, and total fees.

Absolute tolerance is `1e-9`; relative tolerance is `1e-10`. These were not loosened. Across 76 cases the harness compared more than 6.5 million indexed values, 912 aggregate metric values, and 2,668 trades; it reported zero mismatches. Maximum observed absolute difference was `1.3642420526593924e-12`; maximum observed relative difference was `2.986902089396384e-14`. Indicator warmups and boolean/string fields matched exactly. No discrepancy required a trading-logic fix.

Entry signal time and signal-based exit signal time are reported as the immediately preceding candle's open time, because the Python and Rust backtest models schedule close signals for the next candle open. Stop and final-close exits have no signal-exit timestamp. Cash balance is derived as equity minus marked position notional, as neither backtest result API exposes a separate balance series.

## Dataset provenance and known gaps

`fixtures/BTCUSDT_4h_2020-01_2026-11-25.csv` contains 15,125 candles from 2020-01-01 00:00 UTC through the candle opening 2026-11-25 20:00 UTC. Its exclusive cutoff is 2026-11-26 00:00 UTC. The CSV SHA-256 is `19d5d1fa75b83859cfd00160df13450888330eaba919041a1b063cc48857b898`. The adjacent `.provenance.json` records each of the 82 verified monthly archive checksums, REST page checksums, row counts, source cutoff, and CSV hash. One missing 4-hour interval (2020-02-19 12:00 through 16:00 UTC) is left unfilled; no candle was synthesized.

To rebuild the pinned fixture, run:

```bash
python validation/parity/fetch_btc_fixture.py
```

The builder verifies Binance's `.CHECKSUM` sidecar for every monthly ZIP and pins a fixed exclusive cutoff for the current-month REST request. The REST response page hash is recorded to make this snapshot auditable. The Binance public archive documents the monthly kline layout and the Spot timestamp change to microseconds beginning 2025-01-01: [Binance Public Data README](https://github.com/binance/binance-public-data/blob/master/README.md).

## Reproduction

Clone the Python reference repository separately (read-only for this task) and install Python 3.11+, NumPy, and pandas. From the Rust repository run:

```bash
python validation/parity/compare.py --python-ref /path/to/binance-trading-bot
```

The harness verifies both frozen fixture hashes, obtains all parameter grids from the Python strategy source, instruments the Python backtest source in memory to expose quantity and fee fields, runs the Rust dump example, and writes a compact JSON report (`parity-report-expanded.json` by default). It exits nonzero if any comparison differs. No generated report is checked in. Rust golden tests use only checked-in fixtures and do not require Python.

## Limits

- The Python BTC parquet is absent from its repository. Monthly archives and the pinned REST window are authoritative Binance source data and match the documented collection method, but are not the original parquet bytes. BTC parity is established against this verified source reconstruction.
- The frozen ETH fixture covers only Sep 2023–Dec 2024; all 62 Python grid configurations were compared on this fixed sample, not on the original full ETH research history.
- Python long/short research runs are outside the Rust engine's long-only spot scope and were not compared.
- Trailing-stop semantics, fee/slippage sizing, and strategy signals were compared for the exact original Python defaults and grid cases. Non-default execution `Config` values, other timeframes, and arbitrary parameter values were not included.
- EMA crossover and RSI mean reversion both exist in Python and Rust; no Rust-only strategy was introduced in this validation milestone.
