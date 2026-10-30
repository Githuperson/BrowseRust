//! User-facing configuration: background-tab modes, per-site exceptions,
//! memory budgets overrides and appearance. Persisted as JSON.

use browser_core::{ByteSize, fs::write_atomic};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;
use std::time::Duration;

/// How inactive tabs behave. `SmartPause` is the default.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum BackgroundMode {
    /// Normal browser background behaviour (engine-level throttling only).
    Normal,
    /// Stop unnecessary activity while preserving the live page.
    #[default]
    SmartPause,
    /// Aggressively suspend timers, scripts and network activity.
    DeepSleep,
    /// Stop all tab-initiated network activity while retaining page state.
    StrictOffline,
}

/// Per-site override of the global [`BackgroundMode`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SiteMode {
    AlwaysActive,
    SmartPause,
    DeepSleep,
    StrictOffline,
}

impl SiteMode {
    pub fn to_background_mode(self) -> BackgroundMode {
        match self {
            SiteMode::AlwaysActive => BackgroundMode::Normal,
            SiteMode::SmartPause => BackgroundMode::SmartPause,
            SiteMode::DeepSleep => BackgroundMode::DeepSleep,
            SiteMode::StrictOffline => BackgroundMode::StrictOffline,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SiteSettings {
    pub mode: Option<SiteMode>,
    /// "Refresh content when returning to this tab". Off unless set.
    pub refresh_on_return: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LifecycleTimings {
    /// Background-warm → frozen after this much inactivity (SmartPause).
    pub freeze_after: Duration,
    /// DeepSleep freezes sooner.
    pub deep_sleep_freeze_after: Duration,
    /// Frozen → discarded after this long, only if memory pressure is not Normal.
    pub discard_after: Duration,
}

impl Default for LifecycleTimings {
    fn default() -> Self {
        Self {
            freeze_after: Duration::from_secs(5 * 60),
            deep_sleep_freeze_after: Duration::from_secs(30),
            discard_after: Duration::from_secs(60 * 60),
        }
    }
}

/// Optional explicit overrides; `None` means "derive adaptively from system RAM".
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct MemoryOverrides {
    pub global_limit: Option<ByteSize>,
    pub tab_soft_limit: Option<ByteSize>,
    pub tab_hard_limit: Option<ByteSize>,
    pub image_cache_limit: Option<ByteSize>,
    pub font_cache_limit: Option<ByteSize>,
    pub http_cache_limit: Option<ByteSize>,
    pub gpu_budget: Option<ByteSize>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Theme {
    Light,
    Dark,
    #[default]
    System,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Density {
    Compact,
    #[default]
    Comfortable,
    Spacious,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Appearance {
    pub theme: Theme,
    /// sRGB accent colour.
    pub accent: [u8; 3],
    pub density: Density,
    pub sidebar_collapsed: bool,
    /// File name of the selected custom theme in the profile's `themes/` folder.
    #[serde(default)]
    pub custom_theme: Option<String>,
}

impl Default for Appearance {
    fn default() -> Self {
        Self { theme: Theme::System, accent: [0x4F, 0x7C, 0xFF], density: Density::Comfortable, sidebar_collapsed: false, custom_theme: None }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub background_mode: BackgroundMode,
    pub sites: BTreeMap<String, SiteSettings>,
    pub timings: LifecycleTimings,
    pub memory: MemoryOverrides,
    pub appearance: Appearance,
    pub home_page: String,
    pub search_template: String,
    /// Downloads started by the user are never suppressed unless this is set.
    pub suppress_background_downloads: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            background_mode: BackgroundMode::default(),
            sites: BTreeMap::new(),
            timings: LifecycleTimings::default(),
            memory: MemoryOverrides::default(),
            appearance: Appearance::default(),
            home_page: "about:blank".into(),
            search_template: "https://duckduckgo.com/?q={}".into(),
            suppress_background_downloads: false,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SettingsError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("parse: {0}")]
    Parse(#[from] serde_json::Error),
}

impl Settings {
    /// Effective background mode for a host, honouring per-site exceptions.
    pub fn mode_for_host(&self, host: Option<&str>) -> BackgroundMode {
        host.and_then(|h| self.site_entry(h))
            .and_then(|s| s.mode)
            .map(SiteMode::to_background_mode)
            .unwrap_or(self.background_mode)
    }

    pub fn refresh_on_return_for_host(&self, host: Option<&str>) -> bool {
        host.and_then(|h| self.site_entry(h)).is_some_and(|s| s.refresh_on_return)
    }

    /// Exact host match first, then parent domains (`a.b.example.com` → `example.com`).
    fn site_entry(&self, host: &str) -> Option<&SiteSettings> {
        let mut h = host.to_ascii_lowercase();
        loop {
            if let Some(s) = self.sites.get(&h) {
                return Some(s);
            }
            match h.split_once('.') {
                Some((_, rest)) if rest.contains('.') => h = rest.to_string(),
                _ => return None,
            }
        }
    }

    pub fn load(path: &Path) -> Result<Self, SettingsError> {
        match std::fs::read(path) {
            Ok(b) => Ok(serde_json::from_slice(&b)?),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e.into()),
        }
    }

    pub fn save(&self, path: &Path) -> Result<(), SettingsError> {
        write_atomic(path, &serde_json::to_vec_pretty(self)?)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_smart_pause_and_refresh_off() {
        let s = Settings::default();
        assert_eq!(s.background_mode, BackgroundMode::SmartPause);
        assert!(!s.refresh_on_return_for_host(Some("example.com")));
    }

    #[test]
    fn site_exception_applies_to_subdomains() {
        let mut s = Settings::default();
        s.sites.insert("example.com".into(), SiteSettings { mode: Some(SiteMode::AlwaysActive), refresh_on_return: true });
        assert_eq!(s.mode_for_host(Some("mail.example.com")), BackgroundMode::Normal);
        assert_eq!(s.mode_for_host(Some("other.org")), BackgroundMode::SmartPause);
        assert!(s.refresh_on_return_for_host(Some("a.b.example.com")));
        assert_eq!(s.mode_for_host(None), BackgroundMode::SmartPause);
    }

    #[test]
    fn settings_files_from_before_custom_themes_still_load() {
        let old = r#"{"appearance":{"theme":"Dark","accent":[1,2,3],"density":"Compact","sidebar_collapsed":true}}"#;
        let s: Settings = serde_json::from_str(old).unwrap();
        assert_eq!(s.appearance.custom_theme, None);
        assert!(s.appearance.sidebar_collapsed);
    }

    #[test]
    fn round_trips_through_disk_and_tolerates_missing_file() {
        let dir = std::env::temp_dir().join(format!("browserust-settings-{}", std::process::id()));
        let p = dir.join("settings.json");
        assert_eq!(Settings::load(&p).unwrap().home_page, "about:blank");
        let mut s = Settings::default();
        s.background_mode = BackgroundMode::DeepSleep;
        s.save(&p).unwrap();
        assert_eq!(Settings::load(&p).unwrap().background_mode, BackgroundMode::DeepSleep);
        std::fs::remove_dir_all(dir).ok();
    }
}
