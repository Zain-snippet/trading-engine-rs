use std::{cmp::Ordering, collections::HashMap, fmt, str::FromStr};

use serde_json::Value;
use trading_engine_core::{Side, SymbolId};

use crate::ExchangeSymbol;

/// Exact base-10 number used at exchange boundaries. Precision is limited to
/// 18 fractional digits and 38 total significant digits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Decimal {
    coefficient: i128,
    scale: u32,
}

impl Default for Decimal {
    fn default() -> Self {
        Self::zero()
    }
}

impl Decimal {
    pub fn parse(value: &str) -> Result<Self, OrderValidationError> {
        value.parse()
    }

    pub const fn zero() -> Self {
        Self {
            coefficient: 0,
            scale: 0,
        }
    }

    pub fn is_positive(self) -> bool {
        self.coefficient > 0
    }

    fn align(self, other: Self) -> Result<(i128, i128), OrderValidationError> {
        let scale = self.scale.max(other.scale);
        let left = self
            .coefficient
            .checked_mul(pow10(scale - self.scale)?)
            .ok_or(OrderValidationError::DecimalOverflow)?;
        let right = other
            .coefficient
            .checked_mul(pow10(scale - other.scale)?)
            .ok_or(OrderValidationError::DecimalOverflow)?;
        Ok((left, right))
    }

    pub(crate) fn checked_add(self, other: Self) -> Result<Self, OrderValidationError> {
        let scale = self.scale.max(other.scale);
        let (left, right) = self.align(other)?;
        let coefficient = left
            .checked_add(right)
            .ok_or(OrderValidationError::DecimalOverflow)?;
        Ok(Self::normalized(coefficient, scale))
    }

    pub(crate) fn checked_cmp(self, other: Self) -> Result<Ordering, OrderValidationError> {
        let (left, right) = self.align(other)?;
        Ok(left.cmp(&right))
    }

    pub(crate) fn checked_mul(self, other: Self) -> Result<Self, OrderValidationError> {
        let coefficient = self
            .coefficient
            .checked_mul(other.coefficient)
            .ok_or(OrderValidationError::DecimalOverflow)?;
        let scale = self
            .scale
            .checked_add(other.scale)
            .ok_or(OrderValidationError::DecimalOverflow)?;
        if scale > 36 {
            return Err(OrderValidationError::DecimalPrecision);
        }
        Ok(Self::normalized(coefficient, scale))
    }

    fn divisible_by(self, step: Self) -> Result<bool, OrderValidationError> {
        if step.coefficient == 0 {
            return Ok(true);
        }
        let (value, step) = self.align(step)?;
        Ok(value % step == 0)
    }

    fn normalized(mut coefficient: i128, mut scale: u32) -> Self {
        while scale > 0 && coefficient % 10 == 0 {
            coefficient /= 10;
            scale -= 1;
        }
        Self { coefficient, scale }
    }
}

fn pow10(scale: u32) -> Result<i128, OrderValidationError> {
    10_i128
        .checked_pow(scale)
        .ok_or(OrderValidationError::DecimalOverflow)
}

impl FromStr for Decimal {
    type Err = OrderValidationError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        if value.is_empty() || value.starts_with(['+', '-']) || value.contains(['e', 'E']) {
            return Err(OrderValidationError::InvalidDecimal(value.to_owned()));
        }
        let mut parts = value.split('.');
        let whole = parts.next().unwrap_or_default();
        let fraction = parts.next().unwrap_or_default();
        if parts.next().is_some()
            || (whole.is_empty() && fraction.is_empty())
            || !whole.bytes().all(|b| b.is_ascii_digit())
            || !fraction.bytes().all(|b| b.is_ascii_digit())
            || fraction.len() > 18
        {
            return Err(OrderValidationError::InvalidDecimal(value.to_owned()));
        }
        let scale =
            u32::try_from(fraction.len()).map_err(|_| OrderValidationError::DecimalPrecision)?;
        let digits = format!("{}{}", if whole.is_empty() { "0" } else { whole }, fraction);
        if digits.len() > 38 {
            return Err(OrderValidationError::DecimalPrecision);
        }
        let coefficient = digits
            .parse::<i128>()
            .map_err(|_| OrderValidationError::DecimalPrecision)?;
        Ok(Self::normalized(coefficient, scale))
    }
}

impl fmt::Display for Decimal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.scale == 0 {
            return write!(f, "{}", self.coefficient);
        }
        let negative = self.coefficient < 0;
        let digits = self.coefficient.unsigned_abs().to_string();
        let scale = self.scale as usize;
        let padded = format!("{:0>width$}", digits, width = scale + 1);
        let split = padded.len() - scale;
        if negative {
            f.write_str("-")?;
        }
        write!(f, "{}.{}", &padded[..split], &padded[split..])
    }
}

#[derive(Debug)]
pub enum OrderValidationError {
    InvalidDecimal(String),
    DecimalPrecision,
    DecimalOverflow,
    UnsupportedSymbol(String),
    SymbolNotTrading(String),
    AssetMismatch(String),
    MissingExchangeMetadata(String),
    MissingFilter(&'static str),
    InvalidFilter(&'static str),
    InvalidQuantity,
    InvalidReferencePrice,
    QuantityBelowMinimum,
    QuantityAboveMaximum,
    QuantityStepMismatch,
    PriceBelowMinimum,
    PriceAboveMaximum,
    PriceTickMismatch,
    NotionalBelowMinimum,
    NotionalAboveMaximum,
    InsufficientBalance {
        asset: String,
        available: String,
        required: String,
    },
}

impl fmt::Display for OrderValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidDecimal(value) => write!(f, "invalid decimal: {value}"),
            Self::DecimalPrecision => f.write_str("decimal precision is outside supported bounds"),
            Self::DecimalOverflow => f.write_str("decimal arithmetic overflow"),
            Self::UnsupportedSymbol(symbol) => {
                write!(f, "symbol {symbol} is not in exchange metadata")
            }
            Self::SymbolNotTrading(symbol) => write!(f, "symbol {symbol} is not trading"),
            Self::AssetMismatch(symbol) => {
                write!(f, "exchange assets do not match configured symbol {symbol}")
            }
            Self::MissingExchangeMetadata(symbol) => {
                write!(f, "exchange metadata is missing for {symbol}")
            }
            Self::MissingFilter(filter) => {
                write!(f, "required exchange filter {filter} is missing")
            }
            Self::InvalidFilter(filter) => write!(f, "exchange filter {filter} is malformed"),
            Self::InvalidQuantity => f.write_str("order quantity must be positive"),
            Self::InvalidReferencePrice => f.write_str("market reference price must be positive"),
            Self::QuantityBelowMinimum => f.write_str("quantity is below exchange minimum"),
            Self::QuantityAboveMaximum => f.write_str("quantity exceeds exchange maximum"),
            Self::QuantityStepMismatch => {
                f.write_str("quantity does not align to exchange step size")
            }
            Self::PriceBelowMinimum => f.write_str("price is below exchange minimum"),
            Self::PriceAboveMaximum => f.write_str("price exceeds exchange maximum"),
            Self::PriceTickMismatch => f.write_str("price does not align to exchange tick size"),
            Self::NotionalBelowMinimum => f.write_str("order notional is below exchange minimum"),
            Self::NotionalAboveMaximum => f.write_str("order notional exceeds exchange maximum"),
            Self::InsufficientBalance {
                asset,
                available,
                required,
            } => write!(
                f,
                "insufficient {asset} balance: {available} available, {required} required"
            ),
        }
    }
}

impl std::error::Error for OrderValidationError {}

#[derive(Debug, Clone, Copy)]
pub struct PriceFilter {
    pub min_price: Decimal,
    pub max_price: Decimal,
    pub tick_size: Decimal,
}

#[derive(Debug, Clone, Copy)]
pub struct QuantityFilter {
    pub min_qty: Decimal,
    pub max_qty: Decimal,
    pub step_size: Decimal,
}

#[derive(Debug, Clone)]
struct NotionalFilter {
    minimum: Decimal,
    maximum: Option<Decimal>,
    min_applies_to_market: bool,
    max_applies_to_market: bool,
}

/// Parsed trading constraints for one exchange symbol.
#[derive(Debug, Clone)]
pub struct ExchangeRules {
    symbol: String,
    base_asset: String,
    quote_asset: String,
    status: String,
    price: PriceFilter,
    lot: QuantityFilter,
    market_lot: Option<QuantityFilter>,
    notional: Option<NotionalFilter>,
}

impl ExchangeRules {
    pub fn from_exchange_symbol(symbol: &ExchangeSymbol) -> Result<Self, OrderValidationError> {
        let mut price = None;
        let mut lot = None;
        let mut market_lot = None;
        let mut notional = None;
        for filter in &symbol.filters {
            let kind = filter
                .get("filterType")
                .and_then(Value::as_str)
                .ok_or(OrderValidationError::InvalidFilter("filterType"))?;
            match kind {
                "PRICE_FILTER" => {
                    price = Some(PriceFilter {
                        min_price: filter_decimal(filter, "minPrice", "PRICE_FILTER")?,
                        max_price: filter_decimal(filter, "maxPrice", "PRICE_FILTER")?,
                        tick_size: filter_decimal(filter, "tickSize", "PRICE_FILTER")?,
                    })
                }
                "LOT_SIZE" => lot = Some(quantity_filter(filter, "LOT_SIZE")?),
                "MARKET_LOT_SIZE" => market_lot = Some(quantity_filter(filter, "MARKET_LOT_SIZE")?),
                "MIN_NOTIONAL" => {
                    if notional.is_some() {
                        return Err(OrderValidationError::InvalidFilter("MIN_NOTIONAL"));
                    }
                    notional = Some(NotionalFilter {
                        minimum: filter_decimal(filter, "minNotional", "MIN_NOTIONAL")?,
                        maximum: None,
                        min_applies_to_market: filter_bool(
                            filter,
                            "applyToMarket",
                            "MIN_NOTIONAL",
                        )?,
                        max_applies_to_market: false,
                    });
                }
                "NOTIONAL" => {
                    if notional.is_some() {
                        return Err(OrderValidationError::InvalidFilter("NOTIONAL"));
                    }
                    notional = Some(NotionalFilter {
                        minimum: filter_decimal(filter, "minNotional", "NOTIONAL")?,
                        maximum: Some(filter_decimal(filter, "maxNotional", "NOTIONAL")?),
                        min_applies_to_market: filter_bool(filter, "applyMinToMarket", "NOTIONAL")?,
                        max_applies_to_market: filter_bool(filter, "applyMaxToMarket", "NOTIONAL")?,
                    });
                }
                _ => {}
            }
        }
        Ok(Self {
            symbol: symbol.symbol.clone(),
            base_asset: symbol.base_asset.clone(),
            quote_asset: symbol.quote_asset.clone(),
            status: symbol.status.clone(),
            price: price.ok_or(OrderValidationError::MissingFilter("PRICE_FILTER"))?,
            lot: lot.ok_or(OrderValidationError::MissingFilter("LOT_SIZE"))?,
            market_lot,
            notional,
        })
    }

    pub fn validate_price(&self, price: Decimal) -> Result<(), OrderValidationError> {
        if active(self.price.min_price) && compare(price, self.price.min_price)? == Ordering::Less {
            return Err(OrderValidationError::PriceBelowMinimum);
        }
        if active(self.price.max_price)
            && compare(price, self.price.max_price)? == Ordering::Greater
        {
            return Err(OrderValidationError::PriceAboveMaximum);
        }
        if active(self.price.tick_size) && !price.divisible_by(self.price.tick_size)? {
            return Err(OrderValidationError::PriceTickMismatch);
        }
        Ok(())
    }
}

fn quantity_filter(
    filter: &Value,
    kind: &'static str,
) -> Result<QuantityFilter, OrderValidationError> {
    Ok(QuantityFilter {
        min_qty: filter_decimal(filter, "minQty", kind)?,
        max_qty: filter_decimal(filter, "maxQty", kind)?,
        step_size: filter_decimal(filter, "stepSize", kind)?,
    })
}

fn filter_decimal(
    filter: &Value,
    field: &'static str,
    kind: &'static str,
) -> Result<Decimal, OrderValidationError> {
    let text = filter
        .get(field)
        .and_then(Value::as_str)
        .ok_or(OrderValidationError::InvalidFilter(kind))?;
    Decimal::parse(text).map_err(|_| OrderValidationError::InvalidFilter(kind))
}

fn filter_bool(
    filter: &Value,
    field: &'static str,
    kind: &'static str,
) -> Result<bool, OrderValidationError> {
    filter
        .get(field)
        .and_then(Value::as_bool)
        .ok_or(OrderValidationError::InvalidFilter(kind))
}

fn active(value: Decimal) -> bool {
    value.is_positive()
}

fn compare(left: Decimal, right: Decimal) -> Result<Ordering, OrderValidationError> {
    let (left, right) = left.align(right)?;
    Ok(left.cmp(&right))
}

/// Validates market orders locally against exchange metadata and free balances.
#[derive(Debug, Default)]
pub struct OrderValidator {
    rules: HashMap<String, ExchangeRules>,
}

impl OrderValidator {
    pub fn new<'a>(
        symbols: impl IntoIterator<Item = &'a ExchangeSymbol>,
    ) -> Result<Self, OrderValidationError> {
        let mut rules = HashMap::new();
        for symbol in symbols {
            let parsed = ExchangeRules::from_exchange_symbol(symbol)?;
            rules.insert(parsed.symbol.clone(), parsed);
        }
        Ok(Self { rules })
    }

    pub fn add_symbol(&mut self, symbol: &ExchangeSymbol) -> Result<(), OrderValidationError> {
        let parsed = ExchangeRules::from_exchange_symbol(symbol)?;
        self.rules.insert(parsed.symbol.clone(), parsed);
        Ok(())
    }

    pub fn validate_market_order(
        &self,
        symbol: &SymbolId,
        side: Side,
        quantity: Decimal,
        reference_price: Decimal,
        free_balances: &HashMap<String, Decimal>,
    ) -> Result<(), OrderValidationError> {
        if !quantity.is_positive() {
            return Err(OrderValidationError::InvalidQuantity);
        }
        if !reference_price.is_positive() {
            return Err(OrderValidationError::InvalidReferencePrice);
        }
        let key = symbol.binance_symbol();
        let rules = self
            .rules
            .get(&key)
            .ok_or_else(|| OrderValidationError::UnsupportedSymbol(key.clone()))?;
        if rules.status != "TRADING" {
            return Err(OrderValidationError::SymbolNotTrading(key));
        }
        if rules.base_asset != symbol.base() || rules.quote_asset != symbol.quote() {
            return Err(OrderValidationError::AssetMismatch(key));
        }
        let notional = quantity.checked_mul(reference_price)?;
        validate_quantity(quantity, rules.lot)?;
        if let Some(market_lot) = rules.market_lot {
            validate_quantity(quantity, market_lot)?;
        }
        if let Some(filter) = &rules.notional {
            if filter.min_applies_to_market && compare(notional, filter.minimum)? == Ordering::Less
            {
                return Err(OrderValidationError::NotionalBelowMinimum);
            }
            if filter.max_applies_to_market {
                if let Some(maximum) = filter.maximum {
                    if compare(notional, maximum)? == Ordering::Greater {
                        return Err(OrderValidationError::NotionalAboveMaximum);
                    }
                }
            }
        }
        let (asset, available, required) = match side {
            Side::Buy => (
                symbol.quote(),
                free_balances
                    .get(symbol.quote())
                    .copied()
                    .unwrap_or_default(),
                notional,
            ),
            Side::Sell => (
                symbol.base(),
                free_balances
                    .get(symbol.base())
                    .copied()
                    .unwrap_or_default(),
                quantity,
            ),
        };
        if compare(available, required)? == Ordering::Less {
            return Err(OrderValidationError::InsufficientBalance {
                asset: asset.to_owned(),
                available: available.to_string(),
                required: required.to_string(),
            });
        }
        Ok(())
    }
}

fn validate_quantity(
    quantity: Decimal,
    filter: QuantityFilter,
) -> Result<(), OrderValidationError> {
    if active(filter.min_qty) && compare(quantity, filter.min_qty)? == Ordering::Less {
        return Err(OrderValidationError::QuantityBelowMinimum);
    }
    if active(filter.max_qty) && compare(quantity, filter.max_qty)? == Ordering::Greater {
        return Err(OrderValidationError::QuantityAboveMaximum);
    }
    if active(filter.step_size) && !quantity.divisible_by(filter.step_size)? {
        return Err(OrderValidationError::QuantityStepMismatch);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn symbol_metadata(status: &str) -> ExchangeSymbol {
        serde_json::from_value(serde_json::json!({
            "symbol": "ETHUSDT", "status": status, "baseAsset": "ETH", "quoteAsset": "USDT",
            "filters": [
                {"filterType":"PRICE_FILTER","minPrice":"0.01","maxPrice":"1000000","tickSize":"0.01"},
                {"filterType":"LOT_SIZE","minQty":"0.001","maxQty":"1000","stepSize":"0.001"},
                {"filterType":"MARKET_LOT_SIZE","minQty":"0.001","maxQty":"100","stepSize":"0.001"},
                {"filterType":"NOTIONAL","minNotional":"5","maxNotional":"1000000","applyMinToMarket":true,"applyMaxToMarket":true}
            ]
        })).unwrap()
    }

    fn decimal(value: &str) -> Decimal {
        Decimal::parse(value).unwrap()
    }
    fn symbol() -> SymbolId {
        SymbolId::parse("ETH/USDT").unwrap()
    }
    fn balances(usdt: &str, eth: &str) -> HashMap<String, Decimal> {
        HashMap::from([
            ("USDT".to_owned(), decimal(usdt)),
            ("ETH".to_owned(), decimal(eth)),
        ])
    }
    fn validator() -> OrderValidator {
        OrderValidator::new([&symbol_metadata("TRADING")]).unwrap()
    }

    #[test]
    fn decimal_preserves_exact_values_and_normalizes_api_text() {
        assert_eq!(decimal("0.0100"), decimal("0.01"));
        assert_eq!(decimal("0.0100").to_string(), "0.01");
        assert_eq!(decimal("0.00000001").to_string(), "0.00000001");
        assert!(Decimal::parse("1.0000000000000000001").is_err());
    }

    #[test]
    fn valid_market_orders_obey_steps_notional_and_free_balance() {
        let validator = validator();
        assert!(validator
            .validate_market_order(
                &symbol(),
                Side::Buy,
                decimal("0.01"),
                decimal("2000"),
                &balances("25", "0")
            )
            .is_ok());
        assert!(validator
            .validate_market_order(
                &symbol(),
                Side::Sell,
                decimal("0.01"),
                decimal("2000"),
                &balances("0", "0.01")
            )
            .is_ok());
    }

    #[test]
    fn rejects_invalid_quantity_price_notional_and_balance() {
        let validator = validator();
        assert!(matches!(
            validator.validate_market_order(
                &symbol(),
                Side::Buy,
                Decimal::zero(),
                decimal("2000"),
                &balances("1000", "1")
            ),
            Err(OrderValidationError::InvalidQuantity)
        ));
        assert!(matches!(
            validator.validate_market_order(
                &symbol(),
                Side::Buy,
                decimal("0.0005"),
                decimal("2000"),
                &balances("1000", "1")
            ),
            Err(OrderValidationError::QuantityBelowMinimum)
        ));
        assert!(matches!(
            validator.validate_market_order(
                &symbol(),
                Side::Buy,
                decimal("0.0015"),
                decimal("2000"),
                &balances("1000", "1")
            ),
            Err(OrderValidationError::QuantityStepMismatch)
        ));
        assert!(matches!(
            validator.validate_market_order(
                &symbol(),
                Side::Buy,
                decimal("0.001"),
                decimal("2000"),
                &balances("1000", "1")
            ),
            Err(OrderValidationError::NotionalBelowMinimum)
        ));
        assert!(matches!(
            validator.validate_market_order(
                &symbol(),
                Side::Buy,
                decimal("0.1"),
                decimal("2000"),
                &balances("10", "1")
            ),
            Err(OrderValidationError::InsufficientBalance { .. })
        ));
        assert!(matches!(
            validator.validate_market_order(
                &symbol(),
                Side::Sell,
                decimal("0.1"),
                decimal("2000"),
                &balances("1000", "0.05")
            ),
            Err(OrderValidationError::InsufficientBalance { .. })
        ));
        assert!(matches!(
            validator.validate_market_order(
                &symbol(),
                Side::Buy,
                decimal("101"),
                decimal("2000"),
                &balances("500000", "1")
            ),
            Err(OrderValidationError::QuantityAboveMaximum)
        ));
        assert!(matches!(
            validator.validate_market_order(
                &symbol(),
                Side::Buy,
                decimal("0.1"),
                Decimal::zero(),
                &balances("500000", "1")
            ),
            Err(OrderValidationError::InvalidReferencePrice)
        ));
    }

    #[test]
    fn checks_price_filter_separately_for_price_bearing_orders() {
        let metadata = symbol_metadata("TRADING");
        let rules = ExchangeRules::from_exchange_symbol(&metadata).unwrap();
        assert!(rules.validate_price(decimal("2000.01")).is_ok());
        assert!(matches!(
            rules.validate_price(decimal("2000.001")),
            Err(OrderValidationError::PriceTickMismatch)
        ));
        assert!(matches!(
            rules.validate_price(decimal("0.001")),
            Err(OrderValidationError::PriceBelowMinimum)
        ));
    }

    #[test]
    fn honors_min_notional_apply_to_market_flag() {
        let mut metadata = symbol_metadata("TRADING");
        metadata
            .filters
            .retain(|filter| filter["filterType"] != "NOTIONAL");
        metadata.filters.push(serde_json::json!({
            "filterType":"MIN_NOTIONAL", "minNotional":"5", "applyToMarket":true, "avgPriceMins":5
        }));
        let validator = OrderValidator::new([&metadata]).unwrap();
        assert!(matches!(
            validator.validate_market_order(
                &symbol(),
                Side::Buy,
                decimal("0.001"),
                decimal("2000"),
                &balances("100", "1")
            ),
            Err(OrderValidationError::NotionalBelowMinimum)
        ));

        metadata
            .filters
            .retain(|filter| filter["filterType"] != "MIN_NOTIONAL");
        metadata.filters.push(serde_json::json!({
            "filterType":"MIN_NOTIONAL", "minNotional":"5", "applyToMarket":false, "avgPriceMins":5
        }));
        let validator = OrderValidator::new([&metadata]).unwrap();
        assert!(validator
            .validate_market_order(
                &symbol(),
                Side::Buy,
                decimal("0.001"),
                decimal("2000"),
                &balances("100", "1")
            )
            .is_ok());
    }

    #[test]
    fn fails_closed_for_missing_symbol_filters_assets_and_disabled_symbols() {
        let validator = validator();
        assert!(matches!(
            validator.validate_market_order(
                &SymbolId::parse("BTC/USDT").unwrap(),
                Side::Buy,
                decimal("0.1"),
                decimal("100"),
                &balances("1000", "1")
            ),
            Err(OrderValidationError::UnsupportedSymbol(_))
        ));
        let mut disabled = OrderValidator::default();
        disabled.add_symbol(&symbol_metadata("BREAK")).unwrap();
        assert!(matches!(
            disabled.validate_market_order(
                &symbol(),
                Side::Buy,
                decimal("0.01"),
                decimal("2000"),
                &balances("100", "1")
            ),
            Err(OrderValidationError::SymbolNotTrading(_))
        ));
        let mut missing = symbol_metadata("TRADING");
        missing.quote_asset = "USDC".to_owned();
        let mut validator = OrderValidator::default();
        validator.add_symbol(&missing).unwrap();
        assert!(matches!(
            validator.validate_market_order(
                &symbol(),
                Side::Buy,
                decimal("0.01"),
                decimal("2000"),
                &balances("100", "1")
            ),
            Err(OrderValidationError::AssetMismatch(_))
        ));
        let missing_filters: ExchangeSymbol = serde_json::from_value(serde_json::json!({"symbol":"ETHUSDT","status":"TRADING","baseAsset":"ETH","quoteAsset":"USDT","filters":[]})).unwrap();
        assert!(matches!(
            ExchangeRules::from_exchange_symbol(&missing_filters),
            Err(OrderValidationError::MissingFilter(_))
        ));
    }
}
