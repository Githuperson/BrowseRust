//! Shared primitives: identifiers, an injectable clock, byte-size helpers and
//! atomic file persistence. Everything here is engine-agnostic.

pub mod clock;
pub mod fs;
pub mod ids;
pub mod size;
pub mod state;

pub use clock::{Clock, ManualClock, SystemClock};
pub use ids::{GroupId, RequestId, TabId, WorkspaceId};
pub use size::ByteSize;
pub use state::TabState;
