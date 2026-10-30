//! Browser-owned downloads (Servo 0.7.0 has no download API).
//!
//! Flow: the engine adapter holds a user-started main-frame request, calls
//! [`probe::probe`] with the page's cookies, and — if the response is a download —
//! cancels the engine's load and hands the request to [`manager::DownloadManager`],
//! which streams the file itself.

pub mod classify;
pub mod fetch;
pub mod manager;

pub use classify::{Decision, decide, filename_for, parse_content_disposition};
pub use fetch::{ProbeResult, RequestContext, probe};
pub use manager::{DownloadManager, JobState, JobView};
