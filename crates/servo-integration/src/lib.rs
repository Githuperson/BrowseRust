//! Servo adapter. **This is the only crate that names Servo types.**
//!
//! Every Servo API used here is listed, with the behaviour it provides, in
//! `docs/SERVO_INTEGRATION.md`. Nothing in this crate patches Servo.

mod allocator;
mod convert;
mod delegate;
mod engine;
mod memory;

pub use engine::{PermissionPrompt, ServoEngine, ServoEngineConfig};
pub use allocator::{opt_background_thread, opt_dirty_decay_ms, stats as allocator_stats};
pub use memory::ServoMemory;

pub use servo;

// Link the allocator crate so its `#[global_allocator]` (jemalloc) is installed.
#[cfg(not(any(windows, target_env = "ohos")))]
use servo_allocator as _;

/// Servo preferences the browser sets on top of Servo's defaults.
///
/// Only features with a working permission prompt are enabled. Servo 0.7.0 leaves
/// geolocation, WebRTC/getUserMedia, async clipboard and Bluetooth **off**; they
/// stay off until a prompt and a backend for each exist.
pub fn browser_preferences() -> servo::Preferences {
    let mut p = servo::Preferences::default();
    p.dom_notification_enabled = true;
    p.dom_permissions_enabled = true;
    p
}
