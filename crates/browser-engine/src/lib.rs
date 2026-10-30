//! The browser-engine abstraction.
//!
//! Everything above this crate (tabs, memory, network policy, UI) is written
//! against [`Engine`]; only `servo-integration` knows about Servo. Swapping or
//! upgrading the engine therefore touches one crate.

mod mock;

pub use mock::{Call, MockEngine};

use browser_core::{ByteSize, TabId, TabState};
use browser_memory::TrimLevel;
use browser_settings::BackgroundMode;
use url::Url;

/// What the engine can *actually* do. The UI and docs surface these honestly:
/// a capability that is `false` is a documented limitation, not a hidden stub.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EngineCaps {
    /// Hiding a tab stops painting and throttles its timers / rAF.
    pub hide_throttles_script: bool,
    /// Requests can be held/cancelled per tab before they reach the network.
    pub request_interception: bool,
    /// Decoded image memory of a single tab can be released on demand.
    pub release_tab_images: bool,
    /// Memory is attributable to individual tabs.
    pub per_tab_memory: bool,
    /// Incoming WebSocket/SSE messages can be paused without closing the socket.
    pub pause_stream_delivery: bool,
    /// Script execution of a hidden tab can be fully suspended (not just throttled).
    pub suspend_script: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoadState {
    Started,
    HeadParsed,
    Complete,
}

/// Events flowing engine → browser.
#[derive(Clone, Debug, PartialEq)]
pub enum EngineEvent {
    UrlChanged(TabId, Url),
    TitleChanged(TabId, Option<String>),
    Load(TabId, LoadState),
    FaviconChanged(TabId),
    HistoryChanged { tab: TabId, entries: Vec<Url>, current: usize },
    FrameReady(TabId),
    Crashed(TabId, String),
    /// A page asked for a new tab/window (`window.open`, `target=_blank`).
    NewTabRequested { from: TabId, url: Option<Url> },
    /// The engine already created a tab for a gesture-approved `window.open`.
    TabOpenedByPage { tab: TabId, from: TabId },
    FullscreenChanged(TabId, bool),
    /// Result of `request_page_state`: scroll offset and whether the page holds
    /// user-entered form data that would be lost by discarding.
    PageState { tab: TabId, scroll: (f32, f32), dirty_form: bool },
    /// Audible state of the page changed (for the speaker indicator & freeze guard).
    AudibleChanged(TabId, bool),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TabMemorySample {
    pub private: ByteSize,
    pub shared: ByteSize,
}

pub trait Engine {
    fn caps(&self) -> EngineCaps;

    fn create_tab(&mut self, tab: TabId, url: Option<&Url>);
    fn destroy_tab(&mut self, tab: TabId);

    fn load(&mut self, tab: TabId, url: &Url);
    fn reload(&mut self, tab: TabId);
    fn stop(&mut self, tab: TabId);
    fn go_back(&mut self, tab: TabId);
    fn go_forward(&mut self, tab: TabId);

    /// Make the tab visible. Must not navigate.
    fn show(&mut self, tab: TabId);
    /// Hide the tab (stop painting, allow throttling). Must not navigate.
    fn hide(&mut self, tab: TabId);

    /// Install the network policy for a tab's state/mode. Releases or cancels any
    /// requests that were being held, as appropriate.
    fn apply_network(&mut self, tab: TabId, state: TabState, mode: BackgroundMode);
    /// Mark the next main-frame load of `tab` as started by the user.
    fn begin_user_navigation(&mut self, tab: TabId);

    /// Returns `true` if the engine could act on the request.
    fn release_images(&mut self, tab: TabId) -> bool;
    fn trim_caches(&mut self, level: TrimLevel);

    fn tab_memory(&mut self, tab: TabId) -> Option<TabMemorySample>;

    /// Ask the page for its scroll position and form-dirty flag; the answer
    /// arrives later as [`EngineEvent::PageState`].
    fn request_page_state(&mut self, tab: TabId);
    /// Scroll a freshly restored (previously discarded) tab back to where it was.
    fn restore_scroll(&mut self, tab: TabId, x: f32, y: f32);
    /// Periodic maintenance: expire stale held requests, etc.
    fn housekeeping(&mut self);

    /// Drain pending events (call after pumping the engine).
    fn poll_events(&mut self) -> Vec<EngineEvent>;
}
