use crate::tab::Tab;
use browser_core::{GroupId, TabId, TabState, WorkspaceId};
use browser_session::TabSnapshot;
use std::collections::VecDeque;
use std::time::Instant;
use url::Url;

#[derive(Clone, Debug)]
pub struct Workspace {
    pub id: WorkspaceId,
    pub name: String,
}

#[derive(Clone, Debug)]
pub struct Group {
    pub id: GroupId,
    pub name: String,
    pub color: [u8; 3],
    pub collapsed: bool,
}

const CLOSED_HISTORY: usize = 25;

/// Ordered collection of tabs with workspaces, groups, pinning and an undo-close
/// stack. Invariant: within the vector, pinned tabs always precede unpinned ones.
pub struct TabStrip {
    tabs: Vec<Tab>,
    pub workspaces: Vec<Workspace>,
    pub groups: Vec<Group>,
    pub active_workspace: WorkspaceId,
    pub selected: Option<TabId>,
    closed: VecDeque<TabSnapshot>,
}

impl TabStrip {
    pub fn new() -> Self {
        let ws = Workspace { id: WorkspaceId::next(), name: "Personal".into() };
        Self { tabs: vec![], active_workspace: ws.id, workspaces: vec![ws], groups: vec![], selected: None, closed: VecDeque::new() }
    }

    pub fn tabs(&self) -> &[Tab] {
        &self.tabs
    }
    pub fn tab(&self, id: TabId) -> Option<&Tab> {
        self.tabs.iter().find(|t| t.id == id)
    }
    pub fn tab_mut(&mut self, id: TabId) -> Option<&mut Tab> {
        self.tabs.iter_mut().find(|t| t.id == id)
    }
    pub fn index_of(&self, id: TabId) -> Option<usize> {
        self.tabs.iter().position(|t| t.id == id)
    }
    pub fn len(&self) -> usize {
        self.tabs.len()
    }
    pub fn is_empty(&self) -> bool {
        self.tabs.is_empty()
    }

    /// Tabs of the active workspace, in strip order.
    pub fn visible(&self) -> impl Iterator<Item = &Tab> {
        let ws = self.active_workspace;
        self.tabs.iter().filter(move |t| t.workspace == ws)
    }

    pub(crate) fn insert_tab(&mut self, mut tab: Tab, now: Instant) -> TabId {
        let _ = now;
        tab.workspace = if self.workspaces.iter().any(|w| w.id == tab.workspace) { tab.workspace } else { self.active_workspace };
        let id = tab.id;
        // Unpinned tabs go to the end; pinned tabs go after the last pinned one.
        let at = if tab.pinned { self.tabs.iter().take_while(|t| t.pinned).count() } else { self.tabs.len() };
        self.tabs.insert(at, tab);
        id
    }

    pub fn add_workspace(&mut self, name: &str) -> WorkspaceId {
        let id = WorkspaceId::next();
        self.workspaces.push(Workspace { id, name: name.into() });
        id
    }

    pub fn add_group(&mut self, name: &str, color: [u8; 3]) -> GroupId {
        let id = GroupId::next();
        self.groups.push(Group { id, name: name.into(), color, collapsed: false });
        id
    }

    pub fn set_group(&mut self, tab: TabId, group: Option<GroupId>) {
        if let Some(t) = self.tab_mut(tab) {
            t.group = group;
        }
        // Drop groups that became empty.
        let used: Vec<GroupId> = self.tabs.iter().filter_map(|t| t.group).collect();
        self.groups.retain(|g| used.contains(&g.id));
    }

    pub fn set_pinned(&mut self, id: TabId, pinned: bool) {
        let Some(i) = self.index_of(id) else { return };
        if self.tabs[i].pinned == pinned {
            return;
        }
        let mut t = self.tabs.remove(i);
        t.pinned = pinned;
        // Pinning appends to the pinned region; unpinning inserts right after it.
        let boundary = self.tabs.iter().take_while(|t| t.pinned).count();
        self.tabs.insert(boundary, t);
    }

    /// Drag-and-drop reorder. The destination is clamped so pinned tabs stay in
    /// the pinned region and unpinned tabs stay out of it.
    pub fn move_tab(&mut self, id: TabId, to: usize) {
        let Some(from) = self.index_of(id) else { return };
        let pinned = self.tabs[from].pinned;
        let t = self.tabs.remove(from);
        let pinned_count = self.tabs.iter().take_while(|x| x.pinned).count();
        let (lo, hi) = if pinned { (0, pinned_count) } else { (pinned_count, self.tabs.len()) };
        self.tabs.insert(to.clamp(lo, hi), t);
    }

    pub fn set_muted(&mut self, id: TabId, muted: bool) {
        if let Some(t) = self.tab_mut(id) {
            t.muted = muted;
        }
    }

    /// Remove a tab, remembering its snapshot for "reopen closed tab". Returns
    /// the tab that should be selected next if the closed tab was selected.
    pub fn remove(&mut self, id: TabId) -> Option<TabId> {
        let i = self.index_of(id)?;
        let t = self.tabs.remove(i);
        if t.url.is_some() {
            if self.closed.len() == CLOSED_HISTORY {
                self.closed.pop_back();
            }
            self.closed.push_front(t.snapshot());
        }
        self.set_group(id, None);
        let was_selected = self.selected == Some(id);
        if was_selected {
            self.selected = None;
            // Prefer the neighbour to the right, else left, within the workspace.
            let ws = t.workspace;
            let next = self.tabs[i.min(self.tabs.len())..]
                .iter()
                .chain(self.tabs[..i.min(self.tabs.len())].iter().rev())
                .find(|x| x.workspace == ws)
                .map(|x| x.id);
            return next;
        }
        None
    }

    pub fn pop_closed(&mut self) -> Option<TabSnapshot> {
        self.closed.pop_front()
    }

    pub fn closed_count(&self) -> usize {
        self.closed.len()
    }

    pub fn duplicate_of(&self, id: TabId) -> Option<(Option<Url>, TabSnapshot)> {
        self.tab(id).map(|t| (t.url.clone(), t.snapshot()))
    }

    pub fn count_in_state(&self, s: TabState) -> usize {
        self.tabs.iter().filter(|t| t.state == s).count()
    }
}

impl Default for TabStrip {
    fn default() -> Self {
        Self::new()
    }
}
