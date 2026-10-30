use serde::{Deserialize, Serialize};
use std::fmt;

/// A byte count with human-friendly formatting.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ByteSize(pub u64);

impl ByteSize {
    pub const fn kib(n: u64) -> Self { Self(n * 1024) }
    pub const fn mib(n: u64) -> Self { Self(n * 1024 * 1024) }
    pub const fn gib(n: u64) -> Self { Self(n * 1024 * 1024 * 1024) }
    pub const fn bytes(self) -> u64 { self.0 }
    pub fn saturating_sub(self, o: Self) -> Self { Self(self.0.saturating_sub(o.0)) }
    pub fn saturating_add(self, o: Self) -> Self { Self(self.0.saturating_add(o.0)) }
}

impl fmt::Display for ByteSize {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
        let mut v = self.0 as f64;
        let mut u = 0;
        while v >= 1024.0 && u < UNITS.len() - 1 {
            v /= 1024.0;
            u += 1;
        }
        if u == 0 { write!(f, "{} B", self.0) } else { write!(f, "{v:.1} {}", UNITS[u]) }
    }
}
