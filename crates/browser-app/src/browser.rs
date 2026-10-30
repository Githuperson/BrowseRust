use crate::omnibox::{Resolved, resolve_input};
use browser_core::{ByteSize, Clock, TabId, TabState};
use browser_engine::{Engine, EngineEvent, LoadState};
use browser_memory::probe::{MemoryProbe, ProcessMemory};
use browser_memory::{Category, GovernorConfig, MemoryBudget, MemoryGovernor, MemoryLedger, PressureLevel, PressureTracker, Snapshot, SystemMemory};
use browser_security::{NavVerdict, PopupBlocker, check_navigation};
use browser_session::{SavedTab, SessionStore};
use browser_settings::{BackgroundMode, Settings};
use browser_tabs::{LifecycleCommand, TabLifecycleManager};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use url::Url;

/// Latest memory picture, for the UI and for benchmarks.
#[derive(Clone, Debug)]
pub struct MemoryReport {
    pub pressure: PressureLevel,
    pub browser_total: ByteSize,
    pub budget: MemoryBudget,
    pub system: Option<SystemMemory>,
    pub process: Option<ProcessMemory>,
    pub tab_private: Vec<(TabId, ByteSize)>,
}

pub struct Browser<E: Engine> {
    pub engine: E,
    pub settings: Settings,
    pub tabs: TabLifecycleManager,
    pub popups: PopupBlocker,
    clock: Arc<dyn Clock>,
    ledger: MemoryLedger,
    governor: MemoryGovernor,
    pressure: PressureTracker,
    probe: Box<dyn MemoryProbe>,
    last_sample: Option<Instant>,
    pub sample_every: Duration,
    pending_scroll: HashMap<TabId, (f32, f32)>,
    report: Option<MemoryReport>,
}

impl<E: Engine> Browser<E> {
    pub fn new(engine: E, settings: Settings, clock: Arc<dyn Clock>, probe: Box<dyn MemoryProbe>) -> Self {
        Self {
            engine,
            settings,
            tabs: TabLifecycleManager::new(clock.clone()),
            popups: PopupBlocker::new(clock.clone()),
            clock,
            ledger: MemoryLedger::default(),
            governor: MemoryGovernor::new(GovernorConfig::default()),
            pressure: PressureTracker::new(3),
            probe,
            last_sample: None,
            sample_every: Duration::from_secs(2),
            pending_scroll: HashMap::new(),
            report: None,
        }
    }

    // ------------------------------------------------------------------ tabs

    pub fn open_tab(&mut self, url: Option<Url>, foreground: bool) -> TabId {
        let (id, cmds) = self.tabs.open(url, foreground, &self.settings);
        self.execute(cmds);
        id
    }

    pub fn select_tab(&mut self, id: TabId) {
        if let Some(prev) = self.tabs.strip.selected.filter(|p| *p != id) {
            // Capture scroll / dirty-form state while the page is still foreground.
            self.engine.request_page_state(prev);
        }
        let cmds = self.tabs.select(id, &self.settings);
        self.execute(cmds);
    }

    pub fn close_tab(&mut self, id: TabId) {
        self.popups.forget(id);
        self.pending_scroll.remove(&id);
        self.ledger.forget(id);
        let cmds = self.tabs.close(id, &self.settings);
        self.execute(cmds);
    }

    pub fn reopen_closed_tab(&mut self) {
        let cmds = self.tabs.reopen_closed(&self.settings);
        self.execute(cmds);
    }

    /// User-requested discard (context menu). Never discards the selected tab.
    pub fn discard_tab(&mut self, id: TabId) {
        self.engine.request_page_state(id);
        let cmds = self.tabs.discard(id, true);
        self.execute(cmds);
    }

    pub fn duplicate_tab(&mut self, id: TabId) -> Option<TabId> {
        let (url, _snap) = self.tabs.strip.duplicate_of(id)?;
        Some(self.open_tab(url, true))
    }

    pub fn set_pinned(&mut self, id: TabId, pinned: bool) {
        self.tabs.strip.set_pinned(id, pinned);
    }

    pub fn set_muted(&mut self, id: TabId, muted: bool) {
        self.tabs.strip.set_muted(id, muted);
    }

    // ------------------------------------------------------------ navigation

    /// Handle text typed in the address bar for the selected tab.
    pub fn navigate_input(&mut self, input: &str) -> Result<(), String> {
        let Some(tab) = self.tabs.strip.selected else { return Err("no tab selected".into()) };
        let resolved = resolve_input(input, &self.settings.search_template).ok_or("empty input")?;
        let url = match &resolved {
            Resolved::Navigate(u) | Resolved::Search(u) => u.clone(),
        };
        match check_navigation(&url, true, false) {
            NavVerdict::Allow => {}
            NavVerdict::Block(why) => return Err(why.to_string()),
        }
        self.engine.begin_user_navigation(tab);
        self.engine.load(tab, &url);
        Ok(())
    }

    pub fn reload(&mut self) {
        if let Some(t) = self.tabs.strip.selected {
            self.engine.begin_user_navigation(t);
            self.engine.reload(t);
        }
    }
    pub fn stop(&mut self) {
        if let Some(t) = self.tabs.strip.selected {
            self.engine.stop(t);
        }
    }
    pub fn go_back(&mut self) {
        if let Some(t) = self.tabs.strip.selected {
            self.engine.begin_user_navigation(t);
            self.engine.go_back(t);
        }
    }
    pub fn go_forward(&mut self) {
        if let Some(t) = self.tabs.strip.selected {
            self.engine.begin_user_navigation(t);
            self.engine.go_forward(t);
        }
    }

    // ---------------------------------------------------------------- events

    /// Drain and apply engine events. Call after pumping the engine.
    pub fn pump_events(&mut self) {
        for ev in self.engine.poll_events() {
            self.handle_event(ev);
        }
    }

    fn handle_event(&mut self, ev: EngineEvent) {
        match ev {
            EngineEvent::UrlChanged(tab, url) => {
                if let Some(t) = self.tabs.strip.tab_mut(tab) {
                    t.url = Some(url);
                }
            }
            EngineEvent::TitleChanged(tab, title) => {
                if let Some(t) = self.tabs.strip.tab_mut(tab) {
                    t.title = title.unwrap_or_default();
                }
            }
            EngineEvent::Load(tab, state) => {
                self.tabs.set_loading(tab, state != LoadState::Complete);
                if state == LoadState::Complete {
                    if let Some((x, y)) = self.pending_scroll.remove(&tab) {
                        self.engine.restore_scroll(tab, x, y);
                    }
                }
            }
            EngineEvent::HistoryChanged { tab, entries, current } => {
                if let Some(t) = self.tabs.strip.tab_mut(tab) {
                    t.history = entries.iter().map(Url::to_string).collect();
                    t.history_index = current;
                }
            }
            EngineEvent::AudibleChanged(tab, a) => {
                if let Some(t) = self.tabs.strip.tab_mut(tab) {
                    t.audible = a;
                }
            }
            EngineEvent::PageState { tab, scroll, dirty_form } => {
                if let Some(t) = self.tabs.strip.tab_mut(tab) {
                    t.scroll = scroll;
                    t.dirty_form = dirty_form;
                }
            }
            EngineEvent::NewTabRequested { from, url } => {
                let allowed_target = url.as_ref().is_none_or(|u| check_navigation(u, false, true) == NavVerdict::Allow);
                if allowed_target && self.popups.allow_popup(from) {
                    self.open_tab(url, true);
                }
            }
            EngineEvent::TabOpenedByPage { tab, .. } => {
                let cmds = self.tabs.adopt(tab, true, &self.settings);
                self.execute(cmds);
            }
            EngineEvent::FaviconChanged(_) | EngineEvent::FrameReady(_) | EngineEvent::FullscreenChanged(..) | EngineEvent::Crashed(..) => {}
        }
    }

    // ------------------------------------------------------------- execution

    pub fn execute(&mut self, cmds: Vec<LifecycleCommand>) {
        for c in cmds {
            match c {
                LifecycleCommand::Create { tab, url } => self.engine.create_tab(tab, url.as_ref()),
                LifecycleCommand::Show(t) => self.engine.show(t),
                LifecycleCommand::Hide(t) => self.engine.hide(t),
                LifecycleCommand::ApplyNetwork { tab, state, mode } => self.engine.apply_network(tab, state, mode),
                LifecycleCommand::ReleaseImages(t) => {
                    self.engine.release_images(t);
                }
                LifecycleCommand::TrimCaches(level) => self.engine.trim_caches(level),
                LifecycleCommand::Discard { tab, .. } => {
                    self.engine.destroy_tab(tab);
                    self.ledger.forget(tab);
                }
                LifecycleCommand::Restore { tab, snapshot } => {
                    let url = snapshot.restore_url().and_then(|u| Url::parse(u).ok());
                    self.engine.create_tab(tab, url.as_ref());
                    if snapshot.scroll_y > 0.0 || snapshot.scroll_x > 0.0 {
                        self.pending_scroll.insert(tab, (snapshot.scroll_x, snapshot.scroll_y));
                    }
                }
                LifecycleCommand::RefreshOnReturn(t) => {
                    self.engine.begin_user_navigation(t);
                    self.engine.reload(t);
                }
                LifecycleCommand::Close(t) => self.engine.destroy_tab(t),
            }
        }
    }

    // ----------------------------------------------------------------- ticks

    /// Call about once per second.
    pub fn tick(&mut self) {
        self.engine.housekeeping();
        self.pump_events();
        let pressure = self.pressure.level();
        let cmds = self.tabs.tick(&self.settings, pressure);
        self.execute(cmds);

        let now = self.clock.now();
        if self.last_sample.is_none_or(|t| now.saturating_duration_since(t) >= self.sample_every) {
            self.last_sample = Some(now);
            self.sample_and_govern();
        }
    }

    fn sample_and_govern(&mut self) {
        let sys = self.probe.system();
        let proc_mem = self.probe.process();
        if let Some(p) = &proc_mem {
            self.ledger.set_process_footprint(p.best_unique());
        }
        let ids: Vec<TabId> = self.tabs.strip.tabs().iter().filter(|t| t.state.has_webview()).map(|t| t.id).collect();
        for id in ids {
            if let Some(s) = self.engine.tab_memory(id) {
                self.ledger.set_private(id, Category::Other, s.private);
                self.ledger.set_shared(id, s.shared);
            }
        }

        let raw = PressureTracker::raw_level(self.probe.os_pressure(), sys.unwrap_or(SystemMemory { total: ByteSize(1), available: ByteSize(1) }));
        let level = self.pressure.update(raw);
        let total = self.ledger.browser_total();
        let budget = MemoryBudget::adaptive(sys.unwrap_or_default(), total, &self.settings.memory);

        let ledger = &self.ledger;
        let infos = self.tabs.infos(|id| ledger.tab_private(id));
        let actions = self.governor.evaluate(&Snapshot { now: self.clock.now(), pressure: level, browser_total: total, budget, tabs: &infos });
        let tab_private = infos.iter().filter(|i| i.state.has_webview()).map(|i| (i.id, i.private)).collect();
        self.report = Some(MemoryReport { pressure: level, browser_total: total, budget, system: sys, process: proc_mem, tab_private });

        let cmds = self.tabs.apply_governor(&actions, &self.settings);
        self.execute(cmds);
    }

    pub fn memory_report(&self) -> Option<&MemoryReport> {
        self.report.as_ref()
    }

    pub fn set_background_mode(&mut self, mode: BackgroundMode) {
        self.settings.background_mode = mode;
        let cmds = self.tabs.settings_changed(&self.settings);
        self.execute(cmds);
    }

    // --------------------------------------------------------------- session

    pub fn save_session(&self, store: &SessionStore) -> Result<(), browser_session::SessionError> {
        let tabs = self
            .tabs
            .strip
            .tabs()
            .iter()
            .filter(|t| t.url.is_some())
            .map(|t| SavedTab { id: t.id, snapshot: t.snapshot() })
            .collect();
        store.save(tabs, self.tabs.strip.selected)
    }

    /// Re-create tabs from a saved session as Discarded (zero renderer memory),
    /// then select the previously selected one (which loads it).
    pub fn restore_session(&mut self, store: &SessionStore) -> usize {
        let file = store.load();
        let mut selected = None;
        for saved in &file.tabs {
            let (id, _) = self.tabs.open_from_snapshot(&saved.snapshot, None);
            if file.selected == Some(saved.id) {
                selected = Some(id);
            }
        }
        if let Some(id) = selected.or_else(|| self.tabs.strip.tabs().first().map(|t| t.id)) {
            self.select_tab(id);
        }
        file.tabs.len()
    }

    pub fn state_of(&self, id: TabId) -> Option<TabState> {
        self.tabs.strip.tab(id).map(|t| t.state)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use browser_core::ManualClock;
    use browser_engine::{Call, MockEngine, TabMemorySample};
    use browser_memory::probe::MemoryProbe;
    use parking_lot::Mutex;

    struct FakeProbe {
        sys: Mutex<SystemMemory>,
        pressure: Mutex<Option<PressureLevel>>,
        footprint: Mutex<u64>,
    }
    struct SharedProbe(Arc<FakeProbe>);
    impl MemoryProbe for SharedProbe {
        fn system(&self) -> Option<SystemMemory> {
            Some(*self.0.sys.lock())
        }
        fn process(&self) -> Option<ProcessMemory> {
            let f = *self.0.footprint.lock();
            Some(ProcessMemory { rss: ByteSize(f), pss: Some(ByteSize(f)), private: None, footprint: None })
        }
        fn os_pressure(&self) -> Option<PressureLevel> {
            *self.0.pressure.lock()
        }
    }

    struct Fx {
        b: Browser<MockEngine>,
        clock: ManualClock,
        probe: Arc<FakeProbe>,
    }
    fn fx() -> Fx {
        let clock = ManualClock::default();
        let probe = Arc::new(FakeProbe {
            sys: Mutex::new(SystemMemory { total: ByteSize::gib(8), available: ByteSize::gib(5) }),
            pressure: Mutex::new(None),
            footprint: Mutex::new(ByteSize::mib(500).0),
        });
        let b = Browser::new(MockEngine::default(), Settings::default(), Arc::new(clock.clone()), Box::new(SharedProbe(probe.clone())));
        Fx { b, clock, probe }
    }
    fn u(s: &str) -> Url {
        Url::parse(s).unwrap()
    }
    fn open_loaded(f: &mut Fx, url: &str, fg: bool) -> TabId {
        let id = f.b.open_tab(Some(u(url)), fg);
        f.b.engine.events.push(EngineEvent::Load(id, LoadState::Complete));
        f.b.pump_events();
        id
    }

    #[test]
    fn tab_switch_hides_and_shows_without_any_page_load() {
        let mut f = fx();
        let a = open_loaded(&mut f, "https://a.test/", true);
        let b = open_loaded(&mut f, "https://b.test/", true);
        f.b.engine.take_calls();
        f.b.select_tab(a);
        let calls = f.b.engine.take_calls();
        assert!(!f.b.engine.loaded_anything());
        assert!(calls.contains(&Call::Hide(b)) && calls.contains(&Call::Show(a)), "{calls:?}");
        assert!(!calls.iter().any(|c| matches!(c, Call::Load(..) | Call::Reload(_) | Call::Create(..) | Call::Destroy(_))), "{calls:?}");
        assert!(calls.contains(&Call::ApplyNetwork(a, TabState::Foreground, BackgroundMode::SmartPause)));
    }

    #[test]
    fn idle_tab_is_frozen_with_stricter_network_and_returns_without_reload() {
        let mut f = fx();
        let a = open_loaded(&mut f, "https://a.test/", true);
        let _b = open_loaded(&mut f, "https://b.test/", true);
        f.b.engine.take_calls();
        f.clock.advance(Duration::from_secs(6 * 60));
        f.b.tick();
        let calls = f.b.engine.take_calls();
        assert_eq!(f.b.state_of(a), Some(TabState::BackgroundFrozen));
        assert!(calls.contains(&Call::ApplyNetwork(a, TabState::BackgroundFrozen, BackgroundMode::SmartPause)));
        assert!(calls.contains(&Call::ReleaseImages(a)));
        f.b.select_tab(a);
        assert!(!f.b.engine.loaded_anything(), "returning to a frozen tab must not load anything");
    }

    #[test]
    fn critical_pressure_discards_largest_idle_background_tab_never_the_foreground() {
        let mut f = fx();
        let big = open_loaded(&mut f, "https://big.test/", true);
        let fg = open_loaded(&mut f, "https://fg.test/", true);
        f.b.engine.memory.insert(big, TabMemorySample { private: ByteSize::mib(600), shared: ByteSize(0) });
        f.b.engine.memory.insert(fg, TabMemorySample { private: ByteSize::mib(900), shared: ByteSize(0) });
        *f.probe.pressure.lock() = Some(PressureLevel::Critical);
        *f.probe.footprint.lock() = ByteSize::gib(3).0;
        f.clock.advance(Duration::from_secs(60));
        f.b.engine.take_calls();
        f.b.tick();
        let calls = f.b.engine.take_calls();
        assert!(calls.contains(&Call::Destroy(big)), "{calls:?}");
        assert!(!calls.contains(&Call::Destroy(fg)));
        assert_eq!(f.b.state_of(big), Some(TabState::Discarded));
        assert_eq!(f.b.state_of(fg), Some(TabState::Foreground));
        assert!(calls.contains(&Call::Trim(browser_memory::TrimLevel::Aggressive)));
    }

    #[test]
    fn discarded_tab_restores_url_and_scroll_on_select() {
        let mut f = fx();
        let a = open_loaded(&mut f, "https://a.test/long", true);
        f.b.engine.events.push(EngineEvent::PageState { tab: a, scroll: (0.0, 777.0), dirty_form: false });
        f.b.pump_events();
        let _b = open_loaded(&mut f, "https://b.test/", true);
        f.b.discard_tab(a);
        f.b.engine.take_calls();
        f.b.select_tab(a);
        let calls = f.b.engine.take_calls();
        assert!(calls.contains(&Call::Create(a, Some(u("https://a.test/long")))), "{calls:?}");
        f.b.engine.events.push(EngineEvent::Load(a, LoadState::Complete));
        f.b.pump_events();
        assert!(f.b.engine.take_calls().contains(&Call::RestoreScroll(a, 0.0, 777.0)));
    }

    #[test]
    fn navigation_is_marked_user_initiated_before_load_and_unsafe_schemes_are_refused() {
        let mut f = fx();
        let a = open_loaded(&mut f, "https://a.test/", true);
        f.b.engine.take_calls();
        f.b.navigate_input("example.com").unwrap();
        assert_eq!(f.b.engine.take_calls(), vec![Call::UserNav(a), Call::Load(a, u("https://example.com/"))]);
        assert!(f.b.navigate_input("javascript:alert(1)").is_err());
        assert!(f.b.navigate_input("rust lifetimes").is_ok());
        assert!(f.b.engine.calls.iter().all(|c| !matches!(c, Call::Load(_, url) if url.scheme() == "javascript")));
    }

    #[test]
    fn popups_need_a_recent_user_gesture_and_a_safe_target() {
        let mut f = fx();
        let a = open_loaded(&mut f, "https://a.test/", true);
        let before = f.b.tabs.strip.len();
        f.b.engine.events.push(EngineEvent::NewTabRequested { from: a, url: Some(u("https://ad.test/")) });
        f.b.pump_events();
        assert_eq!(f.b.tabs.strip.len(), before, "no gesture → blocked");
        f.b.popups.note_user_input(a);
        f.b.engine.events.push(EngineEvent::NewTabRequested { from: a, url: Some(u("file:///etc/passwd")) });
        f.b.pump_events();
        assert_eq!(f.b.tabs.strip.len(), before, "gesture but unsafe target → blocked");
        f.b.engine.events.push(EngineEvent::NewTabRequested { from: a, url: Some(u("https://ok.test/")) });
        f.b.pump_events();
        assert_eq!(f.b.tabs.strip.len(), before + 1);
    }

    #[test]
    fn changing_background_mode_reinstalls_network_policy_on_background_tabs() {
        let mut f = fx();
        let a = open_loaded(&mut f, "https://a.test/", true);
        let _b = open_loaded(&mut f, "https://b.test/", true);
        f.b.engine.take_calls();
        f.b.set_background_mode(BackgroundMode::StrictOffline);
        assert_eq!(f.b.engine.take_calls(), vec![Call::ApplyNetwork(a, TabState::BackgroundWarm, BackgroundMode::StrictOffline)]);
    }

    #[test]
    fn memory_report_uses_measured_footprint_and_adaptive_budget() {
        let mut f = fx();
        open_loaded(&mut f, "https://a.test/", true);
        f.b.tick();
        let r = f.b.memory_report().expect("sampled");
        assert_eq!(r.browser_total, ByteSize::mib(500));
        assert_eq!(r.pressure, PressureLevel::Normal);
        assert!(r.budget.global_limit > ByteSize::gib(3) && r.budget.global_limit < ByteSize::gib(4));
    }

    #[test]
    fn session_round_trip_restores_tabs_discarded_and_loads_only_the_selected_one() {
        let dir = std::env::temp_dir().join(format!("browserust-app-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        let store = SessionStore::new(&dir);
        let mut f = fx();
        let _a = open_loaded(&mut f, "https://a.test/", true);
        let b = open_loaded(&mut f, "https://b.test/", true);
        f.b.save_session(&store).unwrap();

        let mut g = fx();
        assert_eq!(g.b.restore_session(&store), 2);
        let creates: Vec<_> = g.b.engine.calls.iter().filter(|c| matches!(c, Call::Create(..))).collect();
        assert_eq!(creates.len(), 1, "only the selected tab gets a renderer: {creates:?}");
        assert!(matches!(creates[0], Call::Create(_, Some(url)) if url.as_str() == "https://b.test/"));
        assert_eq!(g.b.tabs.strip.count_in_state(TabState::Discarded), 1);
        let _ = b;
        std::fs::remove_dir_all(dir).ok();
    }
}
