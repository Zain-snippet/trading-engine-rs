# Binance Spot Testnet integration architecture

## Existing boundaries

- `core` owns validated `Candle`, `Money`, `SymbolId`, `Fill`, `OrderIntent`, and long `Position` domain types.
- `config` owns the existing strategy, risk, sizing and local state settings. It has no exchange environment setting today.
- `strategies` maps completed candle histories to causal per-bar signals; strategy code does not depend on the source of candles.
- `risk` owns position sizing, marked equity and daily entry gating.
- `persistence` atomically saves the current paper/backtest state and journals CSV events. Its `PersistedPosition` and `SimBrokerState` represent immediate full fills, not exchange order lifecycle or partial fills.
- `sim` is a caller-ticker simulator which coordinates strategy, risk, immediate fills and existing persistence. `backtest` is a separate completed-candle/next-open model. Neither is an exchange adapter.

## Integration design

The `binance` crate contains only the Spot Testnet transport and exchange-specific types. Its REST base is fixed to `https://testnet.binance.vision/api`; no production URL is configurable. Public market-data operations and signed account/trade operations are separate. Credentials come from `BINANCE_TESTNET_API_KEY` and `BINANCE_TESTNET_API_SECRET`, are not Debug-printable, and never enter configuration files or diagnostics. Unit tests use mocked HTTP responses.

Market data is converted at the boundary into the existing validated `Candle` and `SymbolId` types. The adapter returns candle open and close timestamps so the caller can exclude a candle unless its close time is earlier than a captured UTC server/local decision time. The exchange adapter does not create strategy signals.

Use the existing strategy and risk crates to make entry decisions. Exchange symbol filters and free account balances are applied after risk sizing and before creating an order request. A narrow execution interface can be introduced only at the point where the Testnet engine needs order placement/status/cancel; it should represent exchange order status and actual fills, rather than reuse the simulator's full-fill assumptions.

Persist Testnet order intents with deterministic client order IDs before submission. An uncertain POST result is reconciled by client ID/order queries before any further submit. Persisted exchange orders, actual fills, commissions and reconciliation findings should be additive, version-tolerant state; the existing simulator state and parity behavior stay unchanged. Position quantities and accounting derive from cumulative executed fills, never requested quantities.

The runtime loop is single-writer and deterministic per candle decision: completed market data → strategy → risk/entry controls → exchange validation → durable intent → Testnet submit/reconcile → actual fill accounting → state and journal. Stop checks and other protective exits are evaluated even while pause or kill blocks new entries. Pause/kill are entry gates; process shutdown stops polling and does not itself liquidate positions. No emergency liquidation rule is added.

## Milestone sequence

1. This design and constraints.
2. Public REST market data with mocked response parsing, validation, bounded safe retries and completed-candle filtering.
3. Testnet-only signed REST client and credentials from `BINANCE_TESTNET_API_KEY` / `BINANCE_TESTNET_API_SECRET`.
4. Decimal-safe filter and balance validation before order submission.
5. Durable order lifecycle with partial-fill and ambiguous-submit reconciliation.
6. Startup reconciliation among persisted state, account balances, open orders and trade history.
7. Explicit Testnet-only startup guard and pause/kill/daily-risk controls.
8. CLI commands for validation, inspection and controlled Testnet runtime.

Each milestone stays independently testable. Public calls are mock-tested; authenticated live calls require Testnet credentials. No test or default command submits an order. Any controlled Testnet order must be an explicit CLI operation and reported separately.

## Protocol sources

- [Spot Testnet general information](https://developers.binance.com/en/docs/products/spot/testnet/general-info): Testnet REST base and environment details.
- [Spot REST API](https://developers.binance.com/en/docs/products/spot/rest-api): endpoint semantics, signed request rules and errors.
- [Spot request security](https://developers.binance.com/en/docs/products/spot/rest-api/request-security): HMAC signing, timestamp and receive-window protocol.
- [Spot filters](https://developers.binance.com/en/docs/products/spot/filters): exchange-side filter definitions.
- [Original Python paper engine](https://github.com/mafzalkalwardev/binance-trading-bot/blob/main/paperbot/engine.py) and [state](https://github.com/mafzalkalwardev/binance-trading-bot/blob/main/paperbot/state.py): read-only behavioral references.

The existing paper engine's ticker-price execution and the historical backtest's next-candle-open execution are distinct models and remain distinct.
