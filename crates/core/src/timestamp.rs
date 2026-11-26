use std::fmt;

use chrono::{DateTime, NaiveDateTime, Utc};

use crate::error::DomainError;

/// A UTC instant represented as milliseconds since the Unix epoch.
///
/// The original Python bot treats timezone-naive candle timestamps as UTC:
/// `pandas.to_datetime(..., unit="ms")` produces UTC-naive values, and the
/// replay test explicitly localizes them to UTC. This type makes that
/// convention explicit and never consults the local machine timezone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Timestamp(i64);

impl Timestamp {
    /// Creates a timestamp from Unix epoch milliseconds.
    pub fn from_millis(millis: i64) -> Result<Self, DomainError> {
        if DateTime::<Utc>::from_timestamp_millis(millis).is_some() {
            Ok(Self(millis))
        } else {
            Err(DomainError::InvalidTimestamp(format!(
                "{millis} ms is outside the supported range"
            )))
        }
    }

    /// Parses the fixture/CSV timestamp format `YYYY-MM-DD HH:MM:SS` as UTC.
    pub fn parse_fixture(value: &str) -> Result<Self, DomainError> {
        let naive = NaiveDateTime::parse_from_str(value, "%Y-%m-%d %H:%M:%S")
            .map_err(|_| DomainError::InvalidTimestamp(value.to_owned()))?;
        Self::from_millis(naive.and_utc().timestamp_millis())
    }

    #[must_use]
    pub const fn as_millis(self) -> i64 {
        self.0
    }

    /// Formats the timestamp using the original fixture format in UTC.
    #[must_use]
    pub fn to_fixture_string(&self) -> String {
        DateTime::<Utc>::from_timestamp_millis(self.0)
            .expect("Timestamp values are range-checked at construction")
            .format("%Y-%m-%d %H:%M:%S")
            .to_string()
    }
}

impl fmt::Display for Timestamp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_fixture_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_fixture_timestamps_as_utc() {
        let first = Timestamp::parse_fixture("2023-09-01 00:00:00").unwrap();
        let last = Timestamp::parse_fixture("2024-12-31 20:00:00").unwrap();

        assert_eq!(first.as_millis(), 1_693_526_400_000);
        assert_eq!(last.as_millis(), 1_735_675_200_000);
        assert_eq!(first.to_fixture_string(), "2023-09-01 00:00:00");
        assert_eq!(last.to_fixture_string(), "2024-12-31 20:00:00");
    }

    #[test]
    fn parses_epoch_milliseconds() {
        let ts = Timestamp::from_millis(1_693_526_400_000).unwrap();
        assert_eq!(ts.to_fixture_string(), "2023-09-01 00:00:00");
        assert_eq!(ts.to_string(), "2023-09-01 00:00:00");
    }

    #[test]
    fn rejects_other_timestamp_formats() {
        assert!(Timestamp::parse_fixture("2023-09-01T00:00:00").is_err());
        assert!(Timestamp::parse_fixture("2023-09-01").is_err());
        assert!(Timestamp::parse_fixture("2023-02-30 00:00:00").is_err());
    }

    #[test]
    fn rejects_out_of_range_milliseconds() {
        assert!(Timestamp::from_millis(i64::MAX).is_err());
        assert!(Timestamp::from_millis(i64::MIN).is_err());
    }
}
