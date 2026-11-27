# trading-engine-rs

A modular Rust migration of the long-only Binance Spot research and paper-trading bot at `mafzalkalwardev/binance-trading-bot`. The original Python repository is a read-only behavioral reference.

## Current implementation

The workspace contains eight library crates:

- `trading-engine-core`: validated candles, UTC timestamps, symbols, money, long-only signals and positions, fills, and market-order intents.
- `trading-engine-config`: strict TOML configuration and validation, including the original ETH/USDT and BTC/USDT defaults.
- `trading-engine-indicators`: EMA, RSI, true range, pandas-compatible ATR, and shifted Donchian channels.
- `trading-engine-strategies`: long-only Donchian breakout, EMA crossover, and RSI mean-reversion signals.
- `trading-engine-risk`: paper and backtest sizing helpers, equity calculation, initial stops, and daily loss/kill/pause entry gates.
- `trading-engine-persistence`: JSON state, CSV journal, and known-secret redaction.
- `trading-engine-sim`: deterministic single-process simulated orchestration using caller-supplied completed candles and ticker prices.
- `trading-engine-backtest`: single-symbol historical simulation with next-candle-open execution and performance metrics.

The deterministic ETH 4-hour fixture is in `fixtures/`. Unit and fixture tests cover the implemented library behavior. CI is configured to run formatting, Clippy, and Cargo tests.

## Not implemented yet

This repository is not a runnable unattended trading bot. It has no executable, market data client, or exchange adapter. There is no Binance integration, order reconciliation, exchange precision/filter handling, or process-level locking for persisted files. State and journal access is single-writer; do not run multiple processes against the same paths.

The simulator evaluates each symbol's strategy on completed candle history and fills at the ticker price supplied for that step. It persists positions, balances, closed trades, and last-processed candle IDs, and writes deterministic journal timestamps from candle time. Pause and kill gates block new entries; they do not stop this engine or suppress a position's signal exit or stop-loss. Emergency process shutdown is an operator action and does not liquidate positions. On restart, the persisted last candle prevents replaying the same entry.

The backtester operates on one symbol per run, matching the Python backtest API; callers run symbols independently. It uses close-generated signals, next-open orders, adverse slippage, per-fill fees, intrabar stop checks, and final-close liquidation. Metrics require initial capital explicitly so returns are not silently measured against the Python helper's hard-coded default. The reference `metrics()` helper defaults to 10,000; callers that omit `initial=cfg.initial` get incorrect total return/CAGR and synthetic daily Sharpe when using another starting balance. The reference runner passes the custom initial balance for its primary grid but not consistently in all evaluation paths.

Live execution must not be connected until exchange precision, partial fills, restart recovery, and duplicate-order handling are implemented and tested.

## Behavioral parity constraints

- Timestamps are UTC milliseconds since Unix epoch. Naive fixture timestamps are interpreted as UTC.
- `f64` is retained for parity with Python/pandas calculations.
- Donchian channels exclude the current candle through shifted rolling extrema.
- ATR preserves pandas `ewm(alpha=1/n, adjust=False, min_periods=n)` initialization, not an SMA-seeded Wilder initialization.
- Historical backtest fills use the next candle open with modeled adverse slippage and fees. Paper trading fills use the current public ticker price after candle close. These models are implemented separately in `backtest` and `sim`.
- Existing strategy parameters, risk calculations, timestamp handling, and state formats should not change without parity tests.

The original Python code was inspected read-only, but its dependencies are unavailable in this environment, so no cross-language output comparison has been run. Complete parity is not established.

## Workspace

```text
crates/
  core/          domain types
  config/        typed configuration
  indicators/    technical indicators
  strategies/    long-only strategy signals
  risk/          sizing and entry risk gates
  persistence/   state, journal, redaction
  sim/           deterministic ticker-price simulation orchestration
  backtest/      next-candle-open historical backtester and metrics
fixtures/        frozen parity data
```

## Development checks

```bash
cargo fmt --all -- --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-targets
```

GitHub Actions runs the same checks on pushes to `main` and pull requests.
