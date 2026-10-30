//! Session snapshots: what is saved when a tab is DISCARDED, what is written to
//! disk for session restoration, and the crash-recovery marker.
//!
//! A snapshot is deliberately small (URLs, titles, scroll, zoom): it is what makes
//! a discarded tab cost kilobytes instead of tens of megabytes. Restoring one
//! *does* load the page again — that is the defining difference between DISCARDED
//! and BACKGROUND_FROZEN/WARM, which never reload.

use browser_core::{TabId, fs::write_atomic};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct TabSnapshot {
    pub url: Option<String>,
    pub title: String,
    /// Back/forward entries (oldest first) and the index of the current one.
    pub history: Vec<String>,
    pub history_index: usize,
    pub scroll_x: f32,
    pub scroll_y: f32,
    pub zoom: f32,
    pub pinned: bool,
    pub muted: bool,
    /// Group and workspace are stored as raw ids and re-mapped on restore.
    pub group: Option<u64>,
    pub workspace: u64,
}

impl TabSnapshot {
    /// URL to load on restore: the current history entry when available.
    pub fn restore_url(&self) -> Option<&str> {
        self.history.get(self.history_index).map(String::as_str).or(self.url.as_deref())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SavedTab {
    pub id: TabId,
    pub snapshot: TabSnapshot,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SessionFile {
    pub version: u32,
    pub tabs: Vec<SavedTab>,
    pub selected: Option<TabId>,
}

#[derive(Debug, thiserror::Error)]
pub enum SessionError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("parse: {0}")]
    Parse(#[from] serde_json::Error),
}

const VERSION: u32 = 1;

/// Owns the on-disk session plus a "clean shutdown" marker for crash recovery.
pub struct SessionStore {
    session: PathBuf,
    running_marker: PathBuf,
}

impl SessionStore {
    pub fn new(dir: &Path) -> Self {
        Self { session: dir.join("session.json"), running_marker: dir.join("running.lock") }
    }

    /// Call at startup. Returns `true` if the previous run did not exit cleanly.
    pub fn begin_run(&self) -> std::io::Result<bool> {
        let crashed = self.running_marker.exists();
        write_atomic(&self.running_marker, std::process::id().to_string().as_bytes())?;
        Ok(crashed)
    }

    /// Call on orderly shutdown.
    pub fn end_run(&self) -> std::io::Result<()> {
        match std::fs::remove_file(&self.running_marker) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        }
    }

    pub fn save(&self, tabs: Vec<SavedTab>, selected: Option<TabId>) -> Result<(), SessionError> {
        let f = SessionFile { version: VERSION, tabs, selected };
        write_atomic(&self.session, &serde_json::to_vec(&f)?)?;
        Ok(())
    }

    /// A missing or corrupt file yields an empty session rather than an error:
    /// failing to restore must never prevent the browser from starting.
    pub fn load(&self) -> SessionFile {
        std::fs::read(&self.session)
            .ok()
            .and_then(|b| serde_json::from_slice::<SessionFile>(&b).ok())
            .filter(|f| f.version == VERSION)
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("browserust-session-{name}-{}", std::process::id()));
        std::fs::remove_dir_all(&d).ok();
        d
    }

    #[test]
    fn round_trip_preserves_tabs_and_selection() {
        let d = tmp("rt");
        let s = SessionStore::new(&d);
        let snap = TabSnapshot { url: Some("https://a.test/".into()), title: "A".into(), history: vec!["https://a.test/0".into(), "https://a.test/".into()], history_index: 1, scroll_y: 420.0, zoom: 1.25, ..Default::default() };
        s.save(vec![SavedTab { id: TabId(7), snapshot: snap.clone() }], Some(TabId(7))).unwrap();
        let f = s.load();
        assert_eq!(f.tabs[0].snapshot, snap);
        assert_eq!(f.selected, Some(TabId(7)));
        assert_eq!(f.tabs[0].snapshot.restore_url(), Some("https://a.test/"));
        std::fs::remove_dir_all(d).ok();
    }

    #[test]
    fn corrupt_or_missing_session_is_an_empty_session() {
        let d = tmp("bad");
        let s = SessionStore::new(&d);
        assert!(s.load().tabs.is_empty());
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("session.json"), b"{not json").unwrap();
        assert!(s.load().tabs.is_empty());
        std::fs::remove_dir_all(d).ok();
    }

    #[test]
    fn crash_detection_uses_the_running_marker() {
        let d = tmp("crash");
        let s = SessionStore::new(&d);
        assert!(!s.begin_run().unwrap(), "first run is clean");
        // Simulate a crash: no end_run().
        assert!(s.begin_run().unwrap(), "marker left behind means crash");
        s.end_run().unwrap();
        assert!(!s.begin_run().unwrap(), "orderly exit clears the marker");
        s.end_run().unwrap();
        std::fs::remove_dir_all(d).ok();
    }
}
