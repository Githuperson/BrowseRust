use crate::classify::{AnalyticsMatcher, PollingDetector, RequestClass, classify};
use crate::monitor::{Disposition, NetworkMonitor};
use crate::policy::{Action, NetworkPolicy, effective_mode};
use crate::request::RequestInfo;
use browser_core::{Clock, RequestId, TabId, TabState};
use browser_settings::BackgroundMode;
use std::collections::VecDeque;
use std::sync::Arc;
use std::time::{Duration, Instant};

#[derive(Clone, Debug)]
pub struct ControllerConfig {
    /// Hard cap on simultaneously held requests per tab. Overflow evicts (cancels)
    /// the oldest *idempotent* request, or rejects the new one — memory stays bounded.
    pub max_held: usize,
    /// A held request older than this is cancelled by [`NetworkActivityController::tick`],
    /// so the page's promises settle instead of hanging forever.
    pub hold_ttl: Duration,
    /// A newly navigated tab may finish loading even if it is backgrounded
    /// (e.g. "open in background tab"). Capped so a page cannot use it forever.
    pub initial_load_grace: Duration,
}

impl Default for ControllerConfig {
    fn default() -> Self {
        Self { max_held: 64, hold_ttl: Duration::from_secs(10 * 60), initial_load_grace: Duration::from_secs(30) }
    }
}

/// What the engine adapter must do with a handle the controller hands back.
#[derive(Debug, PartialEq, Eq)]
pub enum Outcome<H> {
    /// Let this request proceed now.
    Proceed(H),
    /// Fail this request (the adapter turns this into a network error).
    Cancel(H, CancelReason),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CancelReason {
    Policy,
    QueueOverflow,
    HoldExpired,
    SupersededPoll,
    TabClosed,
}

/// Result of resuming a tab.
#[derive(Debug)]
pub struct Resumed<H> {
    pub release: Vec<H>,
    pub cancel: Vec<(H, CancelReason)>,
}

struct Held<H> {
    id: RequestId,
    class: RequestClass,
    poll_key: String,
    idempotent: bool,
    since: Instant,
    handle: H,
}

/// Applies the network policy to every request of **one** tab.
///
/// Scoping is per controller instance (= per tab / browsing context), never per
/// origin: a background tab holding `mail.example.com` requests has no effect on
/// another tab talking to the same origin.
pub struct NetworkActivityController<H> {
    tab: TabId,
    clock: Arc<dyn Clock>,
    cfg: ControllerConfig,
    policy: NetworkPolicy,
    analytics: AnalyticsMatcher,
    state: TabState,
    mode: BackgroundMode,
    grace_until: Option<Instant>,
    held: VecDeque<Held<H>>,
    monitor: NetworkMonitor,
}

impl<H> NetworkActivityController<H> {
    pub fn new(tab: TabId, clock: Arc<dyn Clock>, cfg: ControllerConfig, policy: NetworkPolicy) -> Self {
        Self {
            tab,
            monitor: NetworkMonitor::new(clock.clone()),
            clock,
            cfg,
            policy,
            analytics: AnalyticsMatcher::default(),
            state: TabState::Foreground,
            mode: BackgroundMode::default(),
            grace_until: None,
            held: VecDeque::new(),
        }
    }

    pub fn tab(&self) -> TabId {
        self.tab
    }
    pub fn state(&self) -> TabState {
        self.state
    }
    pub fn held_count(&self) -> usize {
        self.held.len()
    }
    pub fn monitor(&self) -> &NetworkMonitor {
        &self.monitor
    }
    pub fn policy_mut(&mut self) -> &mut NetworkPolicy {
        &mut self.policy
    }

    /// Called when the user starts a navigation in this tab; opens the initial-load
    /// grace window that lets a freshly opened background tab finish loading.
    pub fn begin_user_navigation(&mut self) {
        self.grace_until = Some(self.clock.now() + self.cfg.initial_load_grace);
    }

    /// Called when the engine reports the page finished loading.
    pub fn load_complete(&mut self) {
        self.grace_until = None;
    }

    fn in_grace(&self) -> bool {
        self.grace_until.is_some_and(|t| self.clock.now() < t)
    }

    /// Change lifecycle state/mode. Returns handles to release when the change
    /// makes held requests eligible (e.g. mode relaxed, or tab foregrounded).
    pub fn set_state(&mut self, state: TabState, mode: BackgroundMode) -> Resumed<H> {
        self.state = state;
        self.mode = mode;
        if effective_mode(state, mode) == BackgroundMode::Normal { self.drain() } else { Resumed { release: vec![], cancel: vec![] } }
    }

    /// Decide what to do with one request. `polling` comes from the shared
    /// [`PollingDetector`] (it is shared across tabs, keyed by tab).
    pub fn on_request(&mut self, info: &RequestInfo, polling: bool, handle: H) -> Vec<Outcome<H>> {
        let class = classify(info, &self.analytics, polling);
        let id = RequestId::next();
        let eff = effective_mode(self.state, self.mode);
        let mut action = self.policy.action(eff, class);

        // A tab that is still doing its initial load may complete it.
        if action == Action::Hold
            && self.in_grace()
            && matches!(class, RequestClass::Document | RequestClass::Subresource | RequestClass::Api | RequestClass::Media)
        {
            action = Action::Allow;
        }

        match action {
            Action::Allow => {
                self.monitor.record(id, self.tab, &info.url, class, Disposition::Allowed);
                vec![Outcome::Proceed(handle)]
            }
            Action::Cancel => {
                self.monitor.record(id, self.tab, &info.url, class, Disposition::Cancelled);
                vec![Outcome::Cancel(handle, CancelReason::Policy)]
            }
            Action::Hold => self.hold(id, class, info, handle),
        }
    }

    fn hold(&mut self, id: RequestId, class: RequestClass, info: &RequestInfo, handle: H) -> Vec<Outcome<H>> {
        let mut out = Vec::new();
        let poll_key = PollingDetector::key(info);

        // A newer poll supersedes an older held poll for the same endpoint:
        // keeping both would just replay a stale request later.
        if class == RequestClass::BackgroundPolling {
            let mut i = 0;
            while i < self.held.len() {
                if self.held[i].class == RequestClass::BackgroundPolling && self.held[i].poll_key == poll_key && self.held[i].idempotent {
                    let old = self.held.remove(i).expect("index in range");
                    self.monitor.set_disposition(old.id, Disposition::Cancelled);
                    out.push(Outcome::Cancel(old.handle, CancelReason::SupersededPoll));
                } else {
                    i += 1;
                }
            }
        }

        if self.held.len() >= self.cfg.max_held {
            // Evict the oldest idempotent request; if none, reject the newcomer.
            if let Some(pos) = self.held.iter().position(|h| h.idempotent) {
                let old = self.held.remove(pos).expect("index in range");
                self.monitor.set_disposition(old.id, Disposition::Cancelled);
                out.push(Outcome::Cancel(old.handle, CancelReason::QueueOverflow));
            } else {
                self.monitor.record(id, self.tab, &info.url, class, Disposition::Cancelled);
                out.push(Outcome::Cancel(handle, CancelReason::QueueOverflow));
                return out;
            }
        }

        self.monitor.record(id, self.tab, &info.url, class, Disposition::Held);
        self.held.push_back(Held { id, class, poll_key, idempotent: info.is_idempotent(), since: self.clock.now(), handle });
        out
    }

    /// Expire held requests older than the TTL.
    pub fn tick(&mut self) -> Vec<Outcome<H>> {
        let now = self.clock.now();
        let mut out = Vec::new();
        let mut i = 0;
        while i < self.held.len() {
            if now.duration_since(self.held[i].since) >= self.cfg.hold_ttl {
                let h = self.held.remove(i).expect("index in range");
                self.monitor.set_disposition(h.id, Disposition::Cancelled);
                out.push(Outcome::Cancel(h.handle, CancelReason::HoldExpired));
            } else {
                i += 1;
            }
        }
        out
    }

    /// Release everything held. Stale duplicate polls are dropped (only the newest
    /// per endpoint is replayed) and everything else is replayed in arrival order.
    fn drain(&mut self) -> Resumed<H> {
        let mut release = Vec::new();
        let mut cancel = Vec::new();
        let all: Vec<Held<H>> = self.held.drain(..).collect();
        let mut newest_poll: Vec<(String, usize)> = Vec::new();
        for (i, h) in all.iter().enumerate() {
            if h.class == RequestClass::BackgroundPolling {
                match newest_poll.iter_mut().find(|(k, _)| *k == h.poll_key) {
                    Some(e) => e.1 = i,
                    None => newest_poll.push((h.poll_key.clone(), i)),
                }
            }
        }
        for (i, h) in all.into_iter().enumerate() {
            let stale = h.class == RequestClass::BackgroundPolling
                && h.idempotent
                && newest_poll.iter().any(|(k, n)| *k == h.poll_key && *n != i);
            if stale {
                self.monitor.set_disposition(h.id, Disposition::Cancelled);
                cancel.push((h.handle, CancelReason::SupersededPoll));
            } else {
                self.monitor.set_disposition(h.id, Disposition::Released);
                release.push(h.handle);
            }
        }
        Resumed { release, cancel }
    }

    /// Tab is closing: cancel everything still held.
    pub fn close(&mut self) -> Vec<Outcome<H>> {
        self.held
            .drain(..)
            .map(|h| {
                self.monitor.set_disposition(h.id, Disposition::Cancelled);
                Outcome::Cancel(h.handle, CancelReason::TabClosed)
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::request::Destination;
    use browser_core::ManualClock;
    use url::Url;

    type H = u32;

    fn ctl(clock: &ManualClock, tab: u64) -> NetworkActivityController<H> {
        NetworkActivityController::new(TabId(tab), Arc::new(clock.clone()), ControllerConfig::default(), NetworkPolicy::default())
    }
    fn fetch(u: &str) -> RequestInfo {
        RequestInfo::new("GET", Url::parse(u).unwrap(), Destination::Empty)
    }
    fn only(mut v: Vec<Outcome<H>>) -> Outcome<H> {
        assert_eq!(v.len(), 1);
        v.remove(0)
    }

    #[test]
    fn foreground_passes_everything() {
        let clock = ManualClock::default();
        let mut c = ctl(&clock, 1);
        assert_eq!(only(c.on_request(&fetch("https://a.test/api"), false, 1)), Outcome::Proceed(1));
        assert_eq!(c.held_count(), 0);
    }

    #[test]
    fn smart_pause_holds_api_cancels_analytics_then_resume_releases_in_order() {
        let clock = ManualClock::default();
        let mut c = ctl(&clock, 1);
        c.set_state(TabState::BackgroundWarm, BackgroundMode::SmartPause);
        assert!(c.on_request(&fetch("https://a.test/api/1"), false, 1).is_empty());
        assert!(c.on_request(&fetch("https://a.test/api/2"), false, 2).is_empty());
        let an = fetch("https://www.google-analytics.com/collect");
        assert_eq!(only(c.on_request(&an, false, 3)), Outcome::Cancel(3, CancelReason::Policy));
        assert_eq!(c.held_count(), 2);

        let r = c.set_state(TabState::Foreground, BackgroundMode::SmartPause);
        assert_eq!(r.release, vec![1, 2], "held requests are replayed in arrival order");
        assert!(r.cancel.is_empty());
        assert_eq!(c.held_count(), 0);
    }

    #[test]
    fn user_navigation_is_never_held() {
        let clock = ManualClock::default();
        let mut c = ctl(&clock, 1);
        c.set_state(TabState::BackgroundFrozen, BackgroundMode::StrictOffline);
        let mut nav = RequestInfo::new("GET", Url::parse("https://a.test/").unwrap(), Destination::Document);
        nav.is_main_frame = true;
        nav.user_initiated = true;
        assert_eq!(only(c.on_request(&nav, false, 9)), Outcome::Proceed(9));
        // …but an automatic main-frame load (meta refresh / location=) is held, not fetched.
        nav.user_initiated = false;
        assert!(c.on_request(&nav, false, 10).is_empty());
    }

    #[test]
    fn user_download_proceeds_in_background() {
        let clock = ManualClock::default();
        let mut c = ctl(&clock, 1);
        c.set_state(TabState::BackgroundWarm, BackgroundMode::StrictOffline);
        let mut dl = fetch("https://a.test/big.zip");
        dl.is_download = true;
        assert_eq!(only(c.on_request(&dl, false, 4)), Outcome::Proceed(4));
    }

    #[test]
    fn controllers_are_isolated_per_tab_even_for_the_same_origin() {
        let clock = ManualClock::default();
        let mut bg = ctl(&clock, 1);
        let mut fg = ctl(&clock, 2);
        bg.set_state(TabState::BackgroundWarm, BackgroundMode::SmartPause);
        let req = fetch("https://mail.example.com/api/inbox");
        assert!(bg.on_request(&req, false, 1).is_empty(), "background tab held");
        assert_eq!(only(fg.on_request(&req, false, 2)), Outcome::Proceed(2), "active tab on same origin unaffected");
    }

    #[test]
    fn hold_queue_is_bounded_and_evicts_oldest_idempotent() {
        let clock = ManualClock::default();
        let cfg = ControllerConfig { max_held: 3, ..Default::default() };
        let mut c = NetworkActivityController::<H>::new(TabId(1), Arc::new(clock.clone()), cfg, NetworkPolicy::default());
        c.set_state(TabState::BackgroundWarm, BackgroundMode::SmartPause);
        for i in 0..3 {
            assert!(c.on_request(&fetch(&format!("https://a.test/api/{i}")), false, i).is_empty());
        }
        let out = c.on_request(&fetch("https://a.test/api/new"), false, 99);
        assert_eq!(out, vec![Outcome::Cancel(0, CancelReason::QueueOverflow)]);
        assert_eq!(c.held_count(), 3, "never exceeds max_held");
    }

    #[test]
    fn non_idempotent_requests_are_never_evicted_silently() {
        let clock = ManualClock::default();
        let cfg = ControllerConfig { max_held: 1, ..Default::default() };
        let mut c = NetworkActivityController::<H>::new(TabId(1), Arc::new(clock.clone()), cfg, NetworkPolicy::default());
        c.set_state(TabState::BackgroundWarm, BackgroundMode::SmartPause);
        let post = RequestInfo::new("POST", Url::parse("https://a.test/save").unwrap(), Destination::Empty);
        assert!(c.on_request(&post, false, 1).is_empty());
        // Queue full of a POST: the newcomer is rejected rather than the POST dropped.
        assert_eq!(only(c.on_request(&fetch("https://a.test/x"), false, 2)), Outcome::Cancel(2, CancelReason::QueueOverflow));
        assert_eq!(c.held_count(), 1);
    }

    #[test]
    fn held_requests_expire_so_promises_settle() {
        let clock = ManualClock::default();
        let mut c = ctl(&clock, 1);
        c.set_state(TabState::BackgroundFrozen, BackgroundMode::SmartPause);
        c.on_request(&fetch("https://a.test/api"), false, 1);
        clock.advance(Duration::from_secs(9 * 60));
        assert!(c.tick().is_empty());
        clock.advance(Duration::from_secs(2 * 60));
        assert_eq!(c.tick(), vec![Outcome::Cancel(1, CancelReason::HoldExpired)]);
        assert_eq!(c.held_count(), 0);
    }

    #[test]
    fn stale_polls_are_superseded_not_replayed() {
        let clock = ManualClock::default();
        let mut c = ctl(&clock, 1);
        c.set_state(TabState::BackgroundWarm, BackgroundMode::SmartPause);
        assert!(c.on_request(&fetch("https://a.test/poll"), true, 1).is_empty());
        let out = c.on_request(&fetch("https://a.test/poll"), true, 2);
        assert_eq!(out, vec![Outcome::Cancel(1, CancelReason::SupersededPoll)]);
        assert_eq!(c.held_count(), 1);
        let r = c.set_state(TabState::Foreground, BackgroundMode::SmartPause);
        assert_eq!(r.release, vec![2]);
    }

    #[test]
    fn deep_sleep_drops_polling_instead_of_queueing() {
        let clock = ManualClock::default();
        let mut c = ctl(&clock, 1);
        c.set_state(TabState::BackgroundWarm, BackgroundMode::DeepSleep);
        assert_eq!(only(c.on_request(&fetch("https://a.test/poll"), true, 1)), Outcome::Cancel(1, CancelReason::Policy));
        assert_eq!(c.held_count(), 0);
    }

    #[test]
    fn initial_load_grace_lets_a_background_tab_finish_loading_then_expires() {
        let clock = ManualClock::default();
        let mut c = ctl(&clock, 1);
        c.begin_user_navigation();
        c.set_state(TabState::BackgroundWarm, BackgroundMode::SmartPause);
        let script = RequestInfo::new("GET", Url::parse("https://a.test/app.js").unwrap(), Destination::Script);
        assert_eq!(only(c.on_request(&script, false, 1)), Outcome::Proceed(1));
        c.load_complete();
        assert!(c.on_request(&script, false, 2).is_empty(), "after load, sub-resources are held");

        let mut c = ctl(&clock, 2);
        c.begin_user_navigation();
        c.set_state(TabState::BackgroundWarm, BackgroundMode::SmartPause);
        clock.advance(Duration::from_secs(31));
        assert!(c.on_request(&script, false, 3).is_empty(), "grace is capped; a page cannot stay 'loading' forever");
    }

    #[test]
    fn relaxing_mode_to_normal_releases_held_requests() {
        let clock = ManualClock::default();
        let mut c = ctl(&clock, 1);
        c.set_state(TabState::BackgroundWarm, BackgroundMode::SmartPause);
        c.on_request(&fetch("https://a.test/api"), false, 1);
        let r = c.set_state(TabState::BackgroundWarm, BackgroundMode::Normal);
        assert_eq!(r.release, vec![1]);
    }

    #[test]
    fn closing_a_tab_cancels_everything_held() {
        let clock = ManualClock::default();
        let mut c = ctl(&clock, 1);
        c.set_state(TabState::BackgroundWarm, BackgroundMode::SmartPause);
        c.on_request(&fetch("https://a.test/a"), false, 1);
        c.on_request(&fetch("https://a.test/b"), false, 2);
        let out = c.close();
        assert_eq!(out.len(), 2);
        assert!(out.iter().all(|o| matches!(o, Outcome::Cancel(_, CancelReason::TabClosed))));
    }

    #[test]
    fn monitor_counts_what_actually_left_the_machine() {
        let clock = ManualClock::default();
        let mut c = ctl(&clock, 1);
        c.on_request(&fetch("https://a.test/fg"), false, 1);
        c.set_state(TabState::BackgroundWarm, BackgroundMode::SmartPause);
        for i in 0..10 {
            c.on_request(&fetch(&format!("https://a.test/bg{i}?token=secret")), false, i + 10);
        }
        assert_eq!(c.monitor().sent_within(Duration::from_secs(60)), 1, "only the foreground request was sent");
        assert!(c.monitor().recent().all(|r| !r.url.contains("secret")), "query strings are not logged");
        assert_eq!(c.monitor().count(RequestClass::Api, Disposition::Held), 10);
    }
}
