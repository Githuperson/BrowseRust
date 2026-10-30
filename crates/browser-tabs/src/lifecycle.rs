//! The four-state tab lifecycle.
//!
//! ```text
//!            select(other)              idle ≥ freeze_after / governor
//!  Foreground ───────────▶ BackgroundWarm ───────────────────▶ BackgroundFrozen
//!      ▲                        │                                    │
//!      │ select                 │ governor (critical) / user         │ governor / user
//!      │                        ▼                                    ▼
//!      └────────────────────────────── Discarded ◀───────────────────┘
//!                      select ⇒ Restore (the only transition that loads a page)
//! ```
//!
//! Returning to a Warm or Frozen tab emits exactly `Show` + `ApplyNetwork` and
//! never a navigation: the existing page keeps running and updates in place.

use crate::strip::TabStrip;
use crate::tab::Tab;
use browser_core::{Clock, TabId, TabState, WorkspaceId};
use browser_memory::{GovernorAction, PressureLevel, TabInfo, TrimLevel};
use browser_session::TabSnapshot;
use browser_settings::{BackgroundMode, Settings};
use std::sync::Arc;
use std::time::Duration;
use url::Url;

/// Work for the shell/engine adapter. Deliberately has **no** `Reload` or
/// `Navigate` variant for tab switches; the only page-loading commands are
/// [`LifecycleCommand::Restore`] (discarded tabs) and the opt-in
/// [`LifecycleCommand::RefreshOnReturn`].
#[derive(Clone, Debug, PartialEq)]
pub enum LifecycleCommand {
    /// Make the tab's WebView visible and focused (engine: `WebView::show`).
    Show(TabId),
    /// Hide the WebView: stops painting and lets the engine throttle script
    /// (engine: `WebView::hide`).
    Hide(TabId),
    /// Install the network policy for the tab's new state/mode and release any
    /// requests that were held.
    ApplyNetwork { tab: TabId, state: TabState, mode: BackgroundMode },
    /// Ask the engine to drop decoded image data for this tab, if it can.
    ReleaseImages(TabId),
    /// Trim shared caches (browser-owned and engine caches).
    TrimCaches(TrimLevel),
    /// Destroy the WebView; `snapshot` is all that remains.
    Discard { tab: TabId, snapshot: TabSnapshot },
    /// Create a WebView for a discarded tab and load its snapshot.
    Restore { tab: TabId, snapshot: TabSnapshot },
    /// Opt-in per-site "Refresh content when returning to this tab": a
    /// conditional revalidation, never triggered by default.
    RefreshOnReturn(TabId),
    /// Create a WebView for a new tab.
    Create { tab: TabId, url: Option<Url> },
    /// Destroy a closed tab's WebView and cancel its held requests.
    Close(TabId),
}

pub struct TabLifecycleManager {
    clock: Arc<dyn Clock>,
    pub strip: TabStrip,
}

impl TabLifecycleManager {
    pub fn new(clock: Arc<dyn Clock>) -> Self {
        Self { clock, strip: TabStrip::new() }
    }

    // ------------------------------------------------------------ creation

    /// Open a tab. `foreground = false` models "open in background tab": it
    /// starts Warm and its initial load is allowed to complete.
    pub fn open(&mut self, url: Option<Url>, foreground: bool, settings: &Settings) -> (TabId, Vec<LifecycleCommand>) {
        let now = self.clock.now();
        let id = TabId::next();
        let mut tab = Tab::new(id, self.strip.active_workspace, now);
        tab.url = url.clone();
        tab.loading = url.is_some();
        self.strip.insert_tab(tab, now);
        let mut cmds = vec![LifecycleCommand::Create { tab: id, url }];
        if foreground {
            cmds.extend(self.select(id, settings));
        } else {
            cmds.push(LifecycleCommand::Hide(id));
            cmds.push(self.apply_net(id, settings));
        }
        (id, cmds)
    }

    /// Register a tab the engine created on its own (a gesture-approved popup).
    /// No `Create` command: the WebView already exists.
    pub fn adopt(&mut self, id: TabId, foreground: bool, settings: &Settings) -> Vec<LifecycleCommand> {
        let now = self.clock.now();
        let mut tab = Tab::new(id, self.strip.active_workspace, now);
        tab.loading = true;
        self.strip.insert_tab(tab, now);
        if foreground {
            self.select(id, settings)
        } else {
            vec![LifecycleCommand::Hide(id), self.apply_net(id, settings)]
        }
    }

    pub fn open_from_snapshot(&mut self, snap: &TabSnapshot, workspace: Option<WorkspaceId>) -> (TabId, Vec<LifecycleCommand>) {
        let now = self.clock.now();
        let id = TabId::next();
        let mut tab = Tab::new(id, workspace.unwrap_or(self.strip.active_workspace), now);
        tab.url = snap.restore_url().and_then(|u| Url::parse(u).ok());
        tab.title = snap.title.clone();
        tab.history = snap.history.clone();
        tab.history_index = snap.history_index;
        tab.scroll = (snap.scroll_x, snap.scroll_y);
        tab.zoom = if snap.zoom > 0.0 { snap.zoom } else { 1.0 };
        tab.pinned = snap.pinned;
        tab.muted = snap.muted;
        // Restored tabs start Discarded: no renderer memory until first selected.
        tab.state = TabState::Discarded;
        self.strip.insert_tab(tab, now);
        (id, vec![])
    }

    /// Engine event: the page started/finished loading. The shell MUST forward
    /// load-status changes here; a tab stuck `loading` is protected from
    /// automatic freezing (for a bounded time) and discarding.
    pub fn set_loading(&mut self, id: TabId, loading: bool) {
        if let Some(t) = self.strip.tab_mut(id) {
            t.loading = loading;
        }
    }

    // ----------------------------------------------------------- selection

    /// Select a tab: the previous foreground tab goes Warm, the new one
    /// Foreground. Never reloads a live page.
    pub fn select(&mut self, id: TabId, settings: &Settings) -> Vec<LifecycleCommand> {
        let now = self.clock.now();
        let mut cmds = Vec::new();
        let Some(target) = self.strip.tab(id) else { return cmds };
        if target.state == TabState::Foreground {
            return cmds;
        }

        // 1. Background the old foreground tab.
        if let Some(prev) = self.strip.selected.filter(|p| *p != id) {
            if let Some(t) = self.strip.tab_mut(prev) {
                if t.state == TabState::Foreground {
                    t.state = TabState::BackgroundWarm;
                    t.state_since = now;
                    t.last_active = now;
                    cmds.push(LifecycleCommand::Hide(prev));
                    cmds.push(self.apply_net(prev, settings));
                }
            }
        }

        // 2. Foreground the new one.
        let was = self.strip.tab(id).map(|t| t.state).unwrap_or(TabState::Foreground);
        let host = self.strip.tab(id).and_then(|t| t.host().map(str::to_owned));
        let snapshot = self.strip.tab(id).map(Tab::snapshot);
        if let Some(t) = self.strip.tab_mut(id) {
            t.state = TabState::Foreground;
            t.state_since = now;
            t.last_active = now;
        }
        self.strip.selected = Some(id);
        match was {
            TabState::Discarded => {
                cmds.push(LifecycleCommand::Restore { tab: id, snapshot: snapshot.expect("tab exists") });
                cmds.push(LifecycleCommand::Show(id));
                cmds.push(self.apply_net(id, settings));
            }
            _ => {
                cmds.push(LifecycleCommand::Show(id));
                // Foreground policy releases everything that was held, in order.
                cmds.push(self.apply_net(id, settings));
                if settings.refresh_on_return_for_host(host.as_deref()) {
                    cmds.push(LifecycleCommand::RefreshOnReturn(id));
                }
            }
        }
        cmds
    }

    pub fn close(&mut self, id: TabId, settings: &Settings) -> Vec<LifecycleCommand> {
        let was_selected = self.strip.selected == Some(id);
        let next = self.strip.remove(id);
        let mut cmds = vec![LifecycleCommand::Close(id)];
        if was_selected {
            match next {
                Some(n) => cmds.extend(self.select(n, settings)),
                None => {
                    let (_, c) = self.open(None, true, settings);
                    cmds.extend(c);
                }
            }
        }
        cmds
    }

    pub fn reopen_closed(&mut self, settings: &Settings) -> Vec<LifecycleCommand> {
        let Some(snap) = self.strip.pop_closed() else { return vec![] };
        let (id, mut cmds) = self.open_from_snapshot(&snap, None);
        cmds.extend(self.select(id, settings));
        cmds
    }

    // -------------------------------------------------------- state changes

    fn apply_net(&self, id: TabId, settings: &Settings) -> LifecycleCommand {
        let t = self.strip.tab(id).expect("tab exists");
        LifecycleCommand::ApplyNetwork { tab: id, state: t.state, mode: settings.mode_for_host(t.host()) }
    }

    fn freeze(&mut self, id: TabId, settings: &Settings) -> Vec<LifecycleCommand> {
        let now = self.clock.now();
        match self.strip.tab_mut(id) {
            Some(t) if t.state == TabState::BackgroundWarm => {
                t.state = TabState::BackgroundFrozen;
                t.state_since = now;
            }
            _ => return vec![],
        }
        vec![self.apply_net(id, settings), LifecycleCommand::ReleaseImages(id)]
    }

    /// Discard a tab. `user_requested` bypasses the automatic protections (audible,
    /// dirty form) but never discards the foreground tab.
    pub fn discard(&mut self, id: TabId, user_requested: bool) -> Vec<LifecycleCommand> {
        let now = self.clock.now();
        let Some(t) = self.strip.tab_mut(id) else { return vec![] };
        if !t.state.is_background() {
            return vec![]; // foreground (and already-discarded) tabs are never discarded
        }
        if !user_requested && (t.audible || t.loading || t.dirty_form) {
            return vec![];
        }
        let snapshot = t.snapshot();
        t.state = TabState::Discarded;
        t.state_since = now;
        t.audible = false;
        t.loading = false;
        vec![LifecycleCommand::Discard { tab: id, snapshot }]
    }

    /// Time-driven transitions. Call about once a second.
    pub fn tick(&mut self, settings: &Settings, pressure: PressureLevel) -> Vec<LifecycleCommand> {
        let now = self.clock.now();
        let mut cmds = Vec::new();
        let ids: Vec<TabId> = self.strip.tabs().iter().map(|t| t.id).collect();
        for id in ids {
            let t = self.strip.tab(id).expect("exists");
            let mode = settings.mode_for_host(t.host());
            let idle = now.saturating_duration_since(t.last_active);
            match t.state {
                TabState::BackgroundWarm => {
                    let after = match mode {
                        BackgroundMode::Normal => None, // user said: keep it fully alive
                        BackgroundMode::SmartPause | BackgroundMode::StrictOffline => Some(settings.timings.freeze_after),
                        BackgroundMode::DeepSleep => Some(settings.timings.deep_sleep_freeze_after),
                    };
                    // Never freeze playing audio; let a loading page finish (up to 3× the delay).
                    if let Some(after) = after {
                        let blocked = t.audible || (t.loading && idle < after * 3);
                        if idle >= after && !blocked {
                            cmds.extend(self.freeze(id, settings));
                        }
                    }
                }
                TabState::BackgroundFrozen => {
                    // Time alone never discards: only when the system is also under strain.
                    let in_state = now.saturating_duration_since(t.state_since);
                    if pressure >= PressureLevel::Warn && in_state >= settings.timings.discard_after {
                        cmds.extend(self.discard(id, false));
                    }
                }
                _ => {}
            }
        }
        cmds
    }

    /// Execute governor decisions through the same state machine (and its vetoes).
    pub fn apply_governor(&mut self, actions: &[GovernorAction], settings: &Settings) -> Vec<LifecycleCommand> {
        let mut cmds = Vec::new();
        for a in actions {
            match *a {
                GovernorAction::TrimCaches(level) => cmds.push(LifecycleCommand::TrimCaches(level)),
                GovernorAction::ReleaseTabImages(id) => {
                    if self.strip.tab(id).is_some_and(|t| t.state.is_background()) {
                        cmds.push(LifecycleCommand::ReleaseImages(id));
                    }
                }
                GovernorAction::Freeze(id, _) => cmds.extend(self.freeze(id, settings)),
                GovernorAction::Discard(id, _) => cmds.extend(self.discard(id, false)),
            }
        }
        cmds
    }

    /// Re-install network policy on every background tab (settings changed).
    pub fn settings_changed(&self, settings: &Settings) -> Vec<LifecycleCommand> {
        self.strip.tabs().iter().filter(|t| t.state.is_background()).map(|t| self.apply_net(t.id, settings)).collect()
    }

    /// Governor input.
    pub fn infos(&self, private_bytes: impl Fn(TabId) -> browser_core::ByteSize) -> Vec<TabInfo> {
        self.strip
            .tabs()
            .iter()
            .map(|t| TabInfo {
                id: t.id,
                state: t.state,
                private: private_bytes(t.id),
                pinned: t.pinned,
                audible: t.audible,
                loading: t.loading,
                dirty_form: t.dirty_form,
                last_active: t.last_active,
            })
            .collect()
    }

    pub fn idle_for(&self, id: TabId) -> Option<Duration> {
        self.strip.tab(id).map(|t| self.clock.now().saturating_duration_since(t.last_active))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use browser_core::{ByteSize, ManualClock};
    use browser_memory::governor::Reason;
    use browser_settings::{SiteMode, SiteSettings};

    fn mgr() -> (TabLifecycleManager, ManualClock, Settings) {
        let clock = ManualClock::default();
        (TabLifecycleManager::new(Arc::new(clock.clone())), clock, Settings::default())
    }
    fn u(s: &str) -> Option<Url> {
        Some(Url::parse(s).unwrap())
    }
    /// Open a tab and report that its page finished loading, as the shell does.
    fn open_loaded(m: &mut TabLifecycleManager, url: &str, fg: bool, s: &Settings) -> TabId {
        let (id, _) = m.open(u(url), fg, s);
        m.set_loading(id, false);
        id
    }
    fn state(m: &TabLifecycleManager, id: TabId) -> TabState {
        m.strip.tab(id).unwrap().state
    }
    fn no_navigation(cmds: &[LifecycleCommand]) -> bool {
        !cmds.iter().any(|c| matches!(c, LifecycleCommand::Restore { .. } | LifecycleCommand::RefreshOnReturn(_) | LifecycleCommand::Create { .. }))
    }

    #[test]
    fn switching_backgrounds_old_tab_and_foregrounds_new_without_navigating() {
        let (mut m, _c, s) = mgr();
        let a = open_loaded(&mut m, "https://a.test/", true, &s);
        let b = open_loaded(&mut m, "https://b.test/", false, &s);
        assert_eq!((state(&m, a), state(&m, b)), (TabState::Foreground, TabState::BackgroundWarm));

        let cmds = m.select(b, &s);
        assert_eq!((state(&m, a), state(&m, b)), (TabState::BackgroundWarm, TabState::Foreground));
        assert_eq!(
            cmds,
            vec![
                LifecycleCommand::Hide(a),
                LifecycleCommand::ApplyNetwork { tab: a, state: TabState::BackgroundWarm, mode: BackgroundMode::SmartPause },
                LifecycleCommand::Show(b),
                LifecycleCommand::ApplyNetwork { tab: b, state: TabState::Foreground, mode: BackgroundMode::SmartPause },
            ]
        );
        assert!(no_navigation(&cmds), "tab switch must not create/reload/restore anything");
    }

    #[test]
    fn warm_tab_freezes_after_idle_and_returns_without_reload() {
        let (mut m, clock, s) = mgr();
        let a = open_loaded(&mut m, "https://a.test/", true, &s);
        let b = open_loaded(&mut m, "https://b.test/", true, &s); // a → warm
        clock.advance(Duration::from_secs(4 * 60));
        assert!(m.tick(&s, PressureLevel::Normal).is_empty());
        clock.advance(Duration::from_secs(2 * 60));
        let cmds = m.tick(&s, PressureLevel::Normal);
        assert_eq!(state(&m, a), TabState::BackgroundFrozen);
        assert!(cmds.contains(&LifecycleCommand::ReleaseImages(a)));
        assert_eq!(state(&m, b), TabState::Foreground, "foreground never touched by tick");

        let back = m.select(a, &s);
        assert_eq!(state(&m, a), TabState::Foreground);
        assert!(no_navigation(&back), "returning to a frozen tab must NOT reload: {back:?}");
        assert!(back.contains(&LifecycleCommand::Show(a)));
        assert!(back.contains(&LifecycleCommand::ApplyNetwork { tab: a, state: TabState::Foreground, mode: BackgroundMode::SmartPause }));
    }

    #[test]
    fn normal_mode_and_always_active_sites_are_never_frozen_by_time() {
        let (mut m, clock, mut s) = mgr();
        s.sites.insert("mail.test".into(), SiteSettings { mode: Some(SiteMode::AlwaysActive), refresh_on_return: false });
        let a = open_loaded(&mut m, "https://mail.test/", true, &s);
        let _b = open_loaded(&mut m, "https://b.test/", true, &s);
        clock.advance(Duration::from_secs(3600));
        m.tick(&s, PressureLevel::Normal);
        assert_eq!(state(&m, a), TabState::BackgroundWarm);
        let net = m.settings_changed(&s);
        assert!(net.contains(&LifecycleCommand::ApplyNetwork { tab: a, state: TabState::BackgroundWarm, mode: BackgroundMode::Normal }));
    }

    #[test]
    fn deep_sleep_freezes_sooner() {
        let (mut m, clock, mut s) = mgr();
        s.background_mode = BackgroundMode::DeepSleep;
        let a = open_loaded(&mut m, "https://a.test/", true, &s);
        open_loaded(&mut m, "https://b.test/", true, &s);
        clock.advance(Duration::from_secs(31));
        m.tick(&s, PressureLevel::Normal);
        assert_eq!(state(&m, a), TabState::BackgroundFrozen);
    }

    #[test]
    fn audible_tabs_are_not_frozen_and_loading_tabs_get_extra_time() {
        let (mut m, clock, s) = mgr();
        let a = open_loaded(&mut m, "https://a.test/", true, &s);
        let b = open_loaded(&mut m, "https://b.test/", true, &s);
        let c = open_loaded(&mut m, "https://c.test/", true, &s);
        m.strip.tab_mut(a).unwrap().audible = true;
        m.set_loading(b, true);
        clock.advance(Duration::from_secs(6 * 60));
        m.tick(&s, PressureLevel::Normal);
        assert_eq!(state(&m, a), TabState::BackgroundWarm, "audible");
        assert_eq!(state(&m, b), TabState::BackgroundWarm, "still loading");
        clock.advance(Duration::from_secs(10 * 60));
        m.tick(&s, PressureLevel::Normal);
        assert_eq!(state(&m, b), TabState::BackgroundFrozen, "loading grace is bounded");
        assert_eq!(state(&m, a), TabState::BackgroundWarm);
        let _ = c;
    }

    #[test]
    fn foreground_can_never_be_discarded_or_frozen_even_by_the_governor_or_user() {
        let (mut m, _c, s) = mgr();
        let a = open_loaded(&mut m, "https://a.test/", true, &s);
        assert!(m.discard(a, true).is_empty());
        let acts = [GovernorAction::Discard(a, Reason::PressureCritical), GovernorAction::Freeze(a, Reason::PressureCritical), GovernorAction::ReleaseTabImages(a)];
        assert!(m.apply_governor(&acts, &s).is_empty());
        assert_eq!(state(&m, a), TabState::Foreground);
    }

    #[test]
    fn discard_keeps_a_snapshot_and_selecting_restores_it() {
        let (mut m, _c, s) = mgr();
        let a = open_loaded(&mut m, "https://a.test/page", true, &s);
        {
            let t = m.strip.tab_mut(a).unwrap();
            t.title = "Page".into();
            t.history = vec!["https://a.test/".into(), "https://a.test/page".into()];
            t.history_index = 1;
            t.scroll = (0.0, 1234.0);
        }
        let b = open_loaded(&mut m, "https://b.test/", true, &s);
        let cmds = m.discard(a, false);
        assert_eq!(state(&m, a), TabState::Discarded);
        let LifecycleCommand::Discard { snapshot, .. } = &cmds[0] else { panic!("{cmds:?}") };
        assert_eq!(snapshot.scroll_y, 1234.0);
        assert_eq!(snapshot.restore_url(), Some("https://a.test/page"));

        let back = m.select(a, &s);
        assert!(matches!(&back[..], [LifecycleCommand::Hide(_), LifecycleCommand::ApplyNetwork { .. }, LifecycleCommand::Restore { .. }, LifecycleCommand::Show(_), LifecycleCommand::ApplyNetwork { .. }]), "{back:?}");
        assert_eq!(state(&m, a), TabState::Foreground);
        assert_eq!(state(&m, b), TabState::BackgroundWarm);
    }

    #[test]
    fn automatic_discard_respects_protections_but_user_discard_does_not() {
        let (mut m, _c, s) = mgr();
        let a = open_loaded(&mut m, "https://a.test/", true, &s);
        open_loaded(&mut m, "https://b.test/", true, &s);
        m.strip.tab_mut(a).unwrap().dirty_form = true;
        assert!(m.discard(a, false).is_empty());
        assert_eq!(m.discard(a, true).len(), 1);
        assert_eq!(state(&m, a), TabState::Discarded);
    }

    #[test]
    fn time_alone_never_discards_but_time_plus_pressure_does() {
        let (mut m, clock, s) = mgr();
        let a = open_loaded(&mut m, "https://a.test/", true, &s);
        open_loaded(&mut m, "https://b.test/", true, &s);
        clock.advance(Duration::from_secs(10 * 60));
        m.tick(&s, PressureLevel::Normal);
        assert_eq!(state(&m, a), TabState::BackgroundFrozen);
        clock.advance(Duration::from_secs(3 * 3600));
        m.tick(&s, PressureLevel::Normal);
        assert_eq!(state(&m, a), TabState::BackgroundFrozen, "no pressure → stays frozen, page intact");
        m.tick(&s, PressureLevel::Warn);
        assert_eq!(state(&m, a), TabState::Discarded);
    }

    #[test]
    fn refresh_on_return_is_opt_in_per_site() {
        let (mut m, _c, mut s) = mgr();
        let a = open_loaded(&mut m, "https://news.test/", true, &s);
        let b = open_loaded(&mut m, "https://other.test/", true, &s);
        assert!(!m.select(a, &s).contains(&LifecycleCommand::RefreshOnReturn(a)), "off by default");
        m.select(b, &s);
        s.sites.insert("news.test".into(), SiteSettings { mode: None, refresh_on_return: true });
        assert!(m.select(a, &s).contains(&LifecycleCommand::RefreshOnReturn(a)));
    }

    #[test]
    fn closing_selected_tab_selects_neighbour_and_can_be_reopened() {
        let (mut m, _c, s) = mgr();
        let a = open_loaded(&mut m, "https://a.test/", true, &s);
        let b = open_loaded(&mut m, "https://b.test/", true, &s);
        let c = open_loaded(&mut m, "https://c.test/", true, &s);
        m.select(b, &s);
        let cmds = m.close(b, &s);
        assert!(cmds.contains(&LifecycleCommand::Close(b)));
        assert_eq!(m.strip.selected, Some(c), "right neighbour");
        assert_eq!(state(&m, c), TabState::Foreground);
        let re = m.reopen_closed(&s);
        assert!(re.iter().any(|c| matches!(c, LifecycleCommand::Restore { .. })));
        assert_eq!(m.strip.len(), 3);
        let _ = a;
    }

    #[test]
    fn closing_the_last_tab_opens_a_fresh_one() {
        let (mut m, _c, s) = mgr();
        let a = open_loaded(&mut m, "https://a.test/", true, &s);
        let cmds = m.close(a, &s);
        assert!(cmds.iter().any(|c| matches!(c, LifecycleCommand::Create { .. })));
        assert_eq!(m.strip.len(), 1);
        assert!(m.strip.selected.is_some());
    }

    #[test]
    fn strip_keeps_pinned_tabs_first_and_clamps_drag_and_drop() {
        let (mut m, _c, s) = mgr();
        let ids: Vec<TabId> = (0..4).map(|i| m.open(u(&format!("https://t{i}.test/")), true, &s).0).collect();
        m.strip.set_pinned(ids[2], true);
        assert_eq!(m.strip.tabs()[0].id, ids[2]);
        m.strip.move_tab(ids[0], 0); // unpinned dropped into pinned region
        assert_eq!(m.strip.tabs()[0].id, ids[2], "clamped behind pinned tabs");
        m.strip.move_tab(ids[2], 3); // pinned dragged past unpinned
        assert_eq!(m.strip.tabs()[0].id, ids[2], "pinned stays in pinned region");
        m.strip.set_pinned(ids[2], false);
        assert!(m.strip.tabs().iter().all(|t| !t.pinned));
    }

    #[test]
    fn groups_workspaces_and_duplicate() {
        let (mut m, _c, s) = mgr();
        let a = open_loaded(&mut m, "https://a.test/", true, &s);
        let g = m.strip.add_group("Work", [10, 20, 30]);
        m.strip.set_group(a, Some(g));
        assert_eq!(m.strip.groups.len(), 1);
        m.strip.set_group(a, None);
        assert!(m.strip.groups.is_empty(), "empty groups are dropped");
        let ws = m.strip.add_workspace("Research");
        m.strip.active_workspace = ws;
        let b = open_loaded(&mut m, "https://b.test/", true, &s);
        assert_eq!(m.strip.visible().map(|t| t.id).collect::<Vec<_>>(), vec![b]);
        let (url, snap) = m.strip.duplicate_of(b).unwrap();
        assert_eq!(url.unwrap().as_str(), "https://b.test/");
        assert_eq!(snap.workspace, ws.0);
    }

    #[test]
    fn governor_integration_freezes_then_discards_through_the_state_machine() {
        let (mut m, clock, s) = mgr();
        let a = open_loaded(&mut m, "https://a.test/", true, &s);
        open_loaded(&mut m, "https://b.test/", true, &s);
        clock.advance(Duration::from_secs(60));
        let cmds = m.apply_governor(&[GovernorAction::Freeze(a, Reason::PressureWarn)], &s);
        assert!(cmds.contains(&LifecycleCommand::ReleaseImages(a)));
        assert_eq!(state(&m, a), TabState::BackgroundFrozen);
        let cmds = m.apply_governor(&[GovernorAction::Discard(a, Reason::PressureCritical)], &s);
        assert!(matches!(cmds[0], LifecycleCommand::Discard { .. }));
        // infos() feeds the governor from the same model.
        let infos = m.infos(|_| ByteSize::mib(10));
        assert_eq!(infos.iter().find(|i| i.id == a).unwrap().state, TabState::Discarded);
    }
}
