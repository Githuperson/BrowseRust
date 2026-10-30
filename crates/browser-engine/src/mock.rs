use crate::{Engine, EngineCaps, EngineEvent, TabMemorySample};
use browser_core::{TabId, TabState};
use browser_memory::TrimLevel;
use browser_settings::BackgroundMode;
use url::Url;

#[derive(Clone, Debug, PartialEq)]
pub enum Call {
    Create(TabId, Option<Url>),
    Destroy(TabId),
    Load(TabId, Url),
    Reload(TabId),
    Stop(TabId),
    Back(TabId),
    Forward(TabId),
    Show(TabId),
    Hide(TabId),
    ApplyNetwork(TabId, TabState, BackgroundMode),
    UserNav(TabId),
    ReleaseImages(TabId),
    Trim(TrimLevel),
    RequestPageState(TabId),
    RestoreScroll(TabId, f32, f32),
}

/// Records every call; used by orchestration tests.
#[derive(Default)]
pub struct MockEngine {
    pub calls: Vec<Call>,
    pub events: Vec<EngineEvent>,
    pub memory: std::collections::HashMap<TabId, TabMemorySample>,
    pub favicons: std::collections::HashMap<TabId, crate::FaviconImage>,
}

impl MockEngine {
    pub fn take_calls(&mut self) -> Vec<Call> {
        std::mem::take(&mut self.calls)
    }
    /// True if any call could cause a page (re)load.
    pub fn loaded_anything(&self) -> bool {
        self.calls.iter().any(|c| matches!(c, Call::Load(..) | Call::Reload(_) | Call::Create(..)))
    }
}

impl Engine for MockEngine {
    fn caps(&self) -> EngineCaps {
        EngineCaps { hide_throttles_script: true, request_interception: true, release_tab_images: true, per_tab_memory: true, ..Default::default() }
    }
    fn create_tab(&mut self, tab: TabId, url: Option<&Url>) {
        self.calls.push(Call::Create(tab, url.cloned()));
    }
    fn destroy_tab(&mut self, tab: TabId) {
        self.calls.push(Call::Destroy(tab));
    }
    fn load(&mut self, tab: TabId, url: &Url) {
        self.calls.push(Call::Load(tab, url.clone()));
    }
    fn reload(&mut self, tab: TabId) {
        self.calls.push(Call::Reload(tab));
    }
    fn stop(&mut self, tab: TabId) {
        self.calls.push(Call::Stop(tab));
    }
    fn go_back(&mut self, tab: TabId) {
        self.calls.push(Call::Back(tab));
    }
    fn go_forward(&mut self, tab: TabId) {
        self.calls.push(Call::Forward(tab));
    }
    fn show(&mut self, tab: TabId) {
        self.calls.push(Call::Show(tab));
    }
    fn hide(&mut self, tab: TabId) {
        self.calls.push(Call::Hide(tab));
    }
    fn apply_network(&mut self, tab: TabId, state: TabState, mode: BackgroundMode) {
        self.calls.push(Call::ApplyNetwork(tab, state, mode));
    }
    fn begin_user_navigation(&mut self, tab: TabId) {
        self.calls.push(Call::UserNav(tab));
    }
    fn release_images(&mut self, tab: TabId) -> bool {
        self.calls.push(Call::ReleaseImages(tab));
        true
    }
    fn trim_caches(&mut self, level: TrimLevel) {
        self.calls.push(Call::Trim(level));
    }
    fn tab_memory(&mut self, tab: TabId) -> Option<TabMemorySample> {
        self.memory.get(&tab).copied()
    }
    fn favicon(&mut self, tab: TabId) -> Option<crate::FaviconImage> {
        self.favicons.get(&tab).cloned()
    }
    fn request_page_state(&mut self, tab: TabId) {
        self.calls.push(Call::RequestPageState(tab));
    }
    fn restore_scroll(&mut self, tab: TabId, x: f32, y: f32) {
        self.calls.push(Call::RestoreScroll(tab, x, y));
    }
    fn housekeeping(&mut self) {}
    fn poll_events(&mut self) -> Vec<EngineEvent> {
        std::mem::take(&mut self.events)
    }
}
