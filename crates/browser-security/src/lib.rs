//! Browser-side security policy. Website content is untrusted: everything here
//! decides what the *browser* will allow, independent of what the engine does.
//!
//! Engine-enforced protections (same-origin policy, CORS, CSP, TLS validation,
//! cookie flags, process isolation) are Servo's responsibility; see
//! `docs/SECURITY.md` for exactly which are relied upon and which are verified.

pub mod download;
pub mod mixed;
pub mod navigation;
pub mod permissions;
pub mod popup;

pub use download::{DownloadRisk, assess_download, sanitize_filename};
pub use mixed::{MixedContent, ResourceKind, check_mixed_content};
pub use navigation::{NavVerdict, SecurityLevel, check_navigation, security_level};
pub use permissions::{Decision, Feature, PermissionStore};
pub use popup::PopupBlocker;
