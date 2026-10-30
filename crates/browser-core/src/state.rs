use serde::{Deserialize, Serialize};

/// The four lifecycle states of a tab.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TabState {
    /// The selected tab. Everything runs normally.
    Foreground,
    /// Default background state. The WebView, DOM and JS heap are all kept; the
    /// engine hides and throttles it and the network policy suppresses
    /// unnecessary requests.
    BackgroundWarm,
    /// Deeper background state after inactivity or memory pressure. Still the
    /// same live WebView (no reload), with caches trimmed and a stricter
    /// network policy.
    BackgroundFrozen,
    /// Renderer resources released; only a session snapshot remains.
    Discarded,
}

impl TabState {
    pub fn is_background(self) -> bool {
        matches!(self, TabState::BackgroundWarm | TabState::BackgroundFrozen)
    }

    pub fn has_webview(self) -> bool {
        !matches!(self, TabState::Discarded)
    }

    pub fn label(self) -> &'static str {
        match self {
            TabState::Foreground => "Active",
            TabState::BackgroundWarm => "Paused",
            TabState::BackgroundFrozen => "Frozen",
            TabState::Discarded => "Discarded",
        }
    }
}
