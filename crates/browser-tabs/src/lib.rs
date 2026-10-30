//! Tab model, tab strip operations and the [`TabLifecycleManager`] state machine.
//!
//! The lifecycle manager never talks to an engine. It mutates tab state and
//! returns [`LifecycleCommand`]s; the shell executes them through the
//! `browser-engine` abstraction. That keeps the full state machine testable and
//! keeps Servo behind one boundary.

pub mod lifecycle;
pub mod strip;
pub mod tab;

pub use lifecycle::{LifecycleCommand, TabLifecycleManager};
pub use strip::{Group, TabStrip, Workspace};
pub use tab::Tab;
