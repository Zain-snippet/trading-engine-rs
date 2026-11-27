use std::{fs, path::PathBuf};

use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::PersistenceError;

/// Column order matching Python `paperbot.journal.FIELDS`.
pub const JOURNAL_FIELDS: &[&str] = &[
    "ts_utc",
    "event",
    "symbol",
    "side",
    "qty",
    "price",
    "ref_price",
    "stop",
    "equity",
    "pnl",
    "r_multiple",
    "order_id",
    "client_id",
    "details",
];

/// One structured row written to the CSV journal, matching `paperbot.journal.Journal.write`.
/// All fields are `Option<String>` so the CSV writer can emit empty cells for omitted fields,
/// preserving the Python journal's column layout exactly.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct JournalEntry {
    pub ts_utc: String,
    pub event: String,
    pub symbol: String,
    pub side: Option<String>,
    pub qty: Option<String>,
    pub price: Option<String>,
    pub ref_price: Option<String>,
    pub stop: Option<String>,
    pub equity: Option<String>,
    pub pnl: Option<String>,
    pub r_multiple: Option<String>,
    pub order_id: Option<String>,
    pub client_id: Option<String>,
    pub details: Option<String>,
}

impl JournalEntry {
    /// Creates a minimal entry with a UTC timestamp and event name.
    #[must_use]
    pub fn now(event: impl Into<String>) -> Self {
        let ts = Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
        Self {
            ts_utc: ts,
            event: event.into(),
            ..Default::default()
        }
    }

    pub fn with_symbol(mut self, sym: impl Into<String>) -> Self {
        self.symbol = sym.into();
        self
    }

    pub fn with_side(mut self, side: impl Into<String>) -> Self {
        self.side = Some(side.into());
        self
    }

    pub fn with_qty(mut self, qty: f64) -> Self {
        self.qty = Some(format!("{qty}"));
        self
    }

    pub fn with_price(mut self, price: f64) -> Self {
        self.price = Some(format!("{price}"));
        self
    }

    pub fn with_ref_price(mut self, price: f64) -> Self {
        self.ref_price = Some(format!("{price}"));
        self
    }

    pub fn with_stop(mut self, stop: f64) -> Self {
        self.stop = Some(format!("{stop:.2}"));
        self
    }

    pub fn with_equity(mut self, equity: f64) -> Self {
        self.equity = Some(format!("{equity:.2}"));
        self
    }

    pub fn with_pnl(mut self, pnl: f64) -> Self {
        self.pnl = Some(format!("{pnl:.4}"));
        self
    }

    pub fn with_r_multiple(mut self, r: f64) -> Self {
        self.r_multiple = Some(format!("{r:.3}"));
        self
    }

    pub fn with_order_id(mut self, id: impl Into<String>) -> Self {
        self.order_id = Some(id.into());
        self
    }

    pub fn with_client_id(mut self, id: impl Into<String>) -> Self {
        self.client_id = Some(id.into());
        self
    }

    pub fn with_details(mut self, details: impl Into<String>) -> Self {
        self.details = Some(details.into());
        self
    }

    fn as_row(&self) -> Vec<Option<String>> {
        vec![
            Some(self.ts_utc.clone()),
            Some(self.event.clone()),
            Some(self.symbol.clone()),
            self.side.clone(),
            self.qty.clone(),
            self.price.clone(),
            self.ref_price.clone(),
            self.stop.clone(),
            self.equity.clone(),
            self.pnl.clone(),
            self.r_multiple.clone(),
            self.order_id.clone(),
            self.client_id.clone(),
            self.details.clone(),
        ]
    }
}

/// Append-only CSV trade journal matching `paperbot.journal.Journal`.
/// Creates the file with a header row on first open.
#[derive(Debug)]
pub struct Journal {
    path: PathBuf,
}

impl Journal {
    /// Opens or creates the journal CSV. Writes the header if the file is new.
    pub fn open(path: impl Into<PathBuf>) -> Result<Self, PersistenceError> {
        let path = path.into();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        if !path.exists() {
            let mut writer = csv::Writer::from_path(&path)?;
            writer.write_record(JOURNAL_FIELDS)?;
            writer.flush()?;
        }
        Ok(Self { path })
    }

    /// Appends one entry to the CSV journal.
    pub fn write(&self, entry: &JournalEntry) -> Result<(), PersistenceError> {
        let file = fs::OpenOptions::new()
            .append(true)
            .create(true)
            .open(&self.path)?;
        let mut writer = csv::WriterBuilder::new()
            .has_headers(false)
            .from_writer(file);
        let owned: Vec<Option<String>> = entry.as_row();
        let row: Vec<&str> = owned
            .iter()
            .map(|cell| cell.as_deref().unwrap_or(""))
            .collect();
        writer.write_record(&row)?;
        writer.flush()?;
        Ok(())
    }

    /// Returns all rows as `JournalEntry` values (skips the header).
    pub fn read_all(&self) -> Result<Vec<JournalEntry>, PersistenceError> {
        let mut reader = csv::ReaderBuilder::new()
            .has_headers(true)
            .from_path(&self.path)?;
        let mut entries = Vec::new();
        for result in reader.records() {
            let record = result?;
            let get = |i: usize| -> Option<String> {
                record.get(i).filter(|s| !s.is_empty()).map(str::to_owned)
            };
            entries.push(JournalEntry {
                ts_utc: get(0).unwrap_or_default(),
                event: get(1).unwrap_or_default(),
                symbol: get(2).unwrap_or_default(),
                side: get(3),
                qty: get(4),
                price: get(5),
                ref_price: get(6),
                stop: get(7),
                equity: get(8),
                pnl: get(9),
                r_multiple: get(10),
                order_id: get(11),
                client_id: get(12),
                details: get(13),
            });
        }
        Ok(entries)
    }

    pub fn path(&self) -> &PathBuf {
        &self.path
    }
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
    fn creates_header_and_roundtrips_entries() {
        let dir = test_dir("journal-test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("journal.csv");

        let j = Journal::open(&path).unwrap();
        let entry = JournalEntry::now("fill")
            .with_symbol("ETH/USDT")
            .with_side("buy")
            .with_qty(1.25)
            .with_price(2_000.0)
            .with_stop(1_920.0)
            .with_equity(10_000.0)
            .with_client_id("dceETH123")
            .with_details("entry fee~2.5");
        j.write(&entry).unwrap();

        let rows = j.read_all().unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].symbol, "ETH/USDT");
        assert_eq!(rows[0].side.as_deref(), Some("buy"));
        assert_eq!(rows[0].qty.as_deref(), Some("1.25"));
        assert_eq!(rows[0].client_id.as_deref(), Some("dceETH123"));
        assert_eq!(rows[0].pnl, None);
    }

    #[test]
    fn appends_across_multiple_opens() {
        let dir = test_dir("journal-append");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("j.csv");

        for i in 0..3 {
            let j = Journal::open(&path).unwrap();
            j.write(&JournalEntry::now(format!("event{i}"))).unwrap();
        }

        let j = Journal::open(&path).unwrap();
        let rows = j.read_all().unwrap();
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].event, "event0");
        assert_eq!(rows[2].event, "event2");
    }

    #[test]
    fn journal_entry_has_expected_field_count() {
        let entry = JournalEntry::now("test");
        assert_eq!(entry.as_row().len(), JOURNAL_FIELDS.len());
    }
}
