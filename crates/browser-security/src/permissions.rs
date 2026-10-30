use browser_core::fs::write_atomic;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Feature {
    Geolocation,
    Notifications,
    Camera,
    Microphone,
    Midi,
    Clipboard,
    Bluetooth,
    PersistentStorage,
    Other,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Decision {
    Allow,
    Deny,
    /// No stored decision: the UI must prompt. Until it does, access is denied.
    Ask,
}

/// Per-origin permission decisions. Private browsing uses `in_memory()` so
/// nothing is ever written to disk.
#[derive(Default, Serialize, Deserialize)]
pub struct PermissionStore {
    entries: BTreeMap<String, BTreeMap<Feature, Decision>>,
    #[serde(skip)]
    path: Option<PathBuf>,
}

impl PermissionStore {
    pub fn in_memory() -> Self {
        Self::default()
    }

    pub fn open(path: PathBuf) -> Self {
        let mut s: Self = std::fs::read(&path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        s.path = Some(path);
        s
    }

    pub fn get(&self, origin: &str, f: Feature) -> Decision {
        self.entries.get(origin).and_then(|m| m.get(&f)).copied().unwrap_or(Decision::Ask)
    }

    pub fn set(&mut self, origin: &str, f: Feature, d: Decision) -> std::io::Result<()> {
        match d {
            Decision::Ask => {
                if let Some(m) = self.entries.get_mut(origin) {
                    m.remove(&f);
                    if m.is_empty() {
                        self.entries.remove(origin);
                    }
                }
            }
            _ => {
                self.entries.entry(origin.to_string()).or_default().insert(f, d);
            }
        }
        self.persist()
    }

    pub fn clear_origin(&mut self, origin: &str) -> std::io::Result<()> {
        self.entries.remove(origin);
        self.persist()
    }

    pub fn origins(&self) -> impl Iterator<Item = (&String, &BTreeMap<Feature, Decision>)> {
        self.entries.iter()
    }

    fn persist(&self) -> std::io::Result<()> {
        match &self.path {
            Some(p) => write_atomic(p, &serde_json::to_vec_pretty(self).map_err(std::io::Error::other)?),
            None => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_is_ask_and_ask_means_not_granted() {
        let s = PermissionStore::in_memory();
        assert_eq!(s.get("https://a.test", Feature::Camera), Decision::Ask);
    }

    #[test]
    fn decisions_are_per_origin_and_per_feature() {
        let mut s = PermissionStore::in_memory();
        s.set("https://a.test", Feature::Camera, Decision::Allow).unwrap();
        assert_eq!(s.get("https://a.test", Feature::Camera), Decision::Allow);
        assert_eq!(s.get("https://a.test", Feature::Microphone), Decision::Ask);
        assert_eq!(s.get("https://b.test", Feature::Camera), Decision::Ask);
        s.set("https://a.test", Feature::Camera, Decision::Ask).unwrap();
        assert_eq!(s.origins().count(), 0);
    }

    #[test]
    fn persistence_round_trips_but_private_stores_never_touch_disk() {
        let d = std::env::temp_dir().join(format!("browserust-perm-{}", std::process::id()));
        let p = d.join("perms.json");
        let mut s = PermissionStore::open(p.clone());
        s.set("https://a.test", Feature::Geolocation, Decision::Deny).unwrap();
        assert_eq!(PermissionStore::open(p.clone()).get("https://a.test", Feature::Geolocation), Decision::Deny);
        let mut private = PermissionStore::in_memory();
        private.set("https://x.test", Feature::Camera, Decision::Allow).unwrap();
        assert_eq!(PermissionStore::open(p).get("https://x.test", Feature::Camera), Decision::Ask);
        std::fs::remove_dir_all(d).ok();
    }
}
