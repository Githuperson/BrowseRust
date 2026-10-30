use crate::classify::RequestClass;
use browser_core::TabState;
use browser_settings::BackgroundMode;

/// What to do with a request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    /// Let it through now.
    Allow,
    /// Keep it pending (bounded) and release it when the tab is foregrounded.
    Hold,
    /// Fail it immediately. Reserved for traffic the page cannot be harmed by
    /// losing: telemetry and speculative prefetch.
    Cancel,
}

/// The mode that actually governs a tab given its lifecycle state.
///
/// * Foreground tabs are never restricted.
/// * A frozen tab is never *less* strict than Smart Pause, even if its site is
///   configured "Always active"/Normal (it was frozen by memory pressure).
/// * A discarded tab has no WebView, so nothing can be requested.
pub fn effective_mode(state: TabState, mode: BackgroundMode) -> BackgroundMode {
    match (state, mode) {
        (TabState::Foreground, _) => BackgroundMode::Normal,
        (TabState::BackgroundFrozen, BackgroundMode::Normal) => BackgroundMode::SmartPause,
        (TabState::Discarded, _) => BackgroundMode::StrictOffline,
        (_, m) => m,
    }
}

/// Policy table. Configurable per category through [`NetworkPolicy::set`].
#[derive(Clone, Debug)]
pub struct NetworkPolicy {
    smart: [Action; 10],
    deep: [Action; 10],
    strict: [Action; 10],
    /// Suppress user-started downloads in the background too (default false).
    pub suppress_downloads: bool,
}

fn idx(c: RequestClass) -> usize {
    RequestClass::ALL.iter().position(|x| *x == c).expect("class in ALL")
}

impl Default for NetworkPolicy {
    fn default() -> Self {
        use Action::*;
        use RequestClass as C;
        let build = |f: fn(C) -> Action| {
            let mut t = [Allow; 10];
            for c in C::ALL {
                t[idx(c)] = f(c);
            }
            t
        };
        Self {
            // Smart Pause: stop what is unnecessary, keep the page loadable.
            smart: build(|c| match c {
                C::EssentialNavigation | C::Download => Allow,
                C::Document | C::Subresource | C::Api | C::BackgroundPolling | C::Streaming | C::Media => Hold,
                C::Analytics | C::Prefetch => Cancel,
            }),
            // Deep Sleep: same classes, but polling is dropped rather than queued.
            deep: build(|c| match c {
                C::EssentialNavigation | C::Download => Allow,
                C::Document | C::Subresource | C::Api | C::Streaming | C::Media => Hold,
                C::BackgroundPolling | C::Analytics | C::Prefetch => Cancel,
            }),
            // Strict Offline: nothing the page initiates leaves the machine.
            strict: build(|c| match c {
                C::EssentialNavigation | C::Download => Allow,
                C::Document | C::Subresource | C::Api | C::Streaming | C::Media => Hold,
                C::BackgroundPolling | C::Analytics | C::Prefetch => Cancel,
            }),
            suppress_downloads: false,
        }
    }
}

impl NetworkPolicy {
    pub fn action(&self, mode: BackgroundMode, class: RequestClass) -> Action {
        if class == RequestClass::Download {
            // User-started downloads survive every mode unless explicitly opted out.
            return if self.suppress_downloads && mode != BackgroundMode::Normal { Action::Hold } else { Action::Allow };
        }
        match mode {
            BackgroundMode::Normal => Action::Allow,
            BackgroundMode::SmartPause => self.smart[idx(class)],
            BackgroundMode::DeepSleep => self.deep[idx(class)],
            BackgroundMode::StrictOffline => self.strict[idx(class)],
        }
    }

    /// Override one cell. `Normal` has no table (it always allows).
    pub fn set(&mut self, mode: BackgroundMode, class: RequestClass, action: Action) {
        match mode {
            BackgroundMode::Normal => {}
            BackgroundMode::SmartPause => self.smart[idx(class)] = action,
            BackgroundMode::DeepSleep => self.deep[idx(class)] = action,
            BackgroundMode::StrictOffline => self.strict[idx(class)] = action,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn foreground_and_normal_never_restricted() {
        let p = NetworkPolicy::default();
        for c in RequestClass::ALL {
            assert_eq!(p.action(effective_mode(TabState::Foreground, BackgroundMode::StrictOffline), c), Action::Allow);
            assert_eq!(p.action(effective_mode(TabState::BackgroundWarm, BackgroundMode::Normal), c), Action::Allow);
        }
    }

    #[test]
    fn frozen_is_never_looser_than_smart_pause() {
        assert_eq!(effective_mode(TabState::BackgroundFrozen, BackgroundMode::Normal), BackgroundMode::SmartPause);
        assert_eq!(effective_mode(TabState::BackgroundFrozen, BackgroundMode::DeepSleep), BackgroundMode::DeepSleep);
    }

    #[test]
    fn user_downloads_survive_every_mode_unless_opted_out() {
        let mut p = NetworkPolicy::default();
        for m in [BackgroundMode::SmartPause, BackgroundMode::DeepSleep, BackgroundMode::StrictOffline] {
            assert_eq!(p.action(m, RequestClass::Download), Action::Allow);
        }
        p.suppress_downloads = true;
        assert_eq!(p.action(BackgroundMode::SmartPause, RequestClass::Download), Action::Hold);
        assert_eq!(p.action(BackgroundMode::Normal, RequestClass::Download), Action::Allow);
    }

    #[test]
    fn smart_pause_defaults() {
        let p = NetworkPolicy::default();
        let m = BackgroundMode::SmartPause;
        assert_eq!(p.action(m, RequestClass::Api), Action::Hold);
        assert_eq!(p.action(m, RequestClass::BackgroundPolling), Action::Hold);
        assert_eq!(p.action(m, RequestClass::Analytics), Action::Cancel);
        assert_eq!(p.action(m, RequestClass::Prefetch), Action::Cancel);
        assert_eq!(p.action(BackgroundMode::DeepSleep, RequestClass::BackgroundPolling), Action::Cancel);
    }
}
