use crate::budget::SystemMemory;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum PressureLevel {
    #[default]
    Normal,
    Warn,
    Critical,
}

/// Turns raw signals into a stable level. Escalation is immediate; de-escalation
/// requires the signal to stay lower for `calm_samples` consecutive samples, so
/// the governor does not oscillate between trimming and re-growing caches.
pub struct PressureTracker {
    level: PressureLevel,
    calm: u32,
    calm_samples: u32,
}

impl PressureTracker {
    pub fn new(calm_samples: u32) -> Self {
        Self { level: PressureLevel::Normal, calm: 0, calm_samples }
    }

    pub fn level(&self) -> PressureLevel {
        self.level
    }

    /// Combine an OS-reported level (macOS memorystatus / Linux PSI, if any) with
    /// the available-memory ratio, taking the worst of the two.
    pub fn raw_level(os: Option<PressureLevel>, sys: SystemMemory) -> PressureLevel {
        let ratio = sys.available_ratio();
        let by_ratio = if ratio < 0.05 {
            PressureLevel::Critical
        } else if ratio < 0.12 {
            PressureLevel::Warn
        } else {
            PressureLevel::Normal
        };
        by_ratio.max(os.unwrap_or_default())
    }

    pub fn update(&mut self, raw: PressureLevel) -> PressureLevel {
        if raw >= self.level {
            self.level = raw;
            self.calm = 0;
        } else {
            self.calm += 1;
            if self.calm >= self.calm_samples {
                // Step down one level at a time.
                self.level = match self.level {
                    PressureLevel::Critical => PressureLevel::Warn,
                    _ => PressureLevel::Normal,
                };
                self.calm = 0;
            }
        }
        self.level
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use browser_core::ByteSize;

    fn sys(avail_pct: u64) -> SystemMemory {
        SystemMemory { total: ByteSize(100), available: ByteSize(avail_pct) }
    }

    #[test]
    fn raw_level_takes_the_worst_signal() {
        assert_eq!(PressureTracker::raw_level(None, sys(50)), PressureLevel::Normal);
        assert_eq!(PressureTracker::raw_level(None, sys(10)), PressureLevel::Warn);
        assert_eq!(PressureTracker::raw_level(None, sys(3)), PressureLevel::Critical);
        assert_eq!(PressureTracker::raw_level(Some(PressureLevel::Warn), sys(50)), PressureLevel::Warn);
        assert_eq!(PressureTracker::raw_level(Some(PressureLevel::Normal), sys(3)), PressureLevel::Critical);
    }

    #[test]
    fn escalates_immediately_and_deescalates_with_hysteresis() {
        let mut t = PressureTracker::new(3);
        assert_eq!(t.update(PressureLevel::Critical), PressureLevel::Critical);
        assert_eq!(t.update(PressureLevel::Normal), PressureLevel::Critical);
        assert_eq!(t.update(PressureLevel::Normal), PressureLevel::Critical);
        assert_eq!(t.update(PressureLevel::Normal), PressureLevel::Warn, "one step down after 3 calm samples");
        for _ in 0..3 {
            t.update(PressureLevel::Normal);
        }
        assert_eq!(t.level(), PressureLevel::Normal);
    }

    #[test]
    fn a_spike_resets_the_calm_counter() {
        let mut t = PressureTracker::new(3);
        t.update(PressureLevel::Warn);
        t.update(PressureLevel::Normal);
        t.update(PressureLevel::Normal);
        t.update(PressureLevel::Warn);
        t.update(PressureLevel::Normal);
        t.update(PressureLevel::Normal);
        assert_eq!(t.level(), PressureLevel::Warn);
    }
}
