use std::fmt;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::error::DomainError;

/// A base/quote asset pair such as `ETH/USDT`.
///
/// Asset codes are uppercase ASCII letters or digits, matching the symbols in
/// the Python configuration (`ETH/USDT`, `BTC/USDT`). Lowercase or punctuated
/// codes are rejected rather than silently normalized.
///
/// Exchange-specific formatting is kept out of the identity type. Binance
/// concatenates the parts (`ETHUSDT`), matching the Python `SymbolCfg.raw`
/// property; other exchanges can add their own formatting later.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SymbolId {
    base: String,
    quote: String,
}

impl SymbolId {
    /// Creates a symbol from its base and quote asset codes.
    pub fn new(base: impl Into<String>, quote: impl Into<String>) -> Result<Self, DomainError> {
        let base = base.into();
        let quote = quote.into();

        if !is_valid_asset_code(&base) {
            return Err(DomainError::InvalidSymbol(format!(
                "invalid base asset code: {base}"
            )));
        }
        if !is_valid_asset_code(&quote) {
            return Err(DomainError::InvalidSymbol(format!(
                "invalid quote asset code: {quote}"
            )));
        }

        Ok(Self { base, quote })
    }

    /// Parses a `BASE/QUOTE` symbol string.
    pub fn parse(value: &str) -> Result<Self, DomainError> {
        let Some((base, quote)) = value.split_once('/') else {
            return Err(DomainError::InvalidSymbol(value.to_owned()));
        };
        if quote.contains('/') {
            return Err(DomainError::InvalidSymbol(value.to_owned()));
        }

        Self::new(base, quote).map_err(|_| DomainError::InvalidSymbol(value.to_owned()))
    }

    #[must_use]
    pub fn base(&self) -> &str {
        &self.base
    }

    #[must_use]
    pub fn quote(&self) -> &str {
        &self.quote
    }

    /// Formats the symbol the way Binance does, matching Python's `raw` property.
    #[must_use]
    pub fn binance_symbol(&self) -> String {
        format!("{}{}", self.base, self.quote)
    }
}

fn is_valid_asset_code(value: &str) -> bool {
    !value.is_empty()
        && value
            .chars()
            .all(|character| character.is_ascii_uppercase() || character.is_ascii_digit())
}

impl fmt::Display for SymbolId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.base, self.quote)
    }
}

impl Serialize for SymbolId {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for SymbolId {
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
    fn parses_eth_and_btc_like_the_python_configuration() {
        let eth = SymbolId::parse("ETH/USDT").unwrap();
        assert_eq!(eth.base(), "ETH");
        assert_eq!(eth.quote(), "USDT");
        assert_eq!(eth.binance_symbol(), "ETHUSDT");
        assert_eq!(eth.to_string(), "ETH/USDT");

        let btc = SymbolId::parse("BTC/USDT").unwrap();
        assert_eq!(btc.base(), "BTC");
        assert_eq!(btc.quote(), "USDT");
        assert_eq!(btc.binance_symbol(), "BTCUSDT");
    }

    #[test]
    fn accepts_numeric_asset_codes() {
        let symbol = SymbolId::parse("1INCH/USDT").unwrap();
        assert_eq!(symbol.base(), "1INCH");
        assert_eq!(symbol.binance_symbol(), "1INCHUSDT");
    }

    #[test]
    fn rejects_malformed_symbols() {
        for value in [
            "",
            "ETHUSDT",
            "ETH/",
            "/USDT",
            "ETH/USDT/EXTRA",
            "eth/usdt",
            "ETH/US DT",
            "ETH/USDT ",
        ] {
            assert!(
                matches!(SymbolId::parse(value), Err(DomainError::InvalidSymbol(_))),
                "expected {value:?} to be rejected"
            );
        }
    }
}
