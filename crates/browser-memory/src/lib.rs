//! Original memory-management layer that sits *above* the engine.
//!
//! * [`budget`] — adaptive budgets derived from real system memory.
//! * [`accounting`] — per-tab ledger distinguishing private vs shared bytes.
//! * [`probe`] — platform probes (Linux `/proc`, macOS `sysctl`).
//! * [`pressure`] — pressure levels with hysteresis.
//! * [`cache`] — byte-bounded LRU cache, string interner, content-addressed blob store.
//! * [`governor`] — [`MemoryGovernor`]: pure `Snapshot -> Vec<GovernorAction>` policy.
//!
//! Nothing here touches Servo directly; the engine adapter feeds samples in and
//! executes the returned actions.

pub mod accounting;
pub mod budget;
pub mod cache;
pub mod governor;
pub mod pressure;
pub mod probe;

pub use accounting::{Category, MemoryLedger, TabAccount};
pub use budget::{MemoryBudget, SystemMemory};
pub use governor::{GovernorAction, GovernorConfig, MemoryGovernor, Snapshot, TabInfo, TrimLevel};
pub use pressure::{PressureLevel, PressureTracker};
