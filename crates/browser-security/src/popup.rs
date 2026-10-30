use browser_core::{Clock, TabId};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Blocks popups that were not caused by a recent real user gesture.
///
/// Servo does not report user activation with `window.open`, so the browser
/// tracks gestures itself: the shell calls [`PopupBlocker::note_user_input`]
/// for every click/key it forwards to a tab's content area. A popup request is
/// honoured only if it arrives shortly after such input — which scripts cannot
/// forge, since they have no way to inject events into the shell.
pub struct PopupBlocker {
    clock: Arc<dyn Clock>,
    window: Duration,
    last_input: HashMap<TabId, Instant>,
    /// Popups we refused, for the "popup blocked" indicator.
    blocked: HashMap<TabId, u32>,
}

impl PopupBlocker {
    pub fn new(clock: Arc<dyn Clock>) -> Self {
        Self { clock, window: Duration::from_millis(1500), last_input: HashMap::new(), blocked: HashMap::new() }
    }

    pub fn note_user_input(&mut self, tab: TabId) {
        self.last_input.insert(tab, self.clock.now());
    }

    /// `true` = open it.
    pub fn allow_popup(&mut self, tab: TabId) -> bool {
        let ok = self.last_input.get(&tab).is_some_and(|t| self.clock.now().saturating_duration_since(*t) <= self.window);
        if !ok {
            *self.blocked.entry(tab).or_default() += 1;
        }
        ok
    }

    pub fn blocked_count(&self, tab: TabId) -> u32 {
        self.blocked.get(&tab).copied().unwrap_or(0)
    }

    pub fn forget(&mut self, tab: TabId) {
        self.last_input.remove(&tab);
        self.blocked.remove(&tab);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use browser_core::ManualClock;

    #[test]
    fn popup_requires_recent_input() {
        let c = ManualClock::default();
        let mut b = PopupBlocker::new(Arc::new(c.clone()));
        let t = TabId(1);
        assert!(!b.allow_popup(t), "no input ever");
        b.note_user_input(t);
        assert!(b.allow_popup(t));
        c.advance(Duration::from_secs(5));
        assert!(!b.allow_popup(t), "gesture expired");
        assert_eq!(b.blocked_count(t), 2);
        assert!(!b.allow_popup(TabId(2)), "other tabs are independent");
    }
}
