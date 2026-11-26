use sha2::{Digest, Sha256};
use trading_engine_core::{Candle, Money, Timestamp};

const FIXTURE: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../fixtures/ETHUSDT_4h_2023-09_2024-12.csv"
));

const FIXTURE_SHA256: &str = "017E927C8C681E7B2E812A5ED77CB4F2A0509D5B947F92BFF247FD307F225C23";
const FIXTURE_BYTES: usize = 181_823;
const FIXTURE_LINES: usize = 2_929;
const FIXTURE_DATA_ROWS: usize = 2_928;
const FOUR_HOURS_MS: i64 = 14_400_000;

#[test]
fn fixture_bytes_match_the_original_python_blob() {
    assert_eq!(FIXTURE.len(), FIXTURE_BYTES);

    let digest = Sha256::digest(FIXTURE);
    let checksum: String = digest.iter().map(|byte| format!("{byte:02X}")).collect();
    assert_eq!(checksum, FIXTURE_SHA256);

    assert_eq!(
        FIXTURE.iter().filter(|byte| **byte == b'\n').count(),
        FIXTURE_LINES
    );
    assert_eq!(
        FIXTURE.iter().filter(|byte| **byte == b'\r').count(),
        0,
        "the original fixture uses LF-only line endings"
    );
}

#[test]
fn fixture_has_the_expected_header_and_row_count() {
    let text = std::str::from_utf8(FIXTURE).expect("fixture is ASCII/UTF-8");
    let mut lines = text.lines();

    assert_eq!(lines.next(), Some("time,open,high,low,close,volume"));
    assert_eq!(lines.count(), FIXTURE_DATA_ROWS);
}

#[test]
fn fixture_parses_into_valid_candles_with_utc_timestamps() {
    let text = std::str::from_utf8(FIXTURE).expect("fixture is ASCII/UTF-8");
    let mut lines = text.lines();
    assert_eq!(lines.next(), Some("time,open,high,low,close,volume"));

    let mut candles = Vec::with_capacity(FIXTURE_DATA_ROWS);
    for (index, line) in lines.enumerate() {
        let fields: Vec<&str> = line.split(',').collect();
        assert_eq!(
            fields.len(),
            6,
            "unexpected field count on data row {index}"
        );

        let timestamp = Timestamp::parse_fixture(fields[0])
            .unwrap_or_else(|error| panic!("bad timestamp on data row {index}: {error}"));
        let open = Money::new(fields[1].parse().unwrap()).unwrap();
        let high = Money::new(fields[2].parse().unwrap()).unwrap();
        let low = Money::new(fields[3].parse().unwrap()).unwrap();
        let close = Money::new(fields[4].parse().unwrap()).unwrap();
        let volume = fields[5].parse().unwrap();

        let candle = Candle::new(timestamp, open, high, low, close, volume)
            .unwrap_or_else(|error| panic!("invalid candle on data row {index}: {error}"));
        candles.push(candle);
    }

    assert_eq!(candles.len(), FIXTURE_DATA_ROWS);

    let first = &candles[0];
    assert_eq!(first.open_time().as_millis(), 1_693_526_400_000);
    assert_eq!(first.open().as_f64(), 1_645.77);
    assert_eq!(first.high().as_f64(), 1_654.53);
    assert_eq!(first.low().as_f64(), 1_645.23);
    assert_eq!(first.close().as_f64(), 1_652.2);
    assert_eq!(first.volume(), 22943.8273);

    let last = &candles[FIXTURE_DATA_ROWS - 1];
    assert_eq!(last.open_time().as_millis(), 1_735_675_200_000);
    assert_eq!(last.open().as_f64(), 3_355.37);
    assert_eq!(last.high().as_f64(), 3_364.64);
    assert_eq!(last.low().as_f64(), 3_328.47);
    assert_eq!(last.close().as_f64(), 3_337.78);
    assert_eq!(last.volume(), 31006.2927);

    for pair in candles.windows(2) {
        let step = pair[1].open_time().as_millis() - pair[0].open_time().as_millis();
        assert_eq!(step, FOUR_HOURS_MS);
    }
}
