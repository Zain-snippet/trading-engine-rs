use std::{
    collections::HashMap,
    env, fmt,
    time::{SystemTime, UNIX_EPOCH},
};

use crate::{Decimal, OrderValidationError, OrderValidator};
use hmac::{Hmac, Mac};
use reqwest::{blocking::Client, Method};
use serde::Deserialize;
use serde_json::Value;
use sha2::Sha256;
use trading_engine_core::{Side, SymbolId};

const TESTNET_REST_BASE: &str = "https://testnet.binance.vision/api";
const RECEIVE_WINDOW_MS: u64 = 5_000;
type HmacSha256 = Hmac<Sha256>;

#[derive(Debug)]
pub enum TestnetError {
    MissingCredentials,
    Transport(String),
    Http {
        status: u16,
        code: Option<i64>,
        message: String,
    },
    Malformed(String),
    InvalidInput(&'static str),
}

impl fmt::Display for TestnetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingCredentials => write!(
                f,
                "set BINANCE_TESTNET_API_KEY and BINANCE_TESTNET_API_SECRET"
            ),
            Self::Transport(message) => write!(f, "Testnet request failed: {message}"),
            Self::Http {
                status,
                code,
                message,
            } => write!(f, "Binance HTTP {status} (code {code:?}): {message}"),
            Self::Malformed(message) => write!(f, "malformed Binance response: {message}"),
            Self::InvalidInput(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for TestnetError {}

/// API credentials intentionally have no `Debug` implementation.
pub struct TestnetCredentials {
    api_key: String,
    api_secret: String,
}

impl TestnetCredentials {
    pub fn from_env() -> Result<Self, TestnetError> {
        let api_key =
            env::var("BINANCE_TESTNET_API_KEY").map_err(|_| TestnetError::MissingCredentials)?;
        let api_secret =
            env::var("BINANCE_TESTNET_API_SECRET").map_err(|_| TestnetError::MissingCredentials)?;
        Self::new(api_key, api_secret)
    }

    pub fn new(api_key: String, api_secret: String) -> Result<Self, TestnetError> {
        if api_key.trim().is_empty() || api_secret.trim().is_empty() {
            return Err(TestnetError::MissingCredentials);
        }
        Ok(Self {
            api_key,
            api_secret,
        })
    }
}

pub struct BinanceTestnetClient {
    client: Client,
    base_url: String,
    credentials: TestnetCredentials,
}

impl BinanceTestnetClient {
    pub fn from_env() -> Result<Self, TestnetError> {
        Self::new(TestnetCredentials::from_env()?)
    }

    pub fn new(credentials: TestnetCredentials) -> Result<Self, TestnetError> {
        Self::with_base_url(credentials, TESTNET_REST_BASE.to_owned())
    }

    fn with_base_url(
        credentials: TestnetCredentials,
        base_url: String,
    ) -> Result<Self, TestnetError> {
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .user_agent(concat!("trading-engine-rs/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|error| TestnetError::Transport(error.to_string()))?;
        Ok(Self {
            client,
            base_url,
            credentials,
        })
    }

    /// Signed `GET /api/v3/account`.
    pub fn account(&self) -> Result<AccountInfo, TestnetError> {
        self.request(Method::GET, "/v3/account", Vec::new())
    }

    /// Public `GET /api/v3/exchangeInfo`, retained here alongside trading metadata.
    pub fn exchange_info(&self, symbol: Option<&SymbolId>) -> Result<ExchangeInfo, TestnetError> {
        let mut request = self
            .client
            .get(format!("{}/v3/exchangeInfo", self.base_url));
        if let Some(symbol) = symbol {
            request = request.query(&[("symbol", symbol.binance_symbol())]);
        }
        let response = request
            .send()
            .map_err(|error| TestnetError::Transport(error.to_string()))?;
        let status = response.status();
        let body = response
            .text()
            .map_err(|error| TestnetError::Transport(error.to_string()))?;
        if !status.is_success() {
            return Err(http_error(status, &body));
        }
        serde_json::from_str(&body).map_err(|error| TestnetError::Malformed(error.to_string()))
    }

    /// Signed market order. `quantity` is a decimal string so filter validation
    /// can supply its exact, validated representation in M4.
    fn market_order(
        &self,
        symbol: &SymbolId,
        side: Side,
        quantity: &str,
        client_order_id: &str,
    ) -> Result<OrderResponse, TestnetError> {
        if !valid_positive_decimal(quantity) {
            return Err(TestnetError::InvalidInput(
                "quantity must be a positive decimal string",
            ));
        }
        if client_order_id.is_empty() || client_order_id.len() > 36 {
            return Err(TestnetError::InvalidInput(
                "client order ID must contain 1 to 36 characters",
            ));
        }
        self.request(
            Method::POST,
            "/v3/order",
            vec![
                ("symbol".to_owned(), symbol.binance_symbol()),
                ("side".to_owned(), side.as_str().to_ascii_uppercase()),
                ("type".to_owned(), "MARKET".to_owned()),
                ("quantity".to_owned(), quantity.to_owned()),
                ("newClientOrderId".to_owned(), client_order_id.to_owned()),
                ("newOrderRespType".to_owned(), "FULL".to_owned()),
            ],
        )
    }

    /// Validates the exact market order against cached exchange rules and
    /// current free balances immediately before submitting it to Testnet.
    #[allow(clippy::too_many_arguments)]
    pub fn market_order_validated(
        &self,
        validator: &OrderValidator,
        symbol: &SymbolId,
        side: Side,
        quantity: &Decimal,
        reference_price: &Decimal,
        free_balances: &HashMap<String, Decimal>,
        client_order_id: &str,
    ) -> Result<OrderResponse, OrderSubmitError> {
        validator
            .validate_market_order(symbol, side, *quantity, *reference_price, free_balances)
            .map_err(OrderSubmitError::Validation)?;
        self.market_order(symbol, side, &quantity.to_string(), client_order_id)
            .map_err(OrderSubmitError::Exchange)
    }

    pub fn query_order(
        &self,
        symbol: &SymbolId,
        order_id: Option<u64>,
        client_order_id: Option<&str>,
    ) -> Result<OrderResponse, TestnetError> {
        if order_id.is_some() == client_order_id.is_some() {
            return Err(TestnetError::InvalidInput(
                "provide exactly one of order ID or client order ID",
            ));
        }
        let mut params = vec![("symbol".to_owned(), symbol.binance_symbol())];
        if let Some(order_id) = order_id {
            params.push(("orderId".to_owned(), order_id.to_string()));
        }
        if let Some(client_id) = client_order_id {
            params.push(("origClientOrderId".to_owned(), client_id.to_owned()));
        }
        self.request(Method::GET, "/v3/order", params)
    }

    pub fn cancel_order(
        &self,
        symbol: &SymbolId,
        client_order_id: &str,
    ) -> Result<OrderResponse, TestnetError> {
        self.request(
            Method::DELETE,
            "/v3/order",
            vec![
                ("symbol".to_owned(), symbol.binance_symbol()),
                ("origClientOrderId".to_owned(), client_order_id.to_owned()),
            ],
        )
    }

    /// Signed `GET /api/v3/myTrades` filtered by exchange order ID.
    pub fn my_trades(
        &self,
        symbol: &SymbolId,
        order_id: u64,
    ) -> Result<Vec<BinanceTrade>, TestnetError> {
        self.request(
            Method::GET,
            "/v3/myTrades",
            vec![
                ("symbol".to_owned(), symbol.binance_symbol()),
                ("orderId".to_owned(), order_id.to_string()),
                ("limit".to_owned(), "1000".to_owned()),
            ],
        )
    }

    fn request<T: for<'de> Deserialize<'de>>(
        &self,
        method: Method,
        path: &str,
        mut params: Vec<(String, String)>,
    ) -> Result<T, TestnetError> {
        params.push(("recvWindow".to_owned(), RECEIVE_WINDOW_MS.to_string()));
        params.push(("timestamp".to_owned(), current_time_ms()?.to_string()));
        let encoded = encode_params(&params);
        let signature = sign_query(&self.credentials.api_secret, &encoded)?;
        let url = format!(
            "{}{}?{}&signature={signature}",
            self.base_url, path, encoded
        );
        let response = self
            .client
            .request(method, url)
            .header("X-MBX-APIKEY", &self.credentials.api_key)
            .send()
            .map_err(|error| TestnetError::Transport(error.to_string()))?;
        let status = response.status();
        let body = response
            .text()
            .map_err(|error| TestnetError::Transport(error.to_string()))?;
        if !status.is_success() {
            return Err(http_error(status, &body));
        }
        serde_json::from_str(&body).map_err(|error| TestnetError::Malformed(error.to_string()))
    }
}

#[derive(Debug)]
pub enum OrderSubmitError {
    Validation(OrderValidationError),
    Exchange(TestnetError),
}

impl fmt::Display for OrderSubmitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Validation(error) => write!(f, "local order validation failed: {error}"),
            Self::Exchange(error) => write!(f, "Testnet order submission failed: {error}"),
        }
    }
}

impl std::error::Error for OrderSubmitError {}

fn valid_positive_decimal(value: &str) -> bool {
    let mut dot_seen = false;
    let mut digit_seen = false;
    let mut nonzero_seen = false;
    for byte in value.bytes() {
        match byte {
            b'0'..=b'9' => {
                digit_seen = true;
                nonzero_seen |= byte != b'0';
            }
            b'.' if !dot_seen => dot_seen = true,
            _ => return false,
        }
    }
    digit_seen && nonzero_seen
}

fn http_error(status: reqwest::StatusCode, body: &str) -> TestnetError {
    let parsed: Value = serde_json::from_str(body).unwrap_or(Value::Null);
    let code = parsed.get("code").and_then(Value::as_i64);
    let message = parsed.get("msg").and_then(Value::as_str).unwrap_or(body);
    TestnetError::Http {
        status: status.as_u16(),
        code,
        message: message.chars().take(512).collect(),
    }
}

fn current_time_ms() -> Result<u64, TestnetError> {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| TestnetError::InvalidInput("system clock is before Unix epoch"))?
        .as_millis();
    u64::try_from(millis)
        .map_err(|_| TestnetError::InvalidInput("system clock is outside supported range"))
}

fn encode_params(params: &[(String, String)]) -> String {
    params
        .iter()
        .map(|(key, value)| format!("{}={}", percent_encode(key), percent_encode(value)))
        .collect::<Vec<_>>()
        .join("&")
}

fn percent_encode(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            encoded.push(char::from(byte));
        } else {
            use fmt::Write as _;
            let _ = write!(encoded, "%{byte:02X}");
        }
    }
    encoded
}

fn sign_query(secret: &str, query: &str) -> Result<String, TestnetError> {
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes())
        .map_err(|_| TestnetError::InvalidInput("invalid HMAC key"))?;
    mac.update(query.as_bytes());
    let bytes = mac.finalize().into_bytes();
    let mut result = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use fmt::Write as _;
        let _ = write!(result, "{byte:02x}");
    }
    Ok(result)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountInfo {
    #[serde(default)]
    pub balances: Vec<AssetBalance>,
    #[serde(default)]
    pub can_trade: bool,
}

impl AccountInfo {
    /// Returns the exchange-reported free amount for an asset, without a
    /// lossy floating-point conversion.
    #[must_use]
    pub fn free_balance(&self, asset: &str) -> Option<&str> {
        self.balances
            .iter()
            .find(|balance| balance.asset == asset)
            .map(|balance| balance.free.as_str())
    }
}

#[derive(Debug, Deserialize)]
pub struct AssetBalance {
    pub asset: String,
    pub free: String,
    pub locked: String,
}

#[derive(Debug, Deserialize)]
pub struct ExchangeInfo {
    #[serde(default)]
    pub symbols: Vec<ExchangeSymbol>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExchangeSymbol {
    pub symbol: String,
    pub status: String,
    pub base_asset: String,
    pub quote_asset: String,
    #[serde(default)]
    pub filters: Vec<Value>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrderStatus {
    New,
    PartiallyFilled,
    Filled,
    Canceled,
    Rejected,
    Expired,
    ExpiredInMatch,
}

impl<'de> Deserialize<'de> for OrderStatus {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let status = String::deserialize(deserializer)?;
        match status.as_str() {
            "NEW" => Ok(Self::New),
            "PARTIALLY_FILLED" => Ok(Self::PartiallyFilled),
            "FILLED" => Ok(Self::Filled),
            "CANCELED" => Ok(Self::Canceled),
            "REJECTED" => Ok(Self::Rejected),
            "EXPIRED" => Ok(Self::Expired),
            "EXPIRED_IN_MATCH" => Ok(Self::ExpiredInMatch),
            _ => Err(serde::de::Error::custom("unknown Binance order status")),
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OrderResponse {
    pub symbol: String,
    pub order_id: u64,
    pub client_order_id: String,
    pub status: OrderStatus,
    pub executed_qty: String,
    #[serde(rename = "cummulativeQuoteQty")]
    pub cumulative_quote_qty: String,
    #[serde(default)]
    pub fills: Vec<OrderFill>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OrderFill {
    pub price: String,
    pub qty: String,
    pub commission: String,
    pub commission_asset: String,
    pub trade_id: u64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BinanceTrade {
    pub id: u64,
    pub order_id: u64,
    pub price: String,
    pub qty: String,
    pub quote_qty: String,
    pub commission: String,
    pub commission_asset: String,
    pub time: u64,
    pub is_buyer: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{Read, Write},
        net::TcpListener,
        thread,
    };

    fn mock_request(body: String) -> (BinanceTestnetClient, thread::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut data = [0; 4096];
            let size = stream.read(&mut data).unwrap();
            let request = String::from_utf8_lossy(&data[..size]).to_string();
            let response = format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
            stream.write_all(response.as_bytes()).unwrap();
            request
        });
        let credentials =
            TestnetCredentials::new("public-test-key".to_owned(), "not-a-real-secret".to_owned())
                .unwrap();
        (
            BinanceTestnetClient::with_base_url(credentials, format!("http://{addr}")).unwrap(),
            handle,
        )
    }

    #[test]
    fn hmac_sha256_matches_rfc_4231_vector() {
        assert_eq!(
            sign_query(&"\x0b".repeat(20), "Hi There").unwrap(),
            "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7"
        );
    }

    #[test]
    fn request_parameter_encoding_is_canonical() {
        assert_eq!(
            encode_params(&[
                ("symbol".to_owned(), "ETHUSDT".to_owned()),
                ("note".to_owned(), "a b".to_owned())
            ]),
            "symbol=ETHUSDT&note=a%20b"
        );
    }

    #[test]
    fn account_request_is_signed_and_credentials_are_only_in_header() {
        let (client, server) = mock_request(
            r#"{"canTrade":true,"balances":[{"asset":"USDT","free":"12.3","locked":"0"}]}"#
                .to_owned(),
        );
        let account = client.account().unwrap();
        let request = server.join().unwrap();
        assert!(request.contains("GET /v3/account?"));
        assert!(request.contains("signature="));
        assert!(request
            .to_ascii_lowercase()
            .contains("x-mbx-apikey: public-test-key"));
        assert!(!request.contains("not-a-real-secret"));
        assert!(account.can_trade);
        assert_eq!(account.balances[0].free, "12.3");
    }

    #[test]
    fn market_order_uses_testnet_post_and_signed_client_id() {
        let body = r#"{"symbol":"ETHUSDT","orderId":42,"clientOrderId":"run-entry-1","status":"FILLED","executedQty":"0.1","cummulativeQuoteQty":"200","fills":[]}"#.to_owned();
        let (client, server) = mock_request(body);
        let symbol = SymbolId::parse("ETH/USDT").unwrap();
        let order = client
            .market_order(&symbol, Side::Buy, "0.1", "run-entry-1")
            .unwrap();
        let request = server.join().unwrap();
        assert!(request.starts_with("POST /v3/order?"));
        assert!(request.contains("side=BUY"));
        assert!(request.contains("type=MARKET"));
        assert!(request.contains("quantity=0.1"));
        assert!(request.contains("newClientOrderId=run-entry-1"));
        assert!(request
            .to_ascii_lowercase()
            .contains("x-mbx-apikey: public-test-key"));
        assert_eq!(order.status, OrderStatus::Filled);
    }

    #[test]
    fn market_order_rejects_bad_quantity_before_submission() {
        let credentials = TestnetCredentials::new("key".to_owned(), "secret".to_owned()).unwrap();
        let client =
            BinanceTestnetClient::with_base_url(credentials, "http://127.0.0.1:1".to_owned())
                .unwrap();
        let symbol = SymbolId::parse("ETH/USDT").unwrap();
        for quantity in ["", ".", "0", "-1", "1.2.3", "1e-4"] {
            assert!(matches!(
                client.market_order(&symbol, Side::Buy, quantity, "client-1"),
                Err(TestnetError::InvalidInput(_))
            ));
        }
    }

    #[test]
    fn validated_market_order_rejects_exchange_filter_failures_locally() {
        let metadata: ExchangeSymbol = serde_json::from_value(serde_json::json!({
            "symbol":"ETHUSDT", "status":"TRADING", "baseAsset":"ETH", "quoteAsset":"USDT",
            "filters":[
                {"filterType":"PRICE_FILTER","minPrice":"0.01","maxPrice":"1000000","tickSize":"0.01"},
                {"filterType":"LOT_SIZE","minQty":"0.001","maxQty":"1000","stepSize":"0.001"},
                {"filterType":"MIN_NOTIONAL","minNotional":"5","applyToMarket":true}
            ]
        })).unwrap();
        let validator = OrderValidator::new([&metadata]).unwrap();
        let credentials = TestnetCredentials::new("key".to_owned(), "secret".to_owned()).unwrap();
        let client =
            BinanceTestnetClient::with_base_url(credentials, "http://127.0.0.1:1".to_owned())
                .unwrap();
        let balances = HashMap::from([("USDT".to_owned(), Decimal::parse("100").unwrap())]);
        let error = client
            .market_order_validated(
                &validator,
                &SymbolId::parse("ETH/USDT").unwrap(),
                Side::Buy,
                &Decimal::parse("0.001").unwrap(),
                &Decimal::parse("2000").unwrap(),
                &balances,
                "entry-1",
            )
            .unwrap_err();
        assert!(matches!(
            error,
            OrderSubmitError::Validation(OrderValidationError::NotionalBelowMinimum)
        ));
    }

    #[test]
    fn query_and_cancel_use_client_order_id_without_resubmission() {
        let body = r#"{"symbol":"ETHUSDT","orderId":42,"clientOrderId":"entry-1","status":"NEW","executedQty":"0","cummulativeQuoteQty":"0","fills":[]}"#;
        let (client, server) = mock_request(body.to_owned());
        let symbol = SymbolId::parse("ETH/USDT").unwrap();
        assert_eq!(
            client
                .query_order(&symbol, None, Some("entry-1"))
                .unwrap()
                .status,
            OrderStatus::New
        );
        assert!(server.join().unwrap().contains("origClientOrderId=entry-1"));

        let (client, server) = mock_request(body.to_owned());
        assert_eq!(
            client.cancel_order(&symbol, "entry-1").unwrap().order_id,
            42
        );
        assert!(server.join().unwrap().starts_with("DELETE /v3/order?"));
    }

    #[test]
    fn my_trades_returns_exchange_fill_commission_and_identity_fields() {
        let body = r#"[{"symbol":"ETHUSDT","id":7,"orderId":44,"price":"2000","qty":"0.1","quoteQty":"200","commission":"0.0001","commissionAsset":"ETH","time":123,"isBuyer":true,"isMaker":false,"isBestMatch":true}]"#;
        let (client, server) = mock_request(body.to_owned());
        let trades = client
            .my_trades(&SymbolId::parse("ETH/USDT").unwrap(), 44)
            .unwrap();
        let request = server.join().unwrap();
        assert!(request.contains("GET /v3/myTrades?"));
        assert!(request.contains("orderId=44"));
        assert_eq!(trades[0].id, 7);
        assert_eq!(trades[0].commission_asset, "ETH");
        assert!(trades[0].is_buyer);
    }

    #[test]
    fn account_free_balance_is_exact_exchange_text() {
        let account: AccountInfo = serde_json::from_str(
            r#"{"canTrade":true,"balances":[{"asset":"USDT","free":"123.4500","locked":"0"}]}"#,
        )
        .unwrap();
        assert_eq!(account.free_balance("USDT"), Some("123.4500"));
        assert_eq!(account.free_balance("ETH"), None);
    }

    #[test]
    fn exchange_info_remains_a_public_testnet_request() {
        let body = r#"{"symbols":[{"symbol":"ETHUSDT","status":"TRADING","baseAsset":"ETH","quoteAsset":"USDT","filters":[]}]}"#;
        let (client, server) = mock_request(body.to_owned());
        let info = client
            .exchange_info(Some(&SymbolId::parse("ETH/USDT").unwrap()))
            .unwrap();
        let request = server.join().unwrap();
        assert!(request.starts_with("GET /v3/exchangeInfo?symbol=ETHUSDT"));
        assert!(!request.contains("signature="));
        assert_eq!(info.symbols[0].base_asset, "ETH");
    }

    #[test]
    fn order_models_parse_actual_fill_fields_and_reject_unknown_status() {
        let response: OrderResponse = serde_json::from_str(r#"{"symbol":"ETHUSDT","orderId":42,"clientOrderId":"abc","status":"PARTIALLY_FILLED","executedQty":"0.2","cummulativeQuoteQty":"400","fills":[{"price":"2000","qty":"0.2","commission":"0.2","commissionAsset":"USDT","tradeId":9}]}"#).unwrap();
        assert_eq!(response.status, OrderStatus::PartiallyFilled);
        assert_eq!(response.fills[0].trade_id, 9);
        assert_eq!(response.cumulative_quote_qty, "400");
        assert!(serde_json::from_str::<OrderStatus>("\"FUTURE_STATUS\"").is_err());
    }
}
