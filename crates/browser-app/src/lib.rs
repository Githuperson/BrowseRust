//! The browser orchestrator: wires settings, tab lifecycle, memory governor and
//! security policy to an [`Engine`]. UI code talks to [`Browser`] only.

pub mod browser;
pub mod omnibox;

pub use browser::{Browser, MemoryReport};
pub use omnibox::{Resolved, resolve_input};
