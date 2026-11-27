//! Crash-safe JSON state persistence, CSV trade journaling, and log secret
//! redaction matching `paperbot/state.py` and `paperbot/journal.py`.

mod error;
mod journal;
mod redaction;
mod state;

pub use error::PersistenceError;
pub use journal::{Journal, JournalEntry, JOURNAL_FIELDS};
pub use redaction::{redact_secrets, LogRedactor};
pub use state::{
    BotState, ClosedTradeRecord, ExchangeState, PendingOrder, PendingOrderKind,
    PersistedExchangeFill, PersistedExchangeOrder, PersistedFill, PersistedOrderStatus,
    PersistedPosition, ReconciliationDiscrepancy, SimBrokerState, StateStore,
};
