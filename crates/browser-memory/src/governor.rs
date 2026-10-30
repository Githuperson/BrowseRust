//! The [`MemoryGovernor`]: turns a point-in-time [`Snapshot`] into actions.
//!
//! It is a pure policy object (no I/O, no engine access) so the whole decision
//! table is unit-tested. The shell executes the returned actions through the
//! tab lifecycle manager, which is the single place that enforces the final
//! veto rules (e.g. a foreground tab can never be frozen or discarded).
//!
//! Safety invariants, enforced here *and* re-checked by the lifecycle manager:
//! * the foreground tab is never a target of `Freeze`/`Discard`/`ReleaseTabImages`;
//! * audible, loading and form-dirty tabs are never auto-discarded;
//! * discards are rate-limited and stop as soon as the projected total fits.

use crate::budget::MemoryBudget;
use crate::pressure::PressureLevel;
use browser_core::{ByteSize, TabId, TabState};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum TrimLevel {
    /// Drop cold entries above ~75 % of each cache's budget.
    Light,
    /// Halve caches.
    Moderate,
    /// Empty reclaimable caches (decoded images, HTTP cache memory, fonts not in use).
    Aggressive,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reason {
    TabOverSoftLimit,
    TabOverHardLimit,
    GlobalOverBudget,
    PressureWarn,
    PressureCritical,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GovernorAction {
    TrimCaches(TrimLevel),
    ReleaseTabImages(TabId),
    Freeze(TabId, Reason),
    Discard(TabId, Reason),
}

#[derive(Clone, Debug)]
pub struct TabInfo {
    pub id: TabId,
    pub state: TabState,
    /// Private (reclaimable-by-discard) bytes attributed to the tab.
    pub private: ByteSize,
    pub pinned: bool,
    pub audible: bool,
    pub loading: bool,
    /// The page has user-entered form data that a discard could not restore.
    pub dirty_form: bool,
    /// Last time the tab was foreground / interacted with.
    pub last_active: Instant,
}

pub struct Snapshot<'a> {
    pub now: Instant,
    pub pressure: PressureLevel,
    pub browser_total: ByteSize,
    pub budget: MemoryBudget,
    pub tabs: &'a [TabInfo],
}

#[derive(Clone, Debug)]
pub struct GovernorConfig {
    pub trim_cooldown: Duration,
    pub discard_cooldown: Duration,
    pub max_freezes_per_pass: usize,
    pub max_discards_per_pass: usize,
    /// Under pressure, only tabs idle at least this long are frozen/discarded, so
    /// a tab the user just left is not immediately sacrificed.
    pub min_idle: Duration,
    /// Stop discarding once projected usage is below this fraction of the limit.
    pub target_fraction: f64,
}

impl Default for GovernorConfig {
    fn default() -> Self {
        Self {
            trim_cooldown: Duration::from_secs(5),
            discard_cooldown: Duration::from_secs(3),
            max_freezes_per_pass: 3,
            max_discards_per_pass: 2,
            min_idle: Duration::from_secs(10),
            target_fraction: 0.9,
        }
    }
}

pub struct MemoryGovernor {
    cfg: GovernorConfig,
    last_trim: Option<(Instant, TrimLevel)>,
    last_discard: Option<Instant>,
}

impl MemoryGovernor {
    pub fn new(cfg: GovernorConfig) -> Self {
        Self { cfg, last_trim: None, last_discard: None }
    }

    pub fn evaluate(&mut self, s: &Snapshot<'_>) -> Vec<GovernorAction> {
        let mut out = Vec::new();
        let limit = s.budget.global_limit.0;
        let over_budget = s.browser_total.0 > limit;
        let near_budget = s.browser_total.0 as f64 > limit as f64 * 0.85;
        let idle = |t: &TabInfo| s.now.saturating_duration_since(t.last_active) >= self.cfg.min_idle;

        // ---- 1. Global cache trimming (rate-limited, escalation is immediate).
        let wanted = match s.pressure {
            PressureLevel::Critical => Some(TrimLevel::Aggressive),
            PressureLevel::Warn => Some(TrimLevel::Moderate),
            PressureLevel::Normal if over_budget => Some(TrimLevel::Moderate),
            PressureLevel::Normal if near_budget => Some(TrimLevel::Light),
            PressureLevel::Normal => None,
        };
        if let Some(level) = wanted {
            let allowed = match self.last_trim {
                Some((at, prev)) => level > prev || s.now.saturating_duration_since(at) >= self.cfg.trim_cooldown,
                None => true,
            };
            if allowed {
                self.last_trim = Some((s.now, level));
                out.push(GovernorAction::TrimCaches(level));
            }
        }

        // ---- 2. Per-tab limits (background tabs only).
        let mut targeted: Vec<TabId> = Vec::new();
        for t in s.tabs.iter().filter(|t| t.state.is_background()) {
            if t.private > s.budget.tab_soft_limit {
                out.push(GovernorAction::ReleaseTabImages(t.id));
                if t.state == TabState::BackgroundWarm {
                    out.push(GovernorAction::Freeze(t.id, Reason::TabOverSoftLimit));
                    targeted.push(t.id);
                }
            }
        }

        // ---- 3. Freeze idle warm tabs when the system or budget is under strain.
        let strained = s.pressure >= PressureLevel::Warn || over_budget;
        if strained {
            let reason = match (s.pressure, over_budget) {
                (PressureLevel::Critical, _) => Reason::PressureCritical,
                (PressureLevel::Warn, _) => Reason::PressureWarn,
                _ => Reason::GlobalOverBudget,
            };
            let mut warm: Vec<&TabInfo> = s
                .tabs
                .iter()
                .filter(|t| t.state == TabState::BackgroundWarm && !t.audible && idle(t) && !targeted.contains(&t.id))
                .collect();
            warm.sort_by_key(|t| t.last_active); // oldest first
            for t in warm.into_iter().take(self.cfg.max_freezes_per_pass) {
                out.push(GovernorAction::Freeze(t.id, reason));
                targeted.push(t.id);
            }
        }

        // ---- 4. Discard, only for severe situations.
        let cooled = self.last_discard.is_none_or(|at| s.now.saturating_duration_since(at) >= self.cfg.discard_cooldown);
        let critical = s.pressure == PressureLevel::Critical;
        let over_hard: Vec<TabId> = s
            .tabs
            .iter()
            .filter(|t| t.state.is_background() && t.private > s.budget.tab_hard_limit)
            .map(|t| t.id)
            .collect();
        if cooled && (critical || (over_budget && s.pressure >= PressureLevel::Warn) || !over_hard.is_empty()) {
            let mut cands: Vec<&TabInfo> = s
                .tabs
                .iter()
                .filter(|t| {
                    t.state.is_background()
                        && !t.audible
                        && !t.loading
                        && !t.dirty_form
                        && idle(t)
                        && (!t.pinned || critical)
                })
                .collect();
            // Frozen before warm, then biggest first: frees the most per lost tab.
            cands.sort_by(|a, b| {
                (a.state != TabState::BackgroundFrozen).cmp(&(b.state != TabState::BackgroundFrozen)).then(b.private.cmp(&a.private))
            });
            let target = (limit as f64 * self.cfg.target_fraction) as u64;
            let mut projected = s.browser_total.0;
            let mut n = 0;
            for t in cands {
                let is_over_hard = over_hard.contains(&t.id);
                let need_more = projected > target || (critical && n == 0);
                if n >= self.cfg.max_discards_per_pass || !(need_more || is_over_hard) {
                    if n >= self.cfg.max_discards_per_pass {
                        break;
                    }
                    continue;
                }
                let reason = if is_over_hard {
                    Reason::TabOverHardLimit
                } else if critical {
                    Reason::PressureCritical
                } else {
                    Reason::GlobalOverBudget
                };
                // A discard supersedes a freeze emitted earlier in this pass.
                out.retain(|a| !matches!(a, GovernorAction::Freeze(id, _) if *id == t.id));
                out.push(GovernorAction::Discard(t.id, reason));
                projected = projected.saturating_sub(t.private.0);
                n += 1;
            }
            if n > 0 {
                self.last_discard = Some(s.now);
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn budget() -> MemoryBudget {
        MemoryBudget {
            global_limit: ByteSize::mib(1000),
            tab_soft_limit: ByteSize::mib(100),
            tab_hard_limit: ByteSize::mib(200),
            image_cache_limit: ByteSize::mib(100),
            font_cache_limit: ByteSize::mib(16),
            http_cache_limit: ByteSize::mib(80),
            gpu_budget: ByteSize::mib(100),
        }
    }

    struct Fx {
        t0: Instant,
        tabs: Vec<TabInfo>,
    }
    impl Fx {
        fn new() -> Self {
            Self { t0: Instant::now(), tabs: vec![] }
        }
        fn add(&mut self, id: u64, state: TabState, mib: u64, idle_secs: u64) -> &mut TabInfo {
            // `now` in snapshots is t0 + 1000 s, so last_active = now - idle.
            self.tabs.push(TabInfo {
                id: TabId(id),
                state,
                private: ByteSize::mib(mib),
                pinned: false,
                audible: false,
                loading: false,
                dirty_form: false,
                last_active: self.t0 + Duration::from_secs(1000 - idle_secs),
            });
            self.tabs.last_mut().unwrap()
        }
        fn snap(&self, pressure: PressureLevel, total_mib: u64) -> Snapshot<'_> {
            Snapshot { now: self.t0 + Duration::from_secs(1000), pressure, browser_total: ByteSize::mib(total_mib), budget: budget(), tabs: &self.tabs }
        }
    }

    fn gov() -> MemoryGovernor {
        MemoryGovernor::new(GovernorConfig::default())
    }

    #[test]
    fn healthy_system_does_nothing() {
        let mut fx = Fx::new();
        fx.add(1, TabState::Foreground, 150, 0);
        fx.add(2, TabState::BackgroundWarm, 50, 600);
        assert!(gov().evaluate(&fx.snap(PressureLevel::Normal, 400)).is_empty());
    }

    #[test]
    fn near_budget_triggers_only_a_light_trim() {
        let mut fx = Fx::new();
        fx.add(1, TabState::Foreground, 100, 0);
        let acts = gov().evaluate(&fx.snap(PressureLevel::Normal, 900));
        assert_eq!(acts, vec![GovernorAction::TrimCaches(TrimLevel::Light)]);
    }

    #[test]
    fn background_tab_over_soft_limit_releases_images_and_freezes() {
        let mut fx = Fx::new();
        fx.add(1, TabState::Foreground, 50, 0);
        fx.add(2, TabState::BackgroundWarm, 150, 30);
        let acts = gov().evaluate(&fx.snap(PressureLevel::Normal, 300));
        assert!(acts.contains(&GovernorAction::ReleaseTabImages(TabId(2))));
        assert!(acts.contains(&GovernorAction::Freeze(TabId(2), Reason::TabOverSoftLimit)));
    }

    #[test]
    fn foreground_is_never_targeted_even_when_huge_and_critical() {
        let mut fx = Fx::new();
        fx.add(1, TabState::Foreground, 900, 0);
        fx.add(2, TabState::BackgroundFrozen, 20, 900);
        let acts = gov().evaluate(&fx.snap(PressureLevel::Critical, 1200));
        for a in &acts {
            match a {
                GovernorAction::Freeze(id, _) | GovernorAction::Discard(id, _) | GovernorAction::ReleaseTabImages(id) => {
                    assert_ne!(*id, TabId(1), "foreground must never be targeted: {a:?}")
                }
                GovernorAction::TrimCaches(_) => {}
            }
        }
        assert!(acts.contains(&GovernorAction::TrimCaches(TrimLevel::Aggressive)));
    }

    #[test]
    fn warn_pressure_freezes_oldest_idle_warm_tabs_first_and_limits_batch() {
        let mut fx = Fx::new();
        fx.add(1, TabState::Foreground, 50, 0);
        for (i, idle) in [(2u64, 100u64), (3, 900), (4, 500), (5, 700), (6, 300)] {
            fx.add(i, TabState::BackgroundWarm, 30, idle);
        }
        let acts = gov().evaluate(&fx.snap(PressureLevel::Warn, 400));
        let frozen: Vec<u64> = acts.iter().filter_map(|a| if let GovernorAction::Freeze(id, _) = a { Some(id.0) } else { None }).collect();
        assert_eq!(frozen, vec![3, 5, 4], "oldest-idle first, max 3 per pass");
        assert!(!acts.iter().any(|a| matches!(a, GovernorAction::Discard(..))), "warn pressure never discards by itself");
    }

    #[test]
    fn recently_used_tabs_are_spared_under_pressure() {
        let mut fx = Fx::new();
        fx.add(1, TabState::Foreground, 50, 0);
        fx.add(2, TabState::BackgroundWarm, 30, 3); // left 3 s ago
        let acts = gov().evaluate(&fx.snap(PressureLevel::Warn, 400));
        assert!(!acts.iter().any(|a| matches!(a, GovernorAction::Freeze(..))));
    }

    #[test]
    fn critical_discards_frozen_before_warm_biggest_first_and_stops_at_cap() {
        let mut fx = Fx::new();
        fx.add(1, TabState::Foreground, 100, 0);
        fx.add(2, TabState::BackgroundWarm, 400, 600);
        fx.add(3, TabState::BackgroundFrozen, 60, 600);
        fx.add(4, TabState::BackgroundFrozen, 90, 600);
        fx.add(5, TabState::BackgroundFrozen, 30, 600);
        let acts = gov().evaluate(&fx.snap(PressureLevel::Critical, 1300));
        let discards: Vec<u64> = acts.iter().filter_map(|a| if let GovernorAction::Discard(id, _) = a { Some(id.0) } else { None }).collect();
        assert_eq!(discards, vec![4, 3], "frozen first, largest first, max 2 per pass");
    }

    #[test]
    fn discard_stops_once_projected_total_fits() {
        let mut fx = Fx::new();
        fx.add(1, TabState::Foreground, 100, 0);
        fx.add(2, TabState::BackgroundFrozen, 300, 600);
        fx.add(3, TabState::BackgroundFrozen, 100, 600);
        // 1050 → after discarding the 300 MiB tab: 750 ≤ 900 (90 % of 1000), so only one.
        let acts = gov().evaluate(&fx.snap(PressureLevel::Warn, 1050));
        let discards: Vec<_> = acts.iter().filter(|a| matches!(a, GovernorAction::Discard(..))).collect();
        assert_eq!(discards.len(), 1);
    }

    #[test]
    fn protected_tabs_are_never_auto_discarded() {
        let mut fx = Fx::new();
        fx.add(1, TabState::Foreground, 100, 0);
        fx.add(2, TabState::BackgroundFrozen, 300, 600).audible = true;
        fx.add(3, TabState::BackgroundFrozen, 300, 600).loading = true;
        fx.add(4, TabState::BackgroundFrozen, 300, 600).dirty_form = true;
        let acts = gov().evaluate(&fx.snap(PressureLevel::Critical, 1500));
        assert!(!acts.iter().any(|a| matches!(a, GovernorAction::Discard(..))), "{acts:?}");
    }

    #[test]
    fn pinned_tabs_are_discarded_only_in_critical_pressure() {
        let mut fx = Fx::new();
        fx.add(1, TabState::Foreground, 100, 0);
        fx.add(2, TabState::BackgroundFrozen, 300, 600).pinned = true;
        let warn = gov().evaluate(&fx.snap(PressureLevel::Warn, 1100));
        assert!(!warn.iter().any(|a| matches!(a, GovernorAction::Discard(..))));
        let crit = gov().evaluate(&fx.snap(PressureLevel::Critical, 1100));
        assert!(crit.iter().any(|a| matches!(a, GovernorAction::Discard(TabId(2), _))));
    }

    #[test]
    fn a_tab_over_its_hard_limit_is_discarded_even_without_system_pressure() {
        let mut fx = Fx::new();
        fx.add(1, TabState::Foreground, 100, 0);
        fx.add(2, TabState::BackgroundFrozen, 250, 600); // > 200 MiB hard limit
        let acts = gov().evaluate(&fx.snap(PressureLevel::Normal, 500));
        assert!(acts.contains(&GovernorAction::Discard(TabId(2), Reason::TabOverHardLimit)));
    }

    #[test]
    fn discards_and_trims_are_rate_limited() {
        let mut fx = Fx::new();
        fx.add(1, TabState::Foreground, 100, 0);
        fx.add(2, TabState::BackgroundFrozen, 300, 600);
        let mut g = gov();
        let a1 = g.evaluate(&fx.snap(PressureLevel::Critical, 1200));
        assert!(a1.iter().any(|a| matches!(a, GovernorAction::Discard(..))));
        let a2 = g.evaluate(&fx.snap(PressureLevel::Critical, 1200)); // same instant
        // ReleaseTabImages is idempotent and may repeat; destructive/expensive actions may not.
        assert!(
            !a2.iter().any(|a| matches!(a, GovernorAction::Discard(..) | GovernorAction::TrimCaches(_))),
            "cooldown suppresses repeats: {a2:?}"
        );
    }

    #[test]
    fn escalating_trim_bypasses_cooldown() {
        let mut fx = Fx::new();
        fx.add(1, TabState::Foreground, 100, 0);
        let mut g = gov();
        assert_eq!(g.evaluate(&fx.snap(PressureLevel::Warn, 500)), vec![GovernorAction::TrimCaches(TrimLevel::Moderate)]);
        assert!(g.evaluate(&fx.snap(PressureLevel::Warn, 500)).is_empty());
        assert_eq!(g.evaluate(&fx.snap(PressureLevel::Critical, 500)), vec![GovernorAction::TrimCaches(TrimLevel::Aggressive)]);
    }

    #[test]
    fn discard_replaces_a_freeze_for_the_same_tab_in_one_pass() {
        let mut fx = Fx::new();
        fx.add(1, TabState::Foreground, 100, 0);
        fx.add(2, TabState::BackgroundWarm, 250, 600);
        let acts = gov().evaluate(&fx.snap(PressureLevel::Critical, 800));
        assert!(acts.contains(&GovernorAction::Discard(TabId(2), Reason::TabOverHardLimit)));
        assert!(!acts.iter().any(|a| matches!(a, GovernorAction::Freeze(TabId(2), _))));
    }
}
