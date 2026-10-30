use crate::classify::PollingDetector;
use crate::controller::{ControllerConfig, NetworkActivityController, Outcome, Resumed};
use crate::monitor::NetworkMonitor;
use crate::policy::NetworkPolicy;
use crate::request::{Destination, RequestInfo};
use browser_core::{Clock, TabId, TabState};
use browser_settings::BackgroundMode;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// How long after a browser-initiated navigation a main-frame load counts as
/// "user initiated" (covers redirects, back/forward and reload).
const USER_NAV_WINDOW: Duration = Duration::from_secs(30);

/// Owns one [`NetworkActivityController`] per tab plus the state shared between
/// them (poll detection). Engine adapters talk only to the hub.
///
/// The `user_initiated` flag on a request is **never** taken from the engine or
/// from page content: the hub sets it from the browser's own record of
/// navigations the user started ([`NetworkHub::begin_user_navigation`]).
pub struct NetworkHub<H> {
    clock: Arc<dyn Clock>,
    cfg: ControllerConfig,
    policy: NetworkPolicy,
    polling: PollingDetector,
    tabs: HashMap<TabId, NetworkActivityController<H>>,
    user_nav: HashMap<TabId, Instant>,
}

impl<H> NetworkHub<H> {
    pub fn new(clock: Arc<dyn Clock>, cfg: ControllerConfig, policy: NetworkPolicy) -> Self {
        Self { polling: PollingDetector::new(clock.clone()), clock, cfg, policy, tabs: HashMap::new(), user_nav: HashMap::new() }
    }

    pub fn add_tab(&mut self, tab: TabId) {
        self.tabs
            .entry(tab)
            .or_insert_with(|| NetworkActivityController::new(tab, self.clock.clone(), self.cfg.clone(), self.policy.clone()));
    }

    /// Cancel everything held for a closed tab and forget it.
    pub fn remove_tab(&mut self, tab: TabId) -> Vec<Outcome<H>> {
        self.polling.forget_tab(tab);
        self.user_nav.remove(&tab);
        self.tabs.remove(&tab).map(|mut c| c.close()).unwrap_or_default()
    }

    pub fn begin_user_navigation(&mut self, tab: TabId) {
        self.user_nav.insert(tab, self.clock.now() + USER_NAV_WINDOW);
        if let Some(c) = self.tabs.get_mut(&tab) {
            c.begin_user_navigation();
        }
    }

    /// Is a browser-started navigation currently in flight for this tab?
    pub fn user_nav_active(&self, tab: TabId) -> bool {
        self.user_nav.get(&tab).is_some_and(|until| self.clock.now() < *until)
    }

    pub fn load_complete(&mut self, tab: TabId) {
        self.user_nav.remove(&tab);
        if let Some(c) = self.tabs.get_mut(&tab) {
            c.load_complete();
        }
    }

    pub fn set_state(&mut self, tab: TabId, state: TabState, mode: BackgroundMode) -> Resumed<H> {
        match self.tabs.get_mut(&tab) {
            Some(c) => c.set_state(state, mode),
            None => Resumed { release: vec![], cancel: vec![] },
        }
    }

    /// Entry point for every outgoing request of a tab. An unknown tab is not
    /// restricted (fail open for navigation, never leak a handle).
    pub fn on_request(&mut self, tab: TabId, mut info: RequestInfo, handle: H) -> Vec<Outcome<H>> {
        if info.is_main_frame && info.destination == Destination::Document {
            info.user_initiated = self.user_nav.get(&tab).is_some_and(|until| self.clock.now() < *until);
        } else {
            info.user_initiated = false;
        }
        let is_candidate = !info.is_main_frame && matches!(info.destination, Destination::Empty | Destination::Other);
        let polling = is_candidate && info.is_idempotent() && self.polling.observe(tab, &info);
        match self.tabs.get_mut(&tab) {
            Some(c) => c.on_request(&info, polling, handle),
            None => vec![Outcome::Proceed(handle)],
        }
    }

    /// Expire stale holds in every tab and prune poll history.
    pub fn tick(&mut self) -> Vec<Outcome<H>> {
        self.polling.gc();
        self.tabs.values_mut().flat_map(|c| c.tick()).collect()
    }

    pub fn monitor(&self, tab: TabId) -> Option<&NetworkMonitor> {
        self.tabs.get(&tab).map(|c| c.monitor())
    }

    pub fn held_count(&self, tab: TabId) -> usize {
        self.tabs.get(&tab).map_or(0, |c| c.held_count())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use browser_core::ManualClock;
    use url::Url;

    fn hub() -> (NetworkHub<u32>, ManualClock) {
        let c = ManualClock::default();
        (NetworkHub::new(Arc::new(c.clone()), ControllerConfig::default(), NetworkPolicy::default()), c)
    }
    fn doc(u: &str) -> RequestInfo {
        let mut r = RequestInfo::new("GET", Url::parse(u).unwrap(), Destination::Document);
        r.is_main_frame = true;
        r
    }
    fn fetch(u: &str) -> RequestInfo {
        RequestInfo::new("GET", Url::parse(u).unwrap(), Destination::Empty)
    }

    #[test]
    fn page_cannot_forge_user_initiated() {
        let (mut h, _c) = hub();
        let t = TabId(1);
        h.add_tab(t);
        h.set_state(t, TabState::BackgroundWarm, BackgroundMode::SmartPause);
        let mut forged = doc("https://evil.test/");
        forged.user_initiated = true; // engine/page supplied: must be ignored
        assert!(h.on_request(t, forged, 1).is_empty(), "automatic navigation in a background tab is held");
        assert_eq!(h.held_count(t), 1);
    }

    #[test]
    fn browser_started_navigation_proceeds_even_in_background() {
        let (mut h, c) = hub();
        let t = TabId(1);
        h.add_tab(t);
        h.set_state(t, TabState::BackgroundWarm, BackgroundMode::StrictOffline);
        h.begin_user_navigation(t);
        assert_eq!(h.on_request(t, doc("https://a.test/"), 1), vec![Outcome::Proceed(1)]);
        h.load_complete(t);
        c.advance(Duration::from_secs(1));
        assert!(h.on_request(t, doc("https://a.test/refresh"), 2).is_empty(), "after load, window is closed");
    }

    #[test]
    fn regular_polling_is_detected_then_held_in_background() {
        let (mut h, c) = hub();
        let t = TabId(1);
        h.add_tab(t);
        // Foreground: three regular polls pass through and train the detector.
        for i in 0..3 {
            assert_eq!(h.on_request(t, fetch("https://a.test/status"), i), vec![Outcome::Proceed(i)]);
            c.advance(Duration::from_secs(5));
        }
        h.set_state(t, TabState::BackgroundWarm, BackgroundMode::DeepSleep);
        // DeepSleep cancels classified polling outright.
        let out = h.on_request(t, fetch("https://a.test/status"), 9);
        assert!(matches!(out[..], [Outcome::Cancel(9, _)]), "{out:?}");
    }

    #[test]
    fn unknown_tab_fails_open_without_losing_the_handle() {
        let (mut h, _c) = hub();
        assert_eq!(h.on_request(TabId(99), fetch("https://a.test/"), 5), vec![Outcome::Proceed(5)]);
    }

    #[test]
    fn removing_a_tab_cancels_its_held_requests() {
        let (mut h, _c) = hub();
        let t = TabId(1);
        h.add_tab(t);
        h.set_state(t, TabState::BackgroundWarm, BackgroundMode::SmartPause);
        h.on_request(t, fetch("https://a.test/x"), 1);
        assert_eq!(h.remove_tab(t).len(), 1);
        assert_eq!(h.held_count(t), 0);
    }
}
