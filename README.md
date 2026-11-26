# trading-engine-rs

A modular Rust trading engine that will be migrated from the Python Binance Spot research and paper-trading bot at `mafzalkalwardev/binance-trading-bot`.

This repository is independent. The original Python repository is kept read-only as a reference and will not be modified or used as a submodule.

## Current status

**M1 is complete**: the workspace contains only the two crates needed so far.

- `crates/core` (`trading-engine-core`) provides the core domain types: candles, timeframes, symbols, timestamps, money, signals, positions, fills, order intents, sides, and domain errors.
- `crates/config` (`trading-engine-config`) provides typed configuration with the verified Python defaults, TOML parsing, and validation.
- `fixtures/` contains the frozen ETH 4h parity fixture and its integrity documentation.
- CI runs formatting checks, Clippy, and Cargo tests.
- No indicators, strategies, position sizing, risk engine, backtesting, exchange clients, execution logic, persistence, or WebSockets are implemented yet.
- No API keys, credentials, or private configuration are included.

## Workspace

```text
crates/
  core/      trading-engine-core
  config/    trading-engine-config
fixtures/    frozen parity data
```

The package names use a `trading-engine-` prefix so they cannot be confused with Rust's built-in `core` crate; the directory names stay short. Placeholder crates for future components are intentionally not created.

## Conventions

- Timestamps are UTC milliseconds since the Unix epoch. Naive fixture timestamps are interpreted as UTC, never as local time.
- `f64` is used for numerical parity with Python/pandas during Stage A.
- Binance symbol formatting is isolated in `SymbolId::binance_symbol()`; the symbol identity itself is exchange-neutral.
- Default `state_dir` and `log_dir` are relative (`state`, `logs`) rather than Python's source-root paths. This is a portability decision; callers running from another directory should set explicit paths.
- `ATR_PERIOD = 14` is a fixed constant because the Python bot does not expose an ATR period in `BotCfg`.

## Migration philosophy

The migration is divided into three strictly separate stages. Do not combine them.

### Stage A — Behavioral parity

First reproduce the Python bot's mathematical and decision behavior in Rust.

Verified details that must be preserved:

- Donchian channels use **shifted rolling extrema**; the current candle's high and low are excluded.
- ATR must reproduce the existing pandas `ewm(alpha=1/n, adjust=False, min_periods=n)` initialization. Do not silently switch to a textbook Wilder ATR that starts from an SMA.
- Backtesting fills signals at the **next candle open** with modeled fee and slippage.
- Paper trading fills at the **current public ticker price** after a candle closes.
- These two fill models are different and must remain separate during parity work.
- Existing strategy parameters, risk calculations, rounding, timestamp handling, and state formats must not be silently changed.

Unsafe legacy operational behavior — for example, treating partially filled orders as complete fills — must be documented and evaluated separately. Do not blindly reproduce unsafe execution behavior in the Rust engine.

### Stage B — Engineering optimization

Only after parity tests pass, optimize performance, memory usage, allocations, API-call patterns, error recovery, and execution reliability. Engineering optimization must not change trading behavior without a new parity comparison.

### Stage C — Strategy improvements

Only after Stage B is complete should strategy logic, parameters, or trading approaches be changed. Strategy research must remain separate from engineering work.

## Python baseline

The original Python test suite could not be executed in the audit environment because Python was unavailable. The Python behavioral baseline therefore remains **unverified**. No parity claim should be made until the baseline is executed separately using pinned dependencies and the committed fixture.

## Parity fixture

`fixtures/ETHUSDT_4h_2023-09_2024-12.csv` is a byte-identical copy of the original committed fixture. Its SHA-256, size, row count, and LF line endings are recorded in `fixtures/README.md` and enforced by `crates/core/tests/fixture.rs`.

## Planned direction

The workspace will expand only when implementation requires it. Planned responsibilities include core domain types, configuration, market data, indicators, strategies, risk management, execution, persistence, exchange integrations, backtesting, CLI, and monitoring.

During parity work, `f64` is used for numerical parity with Python/pandas. `reqwest` is the planned thin REST HTTP client; no exchange SDK or API dependency is introduced yet.

## Development

```bash
cargo fmt --all -- --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-targets
```

GitHub Actions runs the same three checks on pushes to `main` and on pull requests.
