use std::{thread, time::Duration};

use reqwest::{blocking::Client, StatusCode};
use serde_json::Value;
use trading_engine_core::{Candle, Money, SymbolId, TimeFrame, Timestamp};

const TESTNET_REST_BASE: &str = "https://testnet.binance.vision/api";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_ATTEMPTS: usize = 3;

#[derive(Debug)]
pub enum MarketDataError {
    Transport(String),
    Http { status: u16, message: String },
    Malformed(String),
    Domain(trading_engine_core::DomainError),
}

impl std::fmt::Display for MarketDataError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Transport(message) => write!(f, "Testnet request failed: {message}"),
            Self::Http { status, message } => write!(f, "Binance HTTP {status}: {message}"),
            Self::Malformed(message) => write!(f, "malformed Binance response: {message}"),
            Self::Domain(error) => write!(f, "invalid market data: {error}"),
        }
    }
}

impl std::error::Error for MarketDataError {}

impl From<trading_engine_core::DomainError> for MarketDataError {
    fn from(error: trading_engine_core::DomainError) -> Self {
        Self::Domain(error)
    }
}

/// Kline values preserve Binance's close time in addition to the shared candle.
#[derive(Debug, Clone, PartialEq)]
pub struct MarketCandle {
    pub candle: Candle,
    pub close_time: Timestamp,
}

impl MarketCandle {
    /// A candle is complete only after the exchange's inclusive close time.
    #[must_use]
    pub const fn is_complete_at(&self, now: Timestamp) -> bool {
        self.close_time.as_millis() < now.as_millis()
    }
}

/// Bounded-retry public REST client for Binance Spot Testnet.
#[derive(Clone)]
pub struct BinancePublicClient {
    client: Client,
    base_url: String,
}

impl Default for BinancePublicClient {
    fn default() -> Self {
        Self::new().expect("static HTTP client configuration is valid")
    }
}

impl BinancePublicClient {
    pub fn new() -> Result<Self, MarketDataError> {
        Self::with_base_url(TESTNET_REST_BASE.to_owned())
    }

    fn with_base_url(base_url: String) -> Result<Self, MarketDataError> {
        let client = Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .user_agent(concat!("trading-engine-rs/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|e| MarketDataError::Transport(e.to_string()))?;
        Ok(Self { client, base_url })
    }

    /// Retrieves klines in exchange order; duplicate or non-chronological bars
    /// are rejected rather than silently reordered.
    pub fn klines(
        &self,
        symbol: &SymbolId,
        interval: TimeFrame,
        limit: u16,
    ) -> Result<Vec<MarketCandle>, MarketDataError> {
        self.klines_in_range(symbol, interval, None, None, limit)
    }

    /// Retrieves klines within an optional inclusive open-time range.
    pub fn klines_in_range(
        &self,
        symbol: &SymbolId,
        interval: TimeFrame,
        start_time: Option<Timestamp>,
        end_time: Option<Timestamp>,
        limit: u16,
    ) -> Result<Vec<MarketCandle>, MarketDataError> {
        if limit == 0 || limit > 1000 {
            return Err(MarketDataError::Malformed(
                "kline limit must be between 1 and 1000".to_owned(),
            ));
        }
        if matches!((start_time, end_time), (Some(start), Some(end)) if start > end) {
            return Err(MarketDataError::Malformed(
                "kline start time must not exceed end time".to_owned(),
            ));
        }
        let mut query = vec![
            ("symbol", symbol.binance_symbol()),
            ("interval", interval.as_str().to_owned()),
            ("limit", limit.to_string()),
        ];
        if let Some(start) = start_time {
            query.push(("startTime", start.as_millis().to_string()));
        }
        if let Some(end) = end_time {
            query.push(("endTime", end.as_millis().to_string()));
        }
        let response = self.get("/v3/klines", &query)?;
        parse_klines(response)
    }

    /// Retrieves recent klines and drops a currently forming final candle.
    pub fn completed_klines(
        &self,
        symbol: &SymbolId,
        interval: TimeFrame,
        limit: u16,
        now: Timestamp,
    ) -> Result<Vec<Candle>, MarketDataError> {
        Ok(self
            .klines(symbol, interval, limit)?
            .into_iter()
            .filter(|item| item.is_complete_at(now))
            .map(|item| item.candle)
            .collect())
    }

    pub fn ticker_price(&self, symbol: &SymbolId) -> Result<f64, MarketDataError> {
        let value = self.get("/v3/ticker/price", &[("symbol", symbol.binance_symbol())])?;
        let text = value
            .get("price")
            .and_then(Value::as_str)
            .ok_or_else(|| MarketDataError::Malformed("ticker price missing".to_owned()))?;
        let price: f64 = text
            .parse()
            .map_err(|_| MarketDataError::Malformed("ticker price is not numeric".to_owned()))?;
        if !price.is_finite() || price <= 0.0 {
            return Err(MarketDataError::Malformed(
                "ticker price must be finite and positive".to_owned(),
            ));
        }
        Ok(price)
    }

    /// Retrieves public Spot Testnet symbol metadata and filters.
    pub fn exchange_info(
        &self,
        symbol: Option<&SymbolId>,
    ) -> Result<crate::ExchangeInfo, MarketDataError> {
        let query = symbol
            .map(|symbol| vec![("symbol", symbol.binance_symbol())])
            .unwrap_or_default();
        let value = self.get("/v3/exchangeInfo", &query)?;
        serde_json::from_value(value).map_err(|error| MarketDataError::Malformed(error.to_string()))
    }

    fn get(&self, path: &str, query: &[(&str, String)]) -> Result<Value, MarketDataError> {
        for attempt in 0..MAX_ATTEMPTS {
            let response = self
                .client
                .get(format!("{}{path}", self.base_url))
                .query(query)
                .send();
            let response = match response {
                Ok(response) => response,
                Err(error) if attempt + 1 < MAX_ATTEMPTS => {
                    let _ = error;
                    thread::sleep(Duration::from_millis(100 * (attempt as u64 + 1)));
                    continue;
                }
                Err(error) => return Err(MarketDataError::Transport(error.to_string())),
            };
            let status = response.status();
            if status.is_success() {
                return response
                    .json()
                    .map_err(|e| MarketDataError::Malformed(e.to_string()));
            }
            let retry = status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error();
            let retry_after = response
                .headers()
                .get(reqwest::header::RETRY_AFTER)
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.parse::<u64>().ok())
                .unwrap_or(0)
                .min(2);
            let body = response.text().unwrap_or_default();
            if retry && attempt + 1 < MAX_ATTEMPTS {
                let delay = if retry_after > 0 {
                    Duration::from_secs(retry_after)
                } else {
                    Duration::from_millis(100 * (attempt as u64 + 1))
                };
                thread::sleep(delay);
                continue;
            }
            return Err(MarketDataError::Http {
                status: status.as_u16(),
                message: body.chars().take(512).collect(),
            });
        }
        unreachable!("bounded request loop returns on its final attempt")
    }
}

fn parse_klines(value: Value) -> Result<Vec<MarketCandle>, MarketDataError> {
    let rows = value
        .as_array()
        .ok_or_else(|| MarketDataError::Malformed("klines response is not an array".to_owned()))?;
    let mut candles = Vec::with_capacity(rows.len());
    for row in rows {
        let fields = row
            .as_array()
            .ok_or_else(|| MarketDataError::Malformed("kline row is not an array".to_owned()))?;
        if fields.len() < 7 {
            return Err(MarketDataError::Malformed(
                "kline row has fewer than 7 fields".to_owned(),
            ));
        }
        let open_ms = integer(&fields[0], "open time")?;
        let close_ms = integer(&fields[6], "close time")?;
        if close_ms < open_ms {
            return Err(MarketDataError::Malformed(
                "kline close time precedes open time".to_owned(),
            ));
        }
        let candle = Candle::new(
            Timestamp::from_millis(open_ms)?,
            Money::new(number(&fields[1], "open")?)?,
            Money::new(number(&fields[2], "high")?)?,
            Money::new(number(&fields[3], "low")?)?,
            Money::new(number(&fields[4], "close")?)?,
            number(&fields[5], "volume")?,
        )?;
        if candles.last().is_some_and(|previous: &MarketCandle| {
            previous.candle.open_time() >= candle.open_time()
        }) {
            return Err(MarketDataError::Malformed(
                "klines are not strictly chronological".to_owned(),
            ));
        }
        candles.push(MarketCandle {
            candle,
            close_time: Timestamp::from_millis(close_ms)?,
        });
    }
    Ok(candles)
}

fn integer(value: &Value, field: &str) -> Result<i64, MarketDataError> {
    value
        .as_i64()
        .ok_or_else(|| MarketDataError::Malformed(format!("{field} is not an integer")))
}

fn number(value: &Value, field: &str) -> Result<f64, MarketDataError> {
    let text = value
        .as_str()
        .ok_or_else(|| MarketDataError::Malformed(format!("{field} is not a string")))?;
    let result = text
        .parse::<f64>()
        .map_err(|_| MarketDataError::Malformed(format!("{field} is not numeric")))?;
    if !result.is_finite() {
        return Err(MarketDataError::Malformed(format!("{field} is not finite")));
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{Read, Write},
        net::TcpListener,
        thread,
    };

    fn ts(millis: i64) -> Timestamp {
        Timestamp::from_millis(millis).unwrap()
    }

    fn mock(response: String) -> (BinancePublicClient, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0; 2048];
            let size = stream.read(&mut request).unwrap();
            assert!(String::from_utf8_lossy(&request[..size]).starts_with("GET /"));
            stream.write_all(response.as_bytes()).unwrap();
        });
        (
            BinancePublicClient::with_base_url(format!("http://{address}")).unwrap(),
            handle,
        )
    }

    #[test]
    fn parses_validated_chronological_klines_and_drops_incomplete_tail() {
        let payload = r#"[[1000,"10","12","9","11","2",1999],[2000,"11","13","10","12","3",2999]]"#;
        let response = format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{payload}", payload.len());
        let (client, server) = mock(response);
        let symbol = SymbolId::parse("ETH/USDT").unwrap();
        let bars = client
            .completed_klines(&symbol, TimeFrame::FourHour, 2, ts(2999))
            .unwrap();
        assert_eq!(bars.len(), 1);
        assert_eq!(bars[0].close().as_f64(), 11.0);
        server.join().unwrap();
    }

    #[test]
    fn malformed_or_out_of_order_klines_fail_closed() {
        let reversed = serde_json::json!([
            [2000, "10", "12", "9", "11", "2", 2999],
            [1000, "11", "13", "10", "12", "3", 1999]
        ]);
        assert!(matches!(
            parse_klines(reversed),
            Err(MarketDataError::Malformed(_))
        ));
        let invalid_ohlc = serde_json::json!([[1000, "10", "8", "9", "11", "2", 1999]]);
        assert!(matches!(
            parse_klines(invalid_ohlc),
            Err(MarketDataError::Domain(_))
        ));
    }

    #[test]
    fn ticker_endpoint_parses_positive_price_and_testnet_route() {
        let payload = r#"{"symbol":"ETHUSDT","price":"2312.50"}"#;
        let response = format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{payload}", payload.len());
        let (client, server) = mock(response);
        let symbol = SymbolId::parse("ETH/USDT").unwrap();
        assert_eq!(client.ticker_price(&symbol).unwrap(), 2312.5);
        server.join().unwrap();
    }

    #[test]
    fn exchange_rules_are_retrievable_from_public_endpoint_without_credentials() {
        let payload = r#"{"symbols":[{"symbol":"ETHUSDT","status":"TRADING","baseAsset":"ETH","quoteAsset":"USDT","filters":[]}]}"#;
        let response = format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{payload}", payload.len());
        let (client, server) = mock(response);
        let symbol = SymbolId::parse("ETH/USDT").unwrap();
        let info = client.exchange_info(Some(&symbol)).unwrap();
        assert_eq!(info.symbols.len(), 1);
        assert_eq!(info.symbols[0].base_asset, "ETH");
        server.join().unwrap();
    }

    #[test]
    fn client_returns_non_retryable_api_errors_without_resubmitting() {
        let payload = r#"{"code":-1121,"msg":"Invalid symbol."}"#;
        let response = format!("HTTP/1.1 400 Bad Request\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{payload}", payload.len());
        let (client, server) = mock(response);
        let symbol = SymbolId::parse("ETH/USDT").unwrap();
        assert!(matches!(
            client.ticker_price(&symbol),
            Err(MarketDataError::Http { status: 400, .. })
        ));
        server.join().unwrap();
    }

    #[test]
    fn rejects_invalid_kline_limits_before_network_access() {
        let client = BinancePublicClient::with_base_url("http://127.0.0.1:1".to_owned()).unwrap();
        let symbol = SymbolId::parse("ETH/USDT").unwrap();
        assert!(matches!(
            client.klines(&symbol, TimeFrame::FourHour, 0),
            Err(MarketDataError::Malformed(_))
        ));
        assert!(matches!(
            client.klines(&symbol, TimeFrame::FourHour, 1001),
            Err(MarketDataError::Malformed(_))
        ));
        assert!(matches!(
            client.klines_in_range(
                &symbol,
                TimeFrame::FourHour,
                Some(ts(2000)),
                Some(ts(1000)),
                10
            ),
            Err(MarketDataError::Malformed(_))
        ));
    }
}
