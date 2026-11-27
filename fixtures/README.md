# Frozen parity fixtures

`ETHUSDT_4h_2023-09_2024-12.csv` is a byte-for-byte copy of the committed fixture from the original Python repository:

- Repository: `https://github.com/mafzalkalwardev/binance-trading-bot`
- Original path: `tests/fixtures/ETHUSDT_4h_2023-09_2024-12.csv`
- Original Git blob: `9137df9a042bf0b510ca9c19c5147f9a86b3a00a`

## Integrity

| Property | Value |
|---|---|
| SHA-256 | `017E927C8C681E7B2E812A5ED77CB4F2A0509D5B947F92BFF247FD307F225C23` |
| Size | `181,823` bytes |
| Data rows | `2,928` |
| Header | `time,open,high,low,close,volume` |
| Total lines | `2,929` (1 header + 2,928 data rows) |
| Line endings | LF (`\n`) on every line |

The fixture is marked `-text` in `.gitattributes` so Git preserves its original bytes and LF line endings. Do not regenerate, reformat, normalize, or edit it.

## Timestamp convention

The `time` column uses `YYYY-MM-DD HH:MM:SS` and is timezone-naive in the Python fixture. The original bot treats these values as UTC: `pandas.to_datetime(..., unit="ms")` produces UTC-naive timestamps, and the replay test explicitly localizes them to UTC. The Rust engine therefore parses fixture timestamps as UTC and never uses the local machine timezone.

## Verification

`crates/core/tests/fixture.rs` verifies the SHA-256 checksum, byte size, LF-only line endings, header, exact data-row count, first and last rows, strict 4-hour spacing, and that every row parses into a valid `Candle`.

## BTCUSDT 4-hour research reconstruction

`BTCUSDT_4h_2020-01_2026-11-25.csv` reconstructs the original Python project's documented Binance Spot kline collection range and method. The Python repo's `data/` directory says its parquet files are not committed, so this is explicitly a verified source reconstruction, not a byte-identical copy of the missing parquet.

| Property | Value |
|---|---|
| Source | Binance Spot monthly kline archives plus public REST for November 2026 |
| Range | 2020-01-01 00:00 UTC through 2026-11-25 20:00 UTC (exclusive cutoff 2026-11-26 00:00 UTC) |
| Rows | 15,125 |
| SHA-256 | `19d5d1fa75b83859cfd00160df13450888330eaba919041a1b063cc48857b898` |
| Missing interval | 2020-02-19 12:00–16:00 UTC; left unfilled from source data |
| Provenance | `BTCUSDT_4h_2020-01_2026-11-25.csv.provenance.json` |

The provenance manifest records and validates the Binance checksum sidecars for all 82 monthly archive ZIPs, each archive's row count, the pinned REST page response hash, fixture range, one source gap, and final CSV hash. Rebuild with `python validation/parity/fetch_btc_fixture.py`. No candle is interpolated or substituted.
