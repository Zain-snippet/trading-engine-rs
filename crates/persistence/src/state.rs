use std::{
    collections::HashMap,
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use serde::{Deserialize, Serialize};
use trading_engine_risk::DailyRiskState;

use crate::PersistenceError;

/// A completed trade entry persisted in the `closed` list, matching Python's
/// `_close_from_fill` journal append.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClosedTradeRecord {
    pub symbol: String,
    pub entry_time: String,
    pub exit_time: String,
    pub entry: f64,
    pub exit: f64,
    pub qty: f64,
    pub pnl: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub r: Option<f64>,
    pub reason: String,
}

/// An open long position stored in the `positions` map, matching Python `state["positions"][sym]`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PersistedPosition {
    pub qty: f64,
    pub entry: f64,
    pub stop: f64,
    pub atr: f64,
    pub entry_time: String,
    pub candle: String,
    pub entry_fee: f64,
    pub client_id: String,
    pub order_id: String,
    pub risk_quote: f64,
}

/// A fill record inside `sim["orders"]`, matching Python `SimBroker._fill`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PersistedFill {
    pub order_id: String,
    pub client_id: String,
    pub qty: f64,
    pub price: f64,
    pub fee_quote: f64,
    pub status: String,
}

/// Kind of pending order that survived a crash, matching Python `state["pending"][sym]["kind"]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PendingOrderKind {
    Entry,
    Exit,
}

/// A pending order that was in-flight when the process last stopped.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PendingOrder {
    pub kind: PendingOrderKind,
    pub client_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub atr: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub candle: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// Sim-broker sub-state stored under `state["sim"]`, matching Python `SimBroker.__init__`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SimBrokerState {
    pub balances: HashMap<String, f64>,
    #[serde(default)]
    pub orders: HashMap<String, PersistedFill>,
}

/// Exchange order state stored separately from the simulator's full-fill model.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ExchangeState {
    #[serde(default)]
    pub orders: HashMap<String, PersistedExchangeOrder>,
    #[serde(default)]
    pub discrepancies: Vec<ReconciliationDiscrepancy>,
    /// Latest exchange open orders not backed by an active local intent.
    #[serde(default)]
    pub untracked_open_orders: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReconciliationDiscrepancy {
    pub symbol: String,
    pub local_quantity: String,
    pub exchange_quantity: String,
    pub reason: String,
    pub observed_at_ms: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PersistedOrderStatus {
    Prepared,
    Submitting,
    Unknown,
    New,
    PartiallyFilled,
    Filled,
    Canceled,
    Rejected,
    Expired,
    ExpiredInMatch,
}

/// Durable intent/order/fill metadata for recovery by deterministic client ID.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PersistedExchangeOrder {
    pub symbol: String,
    pub side: String,
    pub client_order_id: String,
    pub requested_qty: String,
    pub status: PersistedOrderStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exchange_order_id: Option<String>,
    pub executed_qty: String,
    pub cumulative_quote_qty: String,
    #[serde(default)]
    pub fills: HashMap<String, PersistedExchangeFill>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signal_candle: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub atr: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop_price: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    pub created_at_ms: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_reconciled_at_ms: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub discrepancy: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PersistedExchangeFill {
    pub trade_id: String,
    pub qty: String,
    pub price: String,
    pub quote_qty: String,
    pub commission: String,
    pub commission_asset: String,
    pub time_ms: u64,
}

impl SimBrokerState {
    #[must_use]
    pub fn new_with_quote(quote: impl Into<String>, initial_usdt: f64) -> Self {
        let mut balances = HashMap::new();
        balances.insert(quote.into(), initial_usdt);
        Self {
            balances,
            orders: HashMap::new(),
        }
    }
}

/// Full bot state matching Python `State` defaults in `paperbot/state.py`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct BotState {
    pub positions: HashMap<String, PersistedPosition>,
    pub pending: HashMap<String, PendingOrder>,
    pub last_candle: HashMap<String, String>,
    #[serde(flatten)]
    pub risk: DailyRiskState,
    pub halted: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub started_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sim: Option<SimBrokerState>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exchange: Option<ExchangeState>,
    #[serde(default)]
    pub closed: Vec<ClosedTradeRecord>,
}

/// Single-writer JSON state store. Saves stage and flush a unique sibling file,
/// then replace the destination atomically where the platform supports it.
#[derive(Debug)]
pub struct StateStore {
    path: PathBuf,
    pub state: BotState,
}

impl StateStore {
    /// Opens or creates the state file. Missing file starts from defaults.
    pub fn open(path: impl Into<PathBuf>) -> Result<Self, PersistenceError> {
        let path = path.into();
        let state = if path.exists() {
            let bytes = fs::read(&path)?;
            serde_json::from_slice(&bytes)?
        } else {
            BotState::default()
        };
        Ok(Self { path, state })
    }

    /// Atomically writes the state to disk: write temp file in same dir, then rename.
    pub fn save(&self) -> Result<(), PersistenceError> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }

        let dir = self.path.parent().unwrap_or(Path::new("."));
        let json = serde_json::to_vec_pretty(&self.state)?;
        let tmp_path = write_unique_temp(dir, &json)?;
        let result = replace_file(&tmp_path, &self.path);
        if result.is_err() {
            let _ = fs::remove_file(&tmp_path);
        }
        result
    }

    /// Convenience: mutate state and immediately save.
    pub fn update<F>(&mut self, mutate: F) -> Result<(), PersistenceError>
    where
        F: FnOnce(&mut BotState),
    {
        mutate(&mut self.state);
        self.save()
    }

    /// Durably records an exchange intent before a caller sends the POST.
    /// Returns `false` when the client order ID already exists.
    pub fn record_exchange_intent(
        &mut self,
        order: PersistedExchangeOrder,
    ) -> Result<bool, PersistenceError> {
        let exchange = self
            .state
            .exchange
            .get_or_insert_with(ExchangeState::default);
        if exchange.orders.contains_key(&order.client_order_id) {
            return Ok(false);
        }
        exchange.orders.insert(order.client_order_id.clone(), order);
        self.save()?;
        Ok(true)
    }

    /// Mutates one durable order record and saves it atomically.
    pub fn update_exchange_order<F>(
        &mut self,
        client_order_id: &str,
        mutate: F,
    ) -> Result<(), PersistenceError>
    where
        F: FnOnce(&mut PersistedExchangeOrder),
    {
        let order = self
            .state
            .exchange
            .as_mut()
            .and_then(|exchange| exchange.orders.get_mut(client_order_id))
            .ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    "exchange order intent not found",
                )
            })?;
        mutate(order);
        self.save()
    }

    pub fn path(&self) -> &PathBuf {
        &self.path
    }
}

static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

fn write_unique_temp(dir: &Path, contents: &[u8]) -> Result<PathBuf, PersistenceError> {
    for _ in 0..100 {
        let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let path = dir.join(format!(".state-{}-{sequence}.tmp", std::process::id()));
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(mut file) => {
                if let Err(error) = file.write_all(contents).and_then(|()| file.sync_all()) {
                    let _ = fs::remove_file(&path);
                    return Err(error.into());
                }
                return Ok(path);
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error.into()),
        }
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::AlreadyExists,
        "could not allocate unique state temp file",
    )
    .into())
}

#[cfg(windows)]
fn replace_file(source: &Path, destination: &Path) -> Result<(), PersistenceError> {
    use std::os::windows::ffi::OsStrExt;
    const MOVEFILE_REPLACE_EXISTING: u32 = 0x1;
    const MOVEFILE_WRITE_THROUGH: u32 = 0x8;
    #[link(name = "Kernel32")]
    extern "system" {
        fn MoveFileExW(existing: *const u16, new: *const u16, flags: u32) -> i32;
    }
    let source_wide: Vec<u16> = source.as_os_str().encode_wide().chain(Some(0)).collect();
    let destination_wide: Vec<u16> = destination
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect();
    // SAFETY: both paths are NUL-terminated UTF-16 buffers that remain alive for the call.
    let ok = unsafe {
        MoveFileExW(
            source_wide.as_ptr(),
            destination_wide.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if ok == 0 {
        Err(std::io::Error::last_os_error().into())
    } else {
        Ok(())
    }
}

#[cfg(not(windows))]
fn replace_file(source: &Path, destination: &Path) -> Result<(), PersistenceError> {
    fs::rename(source, destination).map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_dir(name: &str) -> PathBuf {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("te-{name}-{}-{unique}", std::process::id()))
    }

    #[test]
    fn round_trips_default_state() {
        let dir = test_dir("state-test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("state.json");

        let mut store = StateStore::open(&path).unwrap();
        assert_eq!(store.state, BotState::default());

        store
            .update(|s| {
                s.halted = true;
                s.sim = Some(SimBrokerState::new_with_quote("USDT", 10_000.0));
            })
            .unwrap();

        let reloaded = StateStore::open(&path).unwrap();
        assert!(reloaded.state.halted);
        assert_eq!(
            reloaded.state.sim.as_ref().unwrap().balances["USDT"],
            10_000.0
        );
    }

    #[test]
    fn persists_and_restores_positions_and_pending() {
        let dir = test_dir("state-pos");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("state.json");

        let mut store = StateStore::open(&path).unwrap();
        store
            .update(|s| {
                s.positions.insert(
                    "ETH/USDT".to_owned(),
                    PersistedPosition {
                        qty: 1.25,
                        entry: 2_000.0,
                        stop: 1_920.0,
                        atr: 40.0,
                        entry_time: "2024-01-01T00:00:00Z".to_owned(),
                        candle: "2024-01-01 00:00:00".to_owned(),
                        entry_fee: 2.5,
                        client_id: "dceETH123".to_owned(),
                        order_id: "42".to_owned(),
                        risk_quote: 100.0,
                    },
                );
                s.pending.insert(
                    "ETH/USDT".to_owned(),
                    PendingOrder {
                        kind: PendingOrderKind::Entry,
                        client_id: "dceETH123".to_owned(),
                        atr: Some(40.0),
                        candle: Some("2024-01-01 00:00:00".to_owned()),
                        reason: None,
                    },
                );
            })
            .unwrap();

        let reloaded = StateStore::open(&path).unwrap();
        assert_eq!(reloaded.state.positions["ETH/USDT"].qty, 1.25);
        assert_eq!(
            reloaded.state.pending["ETH/USDT"].kind,
            PendingOrderKind::Entry
        );
    }

    #[test]
    fn atomic_save_does_not_corrupt_on_interleaved_reads() {
        let dir = test_dir("state-atomic");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("state.json");
        let mut store = StateStore::open(&path).unwrap();
        for i in 0..10 {
            store.update(|s| s.halted = i % 2 == 0).unwrap();
            let reloaded = StateStore::open(&path).unwrap();
            assert_eq!(reloaded.state.halted, i % 2 == 0);
        }
    }

    #[test]
    fn persists_exchange_intents_partial_fills_and_restart_state_without_duplicates() {
        let dir = test_dir("state-exchange");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("state.json");
        let mut store = StateStore::open(&path).unwrap();
        let intent = PersistedExchangeOrder {
            symbol: "ETH/USDT".to_owned(),
            side: "BUY".to_owned(),
            client_order_id: "teETHUSDTB123".to_owned(),
            requested_qty: "0.3".to_owned(),
            status: PersistedOrderStatus::Submitting,
            exchange_order_id: None,
            executed_qty: "0".to_owned(),
            cumulative_quote_qty: "0".to_owned(),
            fills: HashMap::new(),
            signal_candle: Some("123".to_owned()),
            atr: Some(10.0),
            stop_price: Some("1980".to_owned()),
            reason: Some("entry".to_owned()),
            created_at_ms: 123,
            last_reconciled_at_ms: None,
            discrepancy: None,
        };
        assert!(store.record_exchange_intent(intent.clone()).unwrap());
        assert!(!store.record_exchange_intent(intent).unwrap());
        store
            .update_exchange_order("teETHUSDTB123", |order| {
                order.status = PersistedOrderStatus::PartiallyFilled;
                order.exchange_order_id = Some("42".to_owned());
                order.executed_qty = "0.1".to_owned();
                order.cumulative_quote_qty = "200".to_owned();
                order.fills.insert(
                    "7".to_owned(),
                    PersistedExchangeFill {
                        trade_id: "7".to_owned(),
                        qty: "0.1".to_owned(),
                        price: "2000".to_owned(),
                        quote_qty: "200".to_owned(),
                        commission: "0.0001".to_owned(),
                        commission_asset: "ETH".to_owned(),
                        time_ms: 456,
                    },
                );
            })
            .unwrap();

        let recovered = StateStore::open(&path).unwrap();
        let recovered_order = &recovered.state.exchange.as_ref().unwrap().orders["teETHUSDTB123"];
        assert_eq!(
            recovered_order.status,
            PersistedOrderStatus::PartiallyFilled
        );
        assert_eq!(recovered_order.executed_qty, "0.1");
        assert_eq!(recovered_order.fills.len(), 1);
        assert!(recovered_order.discrepancy.is_none());
        assert!(!recovered.state.positions.contains_key("ETH/USDT"));
    }

    #[test]
    fn old_simulator_state_without_exchange_field_still_loads() {
        let dir = test_dir("state-old");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("state.json");
        std::fs::write(&path, r#"{"positions":{},"pending":{},"last_candle":{},"day":null,"day_start_equity":null,"paused_day":null,"halted":false,"sim":null,"closed":[]}"#).unwrap();
        let store = StateStore::open(&path).unwrap();
        assert!(store.state.exchange.is_none());
    }
}
