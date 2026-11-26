use std::{collections::HashMap, fs, path::PathBuf};

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
    #[serde(default)]
    pub closed: Vec<ClosedTradeRecord>,
}

/// Atomic crash-safe JSON state store matching Python `State.save()` using
/// `tempfile + os.replace`.
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

        let dir = self.path.parent().unwrap_or(std::path::Path::new("."));
        let tmp_path = dir.join(".state-tmp.json");
        let json = serde_json::to_vec_pretty(&self.state)?;
        fs::write(&tmp_path, &json)?;
        fs::rename(&tmp_path, &self.path)?;

        Ok(())
    }

    /// Convenience: mutate state and immediately save.
    pub fn update<F>(&mut self, mutate: F) -> Result<(), PersistenceError>
    where
        F: FnOnce(&mut BotState),
    {
        mutate(&mut self.state);
        self.save()
    }

    pub fn path(&self) -> &PathBuf {
        &self.path
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_default_state() {
        use std::env;
        let dir = env::temp_dir().join(format!("te-state-test-{}", std::process::id()));
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
        use std::env;
        let dir = env::temp_dir().join(format!("te-state-pos-{}", std::process::id()));
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
        use std::env;
        let dir = env::temp_dir().join(format!("te-state-atomic-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("state.json");
        let mut store = StateStore::open(&path).unwrap();
        for i in 0..10 {
            store.update(|s| s.halted = i % 2 == 0).unwrap();
            let reloaded = StateStore::open(&path).unwrap();
            assert_eq!(reloaded.state.halted, i % 2 == 0);
        }
    }
}
