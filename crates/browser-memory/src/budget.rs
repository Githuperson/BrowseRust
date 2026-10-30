use browser_core::ByteSize;
use browser_settings::MemoryOverrides;

/// What the OS says about physical memory right now.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SystemMemory {
    pub total: ByteSize,
    /// Memory that could be handed to us without forcing swapping: free +
    /// reclaimable file cache (+ purgeable on macOS). *Not* "free" alone — on
    /// Apple Silicon "free" is deliberately tiny.
    pub available: ByteSize,
}

impl SystemMemory {
    pub fn available_ratio(&self) -> f64 {
        if self.total.0 == 0 { 1.0 } else { self.available.0 as f64 / self.total.0 as f64 }
    }
}

/// All limits the governor enforces.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MemoryBudget {
    pub global_limit: ByteSize,
    pub tab_soft_limit: ByteSize,
    pub tab_hard_limit: ByteSize,
    pub image_cache_limit: ByteSize,
    pub font_cache_limit: ByteSize,
    pub http_cache_limit: ByteSize,
    pub gpu_budget: ByteSize,
}

impl MemoryBudget {
    /// Derive budgets adaptively.
    ///
    /// * The browser may use up to 45 % of physical RAM (≈3.6 GB on an 8 GB Mac),
    ///   leaving the rest to the OS and other apps.
    /// * It is additionally capped by what is actually obtainable now:
    ///   `own_usage + available − reserve`, with `reserve` = 12 % of RAM, so a
    ///   memory-hungry neighbour shrinks our budget instead of pushing the
    ///   machine into swap.
    /// * Explicit user overrides always win.
    pub fn adaptive(sys: SystemMemory, own_usage: ByteSize, o: &MemoryOverrides) -> Self {
        let total = sys.total.0.max(512 * MIB);
        let fixed_cap = (total as f64 * 0.45) as u64;
        let reserve = (total as f64 * 0.12) as u64;
        let obtainable = own_usage.0.saturating_add(sys.available.0).saturating_sub(reserve);
        let global = o.global_limit.map(|b| b.0).unwrap_or_else(|| fixed_cap.min(obtainable).max(384 * MIB));

        let soft = o.tab_soft_limit.map(|b| b.0).unwrap_or_else(|| (global / 10).max(96 * MIB));
        let hard = o.tab_hard_limit.map(|b| b.0).unwrap_or(soft * 2).max(soft);
        Self {
            global_limit: ByteSize(global),
            tab_soft_limit: ByteSize(soft),
            tab_hard_limit: ByteSize(hard),
            image_cache_limit: o.image_cache_limit.unwrap_or(ByteSize((global / 10).clamp(16 * MIB, 512 * MIB))),
            font_cache_limit: o.font_cache_limit.unwrap_or(ByteSize((global / 40).clamp(8 * MIB, 64 * MIB))),
            http_cache_limit: o.http_cache_limit.unwrap_or(ByteSize((global / 12).clamp(16 * MIB, 512 * MIB))),
            gpu_budget: o.gpu_budget.unwrap_or(ByteSize((global / 7).clamp(32 * MIB, 1024 * MIB))),
        }
    }
}

const MIB: u64 = 1024 * 1024;

#[cfg(test)]
mod tests {
    use super::*;

    fn sys(total_gib: u64, avail_gib_x10: u64) -> SystemMemory {
        SystemMemory { total: ByteSize::gib(total_gib), available: ByteSize(avail_gib_x10 * 1024 * MIB / 10) }
    }

    #[test]
    fn eight_gib_machine_gets_about_45_percent_when_memory_is_plentiful() {
        let b = MemoryBudget::adaptive(sys(8, 50), ByteSize(0), &MemoryOverrides::default());
        let expected = (8.0 * 1024.0 * 1024.0 * 1024.0 * 0.45) as u64;
        assert_eq!(b.global_limit.0, expected);
        assert!(b.tab_soft_limit < b.tab_hard_limit);
    }

    #[test]
    fn budget_shrinks_when_the_system_is_short_on_memory() {
        let plenty = MemoryBudget::adaptive(sys(8, 50), ByteSize(0), &MemoryOverrides::default());
        let tight = MemoryBudget::adaptive(sys(8, 10), ByteSize::mib(500), &MemoryOverrides::default());
        assert!(tight.global_limit < plenty.global_limit);
        assert!(tight.global_limit.0 >= 384 * MIB, "never below the floor");
    }

    #[test]
    fn overrides_win() {
        let o = MemoryOverrides { global_limit: Some(ByteSize::gib(1)), tab_soft_limit: Some(ByteSize::mib(50)), ..Default::default() };
        let b = MemoryBudget::adaptive(sys(8, 50), ByteSize(0), &o);
        assert_eq!(b.global_limit, ByteSize::gib(1));
        assert_eq!(b.tab_soft_limit, ByteSize::mib(50));
        assert!(b.tab_hard_limit >= b.tab_soft_limit);
    }

    #[test]
    fn cache_limits_are_clamped_sane_on_tiny_machines() {
        let b = MemoryBudget::adaptive(sys(1, 5), ByteSize(0), &MemoryOverrides::default());
        assert!(b.image_cache_limit.0 >= 16 * MIB);
        assert!(b.font_cache_limit.0 >= 8 * MIB);
    }
}
