use std::{fmt, str::FromStr};

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::error::DomainError;

/// A candle interval supported by the original Python bot.
///
/// The source defines `TF_MS = {"1h": 3_600_000, "4h": 14_400_000}`. Other
/// intervals are intentionally rejected in the parity stage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum TimeFrame {
    OneHour,
    FourHour,
}

impl TimeFrame {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::OneHour => "1h",
            Self::FourHour => "4h",
        }
    }

    #[must_use]
    pub const fn millis(self) -> i64 {
        match self {
            Self::OneHour => 3_600_000,
            Self::FourHour => 14_400_000,
        }
    }

    /// Parses a timeframe string such as `"4h"`.
    pub fn parse(value: &str) -> Result<Self, DomainError> {
        match value {
            "1h" => Ok(Self::OneHour),
            "4h" => Ok(Self::FourHour),
            other => Err(DomainError::InvalidTimeFrame(other.to_owned())),
        }
    }
}

impl FromStr for TimeFrame {
    type Err = DomainError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::parse(value)
    }
}

impl fmt::Display for TimeFrame {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl Serialize for TimeFrame {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for TimeFrame {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::parse(&value).map_err(|err| serde::de::Error::custom(err.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_supported_timeframes() {
        assert_eq!(TimeFrame::parse("1h").unwrap(), TimeFrame::OneHour);
        assert_eq!(TimeFrame::parse("4h").unwrap(), TimeFrame::FourHour);
        assert_eq!(TimeFrame::OneHour.millis(), 3_600_000);
        assert_eq!(TimeFrame::FourHour.millis(), 14_400_000);
        assert_eq!(TimeFrame::FourHour.to_string(), "4h");
    }

    #[test]
    fn rejects_unsupported_timeframes() {
        for value in ["", "2h", "1d", "4H", "4h "] {
            assert!(matches!(
                TimeFrame::parse(value),
                Err(DomainError::InvalidTimeFrame(_))
            ));
        }
    }
}
