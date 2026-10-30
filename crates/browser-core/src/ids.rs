use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU64, Ordering};

macro_rules! id_type {
    ($(#[$m:meta])* $name:ident) => {
        $(#[$m])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
        pub struct $name(pub u64);

        impl $name {
            /// Allocate a process-unique id.
            pub fn next() -> Self {
                static COUNTER: AtomicU64 = AtomicU64::new(1);
                Self(COUNTER.fetch_add(1, Ordering::Relaxed))
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, "{}#{}", stringify!($name), self.0)
            }
        }
    };
}

id_type!(/// Identifies a tab for its whole lifetime, including while discarded.
    TabId);
id_type!(/// Identifies a tab group.
    GroupId);
id_type!(/// Identifies a workspace / space.
    WorkspaceId);
id_type!(/// Identifies a single network request observed for a tab.
    RequestId);
