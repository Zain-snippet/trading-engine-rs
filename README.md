# trading-engine-rs

A modular Rust migration of the long-only Binance Spot research and paper-trading bot at `mafzalkalwardev/binance-trading-bot`. The original Python repository is a read-only behavioral reference.

## Current implementation

The workspace contains six library crates:

- `trading-engine-core`: validated candles, UTC timestamps, symbols, money, long-only signals and positions, fills, and market-order intents.
- `trading-engine-config`: strict TOML configuration and validation, including the original ETH/USDT and BTC/USDT defaults.
- `trading-engine-indicators`: EMA, RSI, true range, pandas-compatible ATR, and shifted Donchian channels.
- `trading-engine-strategies`: long-only Donchian breakout, EMA crossover, and RSI mean-reversion signals.
- `trading-engine-risk`: paper and backtest sizing helpers, equity calculation, initial stops, and daily loss/kill/pause entry gates.
- `trading-engine-persistence`: JSON state, CSV journal, and known-secret redaction.

The deterministic ETH 4-hour fixture is in `fixtures/`. Unit and fixture tests cover the implemented library behavior. CI is configured to run formatting, Clippy, and Cargo tests.

## Not implemented yet

This repository is not a runnable trading bot. It has no executable, end-to-end orchestration loop, market data client, broker/exchange adapter, integrated simulated broker, or historical backtester. There is no Binance integration, order reconciliation, exchange precision/filter handling, or process-level locking for persisted files. State storage is single-writer; do not run multiple bot processes against the same state or journal paths.

Kill and pause gates block new entries at the risk-policy layer. Protective exits and stop handling must remain available when the orchestration layer is added. Live execution must not be connected until exchange precision, partial fills, restart recovery, and duplicate-order handling are implemented and tested.

## Behavioral parity constraints

- Timestamps are UTC milliseconds since Unix epoch. Naive fixture timestamps are interpreted as UTC.
- `f64` is retained for parity with Python/pandas calculations.
- Donchian channels exclude the current candle through shifted rolling extrema.
- ATR preserves pandas `ewm(alpha=1/n, adjust=False, min_periods=n)` initialization, not an SMA-seeded Wilder initialization.
- Historical backtest fills must use the next candle open with modeled fee and slippage. Paper trading fills must use the current public ticker price after candle close. These models are distinct and have not yet been implemented as integrated execution paths.
- Existing strategy parameters, risk calculations, timestamp handling, and state formats should not change without parity tests.

The committed Python behavioral baseline has not been executed in this environment, so complete cross-language parity is not yet established.

## Workspace

```text
crates/
  core/          domain types
  config/        typed configuration
  indicators/    technical indicators
  strategies/    long-only strategy signals
  risk/          sizing and entry risk gates
  persistence/   state, journal, redaction
fixtures/        frozen parity data
```

## Development checks

```bash
cargo fmt --all -- --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-targets
```

GitHub Actions runs the same checks on pushes to `main` and pull requests.
