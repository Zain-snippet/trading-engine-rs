"""Build the pinned BTCUSDT 4h fixture from checksum-verified Binance data.

The original Python project excludes its parquet files. This reproduces its
monthly-archive plus current-month REST collection method up to a fixed cutoff.
No original Python project files are changed.
"""
from __future__ import annotations

import argparse
import csv
from datetime import datetime, timezone
import hashlib
import io
import json
from pathlib import Path
import urllib.parse
import urllib.request
import zipfile

BASE = "https://data.binance.vision/data/spot/monthly/klines/BTCUSDT/4h"
REST = "https://api.binance.com/api/v3/klines"
INTERVAL_MS = 4 * 60 * 60 * 1000
DEFAULT_CUTOFF = "2026-11-26T00:00:00Z"


def get(url: str) -> bytes:
    request = urllib.request.Request(url, headers={"User-Agent": "trading-engine-parity-validation/1.0"})
    with urllib.request.urlopen(request, timeout=60) as response:
        return response.read()


def sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def epoch_ms(iso: str) -> int:
    parsed = datetime.fromisoformat(iso.replace("Z", "+00:00"))
    if parsed.tzinfo is None:
        raise ValueError("cutoff must include a timezone, for example 2026-11-26T00:00:00Z")
    return int(parsed.astimezone(timezone.utc).timestamp() * 1000)


def as_millis(value: str) -> int:
    stamp = int(value)
    return stamp // 1000 if stamp >= 10**14 else stamp


def month_sequence(start: tuple[int, int], stop: tuple[int, int]):
    year, month = start
    while (year, month) <= stop:
        yield year, month
        year, month = (year + 1, 1) if month == 12 else (year, month + 1)


def archive_rows(year: int, month: int, manifest: list[dict]) -> list[list[str]]:
    name = f"BTCUSDT-4h-{year}-{month:02d}.zip"
    payload = get(f"{BASE}/{name}")
    checksum = get(f"{BASE}/{name}.CHECKSUM").decode("ascii").strip().split()
    if len(checksum) != 2 or checksum[1] != name or sha256(payload) != checksum[0].lower():
        raise ValueError(f"Binance checksum validation failed for {name}")
    with zipfile.ZipFile(io.BytesIO(payload)) as archive:
        members = archive.namelist()
        if len(members) != 1:
            raise ValueError(f"expected one CSV in {name}, found {members}")
        rows = list(csv.reader(io.TextIOWrapper(archive.open(members[0]), encoding="utf-8")))
    if rows and not rows[0][0].isdigit():
        rows = rows[1:]
    if any(len(row) < 6 for row in rows):
        raise ValueError(f"malformed kline row in {name}")
    manifest.append({"file": name, "sha256": checksum[0].lower(), "rows": len(rows)})
    return rows


def rest_rows(start_ms: int, end_ms: int) -> tuple[list[list[str]], list[dict]]:
    rows: list[list[str]] = []
    pages: list[dict] = []
    cursor = start_ms
    while cursor <= end_ms:
        query = urllib.parse.urlencode({
            "symbol": "BTCUSDT", "interval": "4h", "startTime": cursor,
            "endTime": end_ms, "limit": 1000,
        })
        payload = get(f"{REST}?{query}")
        values = json.loads(payload)
        if not isinstance(values, list):
            raise ValueError(f"unexpected Binance REST response: {values}")
        if not values:
            break
        page = [[str(value) for value in row[:6]] for row in values]
        if any(len(row) != 6 for row in page):
            raise ValueError("malformed Binance REST kline row")
        pages.append({"sha256": sha256(payload), "rows": len(page)})
        rows.extend(page)
        next_cursor = int(page[-1][0]) + 1
        if next_cursor <= cursor:
            raise ValueError("Binance REST pagination did not advance")
        cursor = next_cursor
        if len(page) < 1000:
            break
    return rows, pages


def build(cutoff: str, output: Path) -> None:
    cutoff_ms = epoch_ms(cutoff)
    cutoff_dt = datetime.fromtimestamp(cutoff_ms / 1000, timezone.utc)
    archive_stop = (cutoff_dt.year, cutoff_dt.month - 1) if cutoff_dt.month > 1 else (cutoff_dt.year - 1, 12)
    archives: list[dict] = []
    raw: list[list[str]] = []
    for year, month in month_sequence((2020, 1), archive_stop):
        raw.extend(archive_rows(year, month, archives))

    current_month = datetime(cutoff_dt.year, cutoff_dt.month, 1, tzinfo=timezone.utc)
    current_start_ms = int(current_month.timestamp() * 1000)
    current, pages = rest_rows(current_start_ms, cutoff_ms - 1)
    raw.extend(current)

    by_time: dict[int, list[str]] = {}
    for row in raw:
        opened = as_millis(row[0])
        if opened < 1_577_836_800_000 or opened >= cutoff_ms:
            continue
        normalized = [datetime.fromtimestamp(opened / 1000, timezone.utc).strftime("%Y-%m-%d %H:%M:%S"), *row[1:6]]
        by_time.setdefault(opened, normalized)
    ordered = sorted(by_time.items())
    if not ordered:
        raise ValueError("no BTC candles were collected")
    output.parent.mkdir(parents=True, exist_ok=True)
    with output.open("w", newline="", encoding="utf-8") as stream:
        writer = csv.writer(stream, lineterminator="\n")
        writer.writerow(["time", "open", "high", "low", "close", "volume"])
        writer.writerows(row for _, row in ordered)

    bytes_written = output.read_bytes()
    gaps = [
        {"after_ms": prior, "before_ms": current}
        for (prior, _), (current, _) in zip(ordered, ordered[1:])
        if current - prior != INTERVAL_MS
    ]
    provenance = {
        "source": "Binance Spot Public Data monthly kline archives and public REST klines",
        "symbol": "BTCUSDT", "interval": "4h",
        "range_start_utc": datetime.fromtimestamp(ordered[0][0] / 1000, timezone.utc).isoformat(),
        "cutoff_exclusive_utc": cutoff_dt.isoformat(),
        "last_open_time_utc": datetime.fromtimestamp(ordered[-1][0] / 1000, timezone.utc).isoformat(),
        "row_count": len(ordered), "gap_count": len(gaps), "gaps": gaps,
        "csv_sha256": sha256(bytes_written), "csv_bytes": len(bytes_written),
        "archive_checksums": archives,
        "rest_endpoint": REST, "rest_page_checksums": pages,
    }
    output.with_suffix(output.suffix + ".provenance.json").write_text(
        json.dumps(provenance, indent=2) + "\n", encoding="utf-8"
    )
    print(json.dumps({k: provenance[k] for k in (
        "symbol", "interval", "range_start_utc", "cutoff_exclusive_utc",
        "last_open_time_utc", "row_count", "gap_count", "csv_sha256", "csv_bytes",
    )}, indent=2))


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--cutoff", default=DEFAULT_CUTOFF, help="exclusive UTC cutoff")
    parser.add_argument("--output", type=Path, default=Path("fixtures/BTCUSDT_4h_2020-01_2026-11-25.csv"))
    args = parser.parse_args()
    build(args.cutoff, args.output)
