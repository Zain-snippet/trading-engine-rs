# trading-engine-rs

A modular Rust migration of the long-only Binance Spot research and paper-trading bot at `mafzalkalwardev/binance-trading-bot`. The original Python repository is a read-only behavioral reference.

## Current implementation

The workspace contains the research libraries plus a separate Binance Spot Testnet integration and CLI:

- `trading-engine-core`: validated candles, UTC timestamps, symbols, money, long-only signals and positions, fills, and market-order intents.
- `trading-engine-config`: strict TOML configuration and validation, including the original ETH/USDT and BTC/USDT defaults.
- `trading-engine-indicators`: EMA, RSI, true range, pandas-compatible ATR, and shifted Donchian channels.
- `trading-engine-strategies`: long-only Donchian breakout, EMA crossover, and RSI mean-reversion signals.
- `trading-engine-risk`: paper and backtest sizing helpers, equity calculation, initial stops, and daily loss/kill/pause entry gates.
- `trading-engine-persistence`: JSON state, CSV journal, and known-secret redaction.
- `trading-engine-sim`: deterministic single-process simulated orchestration using caller-supplied completed candles and ticker prices.
- `trading-engine-backtest`: single-symbol historical simulation with next-candle-open execution and performance metrics.
- `trading-engine-binance`: public market data and authenticated Binance Spot Testnet API, exact symbol filter validation, and recoverable order lifecycle types.
- `trading-engine-live`: Testnet-only orchestration, durable order intents, exchange reconciliation, fills, position updates, and protective exits.
- `trading-engine-cli`: local CLI for configuration checks, Testnet inspection, engine controls, and the Testnet loop.

The deterministic ETH 4-hour fixture is in `fixtures/`. Unit and fixture tests cover the implemented library behavior. CI is configured to run formatting, Clippy, and Cargo tests.

## Operating model and limitations

Binance execution is restricted to Spot Testnet by a fixed API host and an explicit `--environment testnet` argument. The CLI supports long-only spot market orders; there is no production endpoint, futures, leverage, shorting, or process-level locking. State and journal access is single-writer; do not run multiple trading processes against the same paths. API credentials are read from `BINANCE_TESTNET_API_KEY` and `BINANCE_TESTNET_API_SECRET`; set them in the process environment and never put credentials in source control. The application does not auto-load `.env` files.

The simulator evaluates each symbol's strategy on completed candle history and fills at the ticker price supplied for that step. It persists positions, balances, closed trades, and last-processed candle IDs, and writes deterministic journal timestamps from candle time. Pause and kill gates block new entries; they do not stop this engine or suppress a position's signal exit or stop-loss. Emergency process shutdown is an operator action and does not liquidate positions. On restart, the persisted last candle prevents replaying the same entry.

The backtester operates on one symbol per run, matching the Python backtest API; callers run symbols independently. It uses close-generated signals, next-open orders, adverse slippage, per-fill fees, intrabar stop checks, and final-close liquidation. Metrics require initial capital explicitly so returns are not silently measured against the Python helper's hard-coded default. The reference `metrics()` helper defaults to 10,000; callers that omit `initial=cfg.initial` get incorrect total return/CAGR and synthetic daily Sharpe when using another starting balance. The reference runner passes the custom initial balance for its primary grid but not consistently in all evaluation paths.

The Testnet loop writes order intent before submission and uses deterministic client IDs to reconcile uncertain submissions after restart. It does not blindly retry an ambiguous POST. Exchange fills and position changes are saved together; journal append is separate and can lag state if the process stops between those writes. Startup compares local tracked base quantities with exchange balances and records discrepancies without silently replacing local positions. Fees paid in an asset other than the traded base or quote asset are recorded but not included in quote-denominated position accounting. The adapter does not provide a futures or production mode.

`pause` blocks new entries but keeps protective exits active. `kill` also blocks entries and persists the halted risk state; `resume` clears control markers and the persisted halt. `stop` requests a graceful drain: no new entries, continue protective exits, then stop the process once tracked positions are closed. These controls change engine behavior; they do not terminate or supervise the OS process. Closing the process directly leaves positions on Testnet and requires reconciliation on restart.

See [`docs/binance-testnet-setup.md`](docs/binance-testnet-setup.md) for Windows setup and command examples. A starter configuration is provided in [`config.example.toml`](config.example.toml). Exchange commands require `--environment testnet`: `account`, `open-orders`, `reconcile`, `rules SYMBOL`, `candles SYMBOL`, and `run`. `run` can submit Spot Testnet market orders when strategy and risk conditions pass. The default config uses ETH/USDT and BTC/USDT, a 4-hour interval, and `state/` and `logs/` paths. Local controls are `status|pause|kill|stop|resume`.

## Behavioral parity constraints

- Timestamps are UTC milliseconds since Unix epoch. Naive fixture timestamps are interpreted as UTC.
- `f64` is retained for parity with Python/pandas calculations.
- Donchian channels exclude the current candle through shifted rolling extrema.
- ATR preserves pandas `ewm(alpha=1/n, adjust=False, min_periods=n)` initialization, not an SMA-seeded Wilder initialization.
- Historical backtest fills use the next candle open with modeled adverse slippage and fees. Paper trading fills use the current public ticker price after candle close. These models are implemented separately in `backtest` and `sim`.
- Existing strategy parameters, risk calculations, timestamp handling, and state formats should not change without parity tests.

Expanded Python/Rust validation scope, exact tested configurations, dataset provenance, tolerances, and known limitations are documented in [`validation/parity/README.md`](validation/parity/README.md). Demonstrated parity is limited to those listed long-only spot backtest configurations and datasets; it is not a blanket claim for every strategy, configuration, or market. The original Python checkout remains read-only.

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
  binance/       Spot Testnet API and filter validation
  live/          Testnet execution orchestration and recovery
  cli/           local operator CLI
validation/parity/  Python reference comparison and fixture provenance tools
fixtures/        frozen parity data
```

## Development checks

```bash
cargo fmt --all -- --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-targets
```

GitHub Actions runs the same checks on pushes to `main` and pull requests.
