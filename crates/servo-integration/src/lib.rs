//! Servo adapter. **This is the only crate that names Servo types.**
//!
//! Every Servo API used here is listed, with the behaviour it provides, in
//! `docs/SERVO_INTEGRATION.md`. Nothing in this crate patches Servo.

mod convert;
mod delegate;
mod engine;
mod memory;

pub use engine::{ServoEngine, ServoEngineConfig};
pub use memory::ServoMemory;

pub use servo;

// Link the allocator crate so its `#[global_allocator]` (jemalloc) is installed.
#[cfg(not(any(windows, target_env = "ohos")))]
use servo_allocator as _;
