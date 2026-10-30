//! Per-tab network activity control.
//!
//! The pieces:
//! * [`classify`] — maps a request to a [`RequestClass`].
//! * [`policy`] — `(mode, state, class) -> Action` table.
//! * [`controller`] — [`NetworkActivityController`]: applies the policy to each
//!   request of one tab, keeps a *bounded* hold queue, coalesces stale polls and
//!   releases everything on resume.
//! * [`stream`] — bounded buffers + the suspension plan for WebSocket/SSE.
//! * [`monitor`] — bounded per-tab request log and requests-per-minute counters.
//!
//! The crate is engine-agnostic: the request handle type `H` is generic. The
//! Servo integration instantiates it with `servo::WebResourceLoad`.

pub mod classify;
pub mod controller;
pub mod hub;
pub mod monitor;
pub mod policy;
pub mod request;
pub mod stream;

pub use classify::{AnalyticsMatcher, PollingDetector, RequestClass, classify};
pub use controller::{CancelReason, ControllerConfig, NetworkActivityController, Outcome, Resumed};
pub use hub::NetworkHub;
pub use monitor::{Disposition, NetworkMonitor, RequestRecord};
pub use policy::{Action, NetworkPolicy, effective_mode};
pub use request::{Destination, RequestInfo};
