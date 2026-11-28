# Local Binance Spot Testnet CLI

This CLI is fixed to Binance Spot Testnet. It has no production endpoint. `run`
can submit Testnet market orders when the configured strategy and risk checks
produce an entry or exit; review the configuration and Testnet balances first.
No command sends an order as part of the automated test suite.

## Windows PowerShell setup

Install a current stable Rust toolchain, clone the repository, then from the
repository root create a local config and set the credentials in the current
PowerShell session:

```powershell
Copy-Item config.example.toml config.toml
$env:BINANCE_TESTNET_API_KEY = "your_spot_testnet_key"
$env:BINANCE_TESTNET_API_SECRET = "your_spot_testnet_secret"
cargo run --locked -p trading-engine-cli -- validate-config config.toml
```

Credentials are read only from these environment variables. The executable
does not parse `.env` files. Keep credentials out of command history where
possible and do not commit the local config, credentials, state, or logs.

## Read-only inspection

Account and order inspection need Testnet credentials. Public candles and
exchange-rule metadata do not:

```powershell
cargo run --locked -p trading-engine-cli -- account --environment testnet
cargo run --locked -p trading-engine-cli -- open-orders --environment testnet
cargo run --locked -p trading-engine-cli -- reconcile --environment testnet config.toml
cargo run --locked -p trading-engine-cli -- rules ETH/USDT --environment testnet
cargo run --locked -p trading-engine-cli -- candles ETH/USDT --interval 4h --environment testnet
```

`account` reports Testnet free and locked balances. `open-orders` reports current
exchange orders. `rules` prints public exchange metadata and filters. `candles`
only prints completed candles.

## Starting and controlling the engine

```powershell
cargo run --locked -p trading-engine-cli -- run --environment testnet config.toml
```

The engine fetches current Testnet ticker prices, completed candles, account
balances, and symbol filters. It reconciles persisted orders and positions
before entering its polling loop. Persisted state goes under `state_dir`; the
CSV journal goes under `log_dir`. Do not run more than one engine against the
same paths.

In another PowerShell window, control the running process using the same
configuration:

```powershell
cargo run --locked -p trading-engine-cli -- pause config.toml
cargo run --locked -p trading-engine-cli -- kill config.toml
cargo run --locked -p trading-engine-cli -- stop config.toml
cargo run --locked -p trading-engine-cli -- status config.toml
```

`pause` and `kill` prevent new entries; existing signal exits and stop-losses
remain active. `stop` prevents new entries and asks the loop to continue until
tracked positions are closed, then return. It does not force liquidation, and
it can continue waiting if no protective exit occurs. `resume` removes the
pause, kill and stop markers and clears the persisted halt, but refuses while
reconciliation findings remain. After correcting the Testnet account/order
mismatch, run the one-shot `reconcile` command, inspect `status`, then run
`resume`.

Stopping the process directly does not liquidate positions. On the next start
or one-shot `reconcile` command, the engine looks up persisted active or incompletely accounted orders by
client ID, checks open exchange orders, and compares tracked base quantities to
Testnet balances. A mismatch is recorded and local positions are retained. An
untracked open exchange order or failed open-order lookup halts new entries;
protective exits for already tracked positions remain available. Reconciliation
findings appear in `status` and in the journal where applicable.

## Safety and limitations

- The CLI requires `--environment testnet` for all commands that access the
  exchange. The REST base URL is compiled as the Spot Testnet URL.
- Strategies and long-only spot risk calculations reuse existing workspace
  crates; this integration does not enable shorting, margin, leverage, or
  futures.
- Exact symbol quantity filters are checked before order submission.
- Ambiguous order submissions are reconciled by deterministic client ID and
  are not automatically resubmitted.
- State-file updates are atomic and fill accounting plus fill IDs are saved in
  one state update. The CSV journal is a separate append-only file and can lag
  state if the process stops between writes.
- Fees paid in a third asset are recorded but are not converted into quote
  accounting. The integration has mocked tests but has not been certified by a
  credentialed Testnet order run.
- `run` is an unattended loop while its process remains active. The CLI does
  not install itself as a Windows service or provide process supervision.
