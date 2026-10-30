use crate::{MAX_THEME_BYTES, Theme, parse_theme};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

#[derive(Clone, Debug)]
pub struct ThemeEntry {
    /// File name (the stable id stored in settings), e.g. `midnight.theme.css`.
    pub file: String,
    pub theme: Theme,
    /// Set when the file could not be read or was too large.
    pub error: Option<String>,
}

impl ThemeEntry {
    pub fn display_name(&self) -> &str {
        if self.theme.name.is_empty() { self.file.trim_end_matches(".theme.css").trim_end_matches(".css") } else { &self.theme.name }
    }
}

/// A themes directory. Files ending in `.css` are themes; names starting with `_` are skipped.
pub struct ThemeStore {
    dir: PathBuf,
    signature: Vec<(String, u64, Option<SystemTime>)>,
}

const MIDNIGHT: &str = include_str!("../examples/midnight.theme.css");
const PAPER: &str = include_str!("../examples/paper.theme.css");
const TEMPLATE: &str = include_str!("../examples/_template.theme.css");

impl ThemeStore {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir, signature: Vec::new() }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Create the folder with starter themes the first time only; never overwrites user files.
    pub fn ensure_examples(&self) -> std::io::Result<()> {
        if self.dir.exists() {
            return Ok(());
        }
        std::fs::create_dir_all(&self.dir)?;
        for (name, body) in [("midnight.theme.css", MIDNIGHT), ("paper.theme.css", PAPER), ("_template.theme.css", TEMPLATE)] {
            std::fs::write(self.dir.join(name), body)?;
        }
        Ok(())
    }

    fn listing(&self) -> Vec<(String, u64, Option<SystemTime>)> {
        let mut v: Vec<_> = std::fs::read_dir(&self.dir)
            .map(|rd| {
                rd.flatten()
                    .filter_map(|e| {
                        let name = e.file_name().to_string_lossy().into_owned();
                        if !name.ends_with(".css") || name.starts_with('_') || name.starts_with('.') {
                            return None;
                        }
                        let md = e.metadata().ok()?;
                        md.is_file().then(|| (name, md.len(), md.modified().ok()))
                    })
                    .collect()
            })
            .unwrap_or_default();
        v.sort_by(|a, b| a.0.to_lowercase().cmp(&b.0.to_lowercase()));
        v
    }

    /// Read every theme file.
    pub fn scan(&mut self) -> Vec<ThemeEntry> {
        self.signature = self.listing();
        self.signature
            .iter()
            .map(|(file, len, _)| {
                let path = self.dir.join(file);
                let (theme, error) = if *len as usize > MAX_THEME_BYTES {
                    (Theme::default(), Some(format!("larger than {} KiB", MAX_THEME_BYTES / 1024)))
                } else {
                    match std::fs::read_to_string(&path) {
                        Ok(src) => match parse_theme(&src) {
                            Ok(t) => (t, None),
                            Err(e) => (Theme::default(), Some(e)),
                        },
                        Err(e) => (Theme::default(), Some(e.to_string())),
                    }
                };
                ThemeEntry { file: file.clone(), theme, error }
            })
            .collect()
    }

    /// `true` if a theme file was added, removed or modified since the last `scan`.
    pub fn changed(&self) -> bool {
        self.listing() != self.signature
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(n: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("browserust-themes-{n}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn first_run_creates_examples_and_they_parse() {
        let d = tmp("first");
        let mut s = ThemeStore::new(d.clone());
        s.ensure_examples().unwrap();
        let t = s.scan();
        let names: Vec<_> = t.iter().map(|e| e.file.as_str()).collect();
        assert_eq!(names, vec!["midnight.theme.css", "paper.theme.css"], "template (_) is skipped");
        assert!(t.iter().all(|e| e.error.is_none() && e.theme.colors.is_some()), "{t:?}");
        assert!(t[0].theme.colors.unwrap().dark && !t[1].theme.colors.unwrap().dark);
        assert!(d.join("_template.theme.css").exists());
        std::fs::remove_dir_all(d).ok();
    }

    #[test]
    fn existing_folder_is_never_overwritten() {
        let d = tmp("keep");
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("mine.theme.css"), ":root{--bp-bar:#123456}").unwrap();
        let s = ThemeStore::new(d.clone());
        s.ensure_examples().unwrap();
        assert!(!d.join("midnight.theme.css").exists());
        std::fs::remove_dir_all(d).ok();
    }

    #[test]
    fn detects_edits_additions_and_removals() {
        let d = tmp("watch");
        std::fs::create_dir_all(&d).unwrap();
        let mut s = ThemeStore::new(d.clone());
        assert!(s.scan().is_empty());
        assert!(!s.changed());
        std::fs::write(d.join("a.css"), ":root{--bp-bar:#111111}").unwrap();
        assert!(s.changed(), "added");
        s.scan();
        std::fs::write(d.join("a.css"), ":root{--bp-bar:#222222; --bp-accent:#ff0000}").unwrap();
        assert!(s.changed(), "edited (size differs)");
        s.scan();
        std::fs::remove_file(d.join("a.css")).unwrap();
        assert!(s.changed(), "removed");
        std::fs::remove_dir_all(d).ok();
    }

    #[test]
    fn bad_files_become_entries_with_errors_not_panics() {
        let d = tmp("bad");
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("big.css"), "a".repeat(MAX_THEME_BYTES + 10)).unwrap();
        std::fs::write(d.join("bin.css"), [0xff, 0xfe, 0x00]).unwrap();
        let mut s = ThemeStore::new(d.clone());
        let t = s.scan();
        assert_eq!(t.len(), 2);
        assert!(t.iter().all(|e| e.error.is_some()));
        std::fs::remove_dir_all(d).ok();
    }
}
