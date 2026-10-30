use browser_core::{GroupId, TabId, TabState, WorkspaceId};
use browser_session::TabSnapshot;
use std::sync::Arc;
use std::time::Instant;
use url::Url;

/// Favicon pixels (RGBA8). The buffer is shared between tabs of the same site
/// through the browser's content-addressed blob store.
#[derive(Clone, Debug)]
pub struct TabIcon {
    pub width: u32,
    pub height: u32,
    pub rgba: Arc<[u8]>,
}

#[derive(Clone, Debug)]
pub struct Tab {
    pub id: TabId,
    pub url: Option<Url>,
    pub title: String,
    pub favicon: Option<TabIcon>,
    pub state: TabState,
    pub pinned: bool,
    pub group: Option<GroupId>,
    pub workspace: WorkspaceId,
    pub muted: bool,
    pub audible: bool,
    pub loading: bool,
    /// User-entered form data the browser cannot restore after a discard.
    pub dirty_form: bool,
    pub last_active: Instant,
    pub state_since: Instant,
    pub history: Vec<String>,
    pub history_index: usize,
    pub scroll: (f32, f32),
    pub zoom: f32,
}

impl Tab {
    pub fn new(id: TabId, workspace: WorkspaceId, now: Instant) -> Self {
        Self {
            id,
            url: None,
            title: String::new(),
            favicon: None,
            state: TabState::BackgroundWarm,
            pinned: false,
            group: None,
            workspace,
            muted: false,
            audible: false,
            loading: false,
            dirty_form: false,
            last_active: now,
            state_since: now,
            history: Vec::new(),
            history_index: 0,
            scroll: (0.0, 0.0),
            zoom: 1.0,
        }
    }

    pub fn host(&self) -> Option<&str> {
        self.url.as_ref().and_then(Url::host_str)
    }

    pub fn display_title(&self) -> &str {
        if !self.title.is_empty() {
            &self.title
        } else {
            self.host().unwrap_or("New Tab")
        }
    }

    pub fn snapshot(&self) -> TabSnapshot {
        TabSnapshot {
            url: self.url.as_ref().map(Url::to_string),
            title: self.title.clone(),
            history: self.history.clone(),
            history_index: self.history_index,
            scroll_x: self.scroll.0,
            scroll_y: self.scroll.1,
            zoom: self.zoom,
            pinned: self.pinned,
            muted: self.muted,
            group: self.group.map(|g| g.0),
            workspace: self.workspace.0,
        }
    }
}
