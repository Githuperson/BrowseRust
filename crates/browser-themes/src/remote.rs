//! Fetching a theme's remote files (only after the user consented in the shell).

use crate::images::{Decoded, decode_image, sniff};
use crate::parse::{Theme, parse_theme, parse_theme_with};
use std::io::Read;
use std::path::PathBuf;
use std::time::Duration;
use url::Url;

const MAX_CSS_BYTES: usize = crate::MAX_THEME_BYTES;
const MAX_IMAGE_BYTES: usize = 8 * 1024 * 1024;
const MAX_IMPORTS: usize = 4;
const CACHE_LIMIT_BYTES: u64 = 64 * 1024 * 1024;

/// FNV-1a 64-bit, hex. Stable across runs (used for cache names and consent fingerprints).
fn fnv(s: &str) -> String {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in s.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("{h:016x}")
}

/// Fingerprint of a set of URLs. Consent is stored against this, so a theme that is edited
/// to download something new must be approved again.
pub fn fingerprint(urls: &[String]) -> String {
    let mut v = urls.to_vec();
    v.sort();
    v.dedup();
    fnv(&v.join("\n"))
}

/// Is this URL acceptable to fetch? https only, no embedded credentials, and never a
/// private/loopback/link-local address (a theme must not be able to poke local services).
/// `allow_loopback` exists for tests with a local server.
pub fn is_allowed_url(u: &str, allow_loopback: bool) -> bool {
    let Ok(url) = Url::parse(u) else { return false };
    if !url.username().is_empty() || url.password().is_some() {
        return false;
    }
    let loopback = match url.host() {
        Some(url::Host::Domain(d)) => d == "localhost" || d.ends_with(".localhost"),
        Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
        Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
        None => return false,
    };
    let private = match url.host() {
        Some(url::Host::Ipv4(ip)) => ip.is_private() || ip.is_link_local() || ip.is_unspecified() || ip.is_broadcast(),
        Some(url::Host::Ipv6(ip)) => ip.is_unspecified() || (ip.segments()[0] & 0xfe00) == 0xfc00 || (ip.segments()[0] & 0xffc0) == 0xfe80,
        _ => false,
    };
    match url.scheme() {
        "https" => !loopback && !private,
        "http" => allow_loopback && loopback,
        _ => false,
    }
}

/// On-disk cache of downloaded theme files, keyed by URL.
pub struct AssetCache {
    dir: PathBuf,
    pub allow_loopback: bool,
}

impl AssetCache {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir, allow_loopback: false }
    }

    fn path(&self, url: &str) -> PathBuf {
        self.dir.join(format!("{}.bin", fnv(url)))
    }

    pub fn get(&self, url: &str) -> Option<Vec<u8>> {
        std::fs::read(self.path(url)).ok()
    }

    fn put(&self, url: &str, bytes: &[u8]) {
        if std::fs::create_dir_all(&self.dir).is_err() {
            return;
        }
        // Keep the cache bounded: drop the oldest files while over the limit.
        let mut files: Vec<_> = std::fs::read_dir(&self.dir).map(|d| d.flatten().filter_map(|e| Some((e.path(), e.metadata().ok()?))).collect()).unwrap_or_default();
        let mut total: u64 = files.iter().map(|(_, m): &(PathBuf, std::fs::Metadata)| m.len()).sum();
        files.sort_by_key(|(_, m)| m.modified().ok());
        for (p, m) in files {
            if total + bytes.len() as u64 <= CACHE_LIMIT_BYTES {
                break;
            }
            if std::fs::remove_file(&p).is_ok() {
                total = total.saturating_sub(m.len());
            }
        }
        let _ = std::fs::write(self.path(url), bytes);
    }

    pub fn clear(&self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }

    /// GET with manual, validated redirects and a hard size cap. No cookies, no auth.
    fn download(&self, url: &str, max: usize) -> Result<Vec<u8>, String> {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .max_redirects(0)
            .http_status_as_error(false)
            .timeout_global(Some(Duration::from_secs(25)))
            .build()
            .into();
        let mut cur = url.to_string();
        for _ in 0..4 {
            if !is_allowed_url(&cur, self.allow_loopback) {
                return Err(format!("blocked URL: {cur}"));
            }
            let resp = agent.get(&cur).header("User-Agent", "BrowseRust-theme-loader").call().map_err(|e| e.to_string())?;
            let status = resp.status().as_u16();
            if matches!(status, 301 | 302 | 303 | 307 | 308) {
                let loc = resp.headers().get("location").and_then(|v| v.to_str().ok()).ok_or("redirect without Location")?;
                cur = Url::parse(&cur).and_then(|b| b.join(loc)).map_err(|e| e.to_string())?.to_string();
                continue;
            }
            if status != 200 {
                return Err(format!("HTTP {status} for {url}"));
            }
            let mut buf = Vec::new();
            resp.into_body().into_reader().take(max as u64 + 1).read_to_end(&mut buf).map_err(|e| e.to_string())?;
            if buf.len() > max {
                return Err(format!("{url} is larger than {} KiB", max / 1024));
            }
            return Ok(buf);
        }
        Err("too many redirects".into())
    }

    /// Cached bytes, else (if `network`) a download that is cached on success.
    fn fetch(&self, url: &str, max: usize, network: bool) -> Result<(Vec<u8>, bool), String> {
        if let Some(b) = self.get(url) {
            return Ok((b, true));
        }
        if !network {
            return Err("not downloaded yet".into());
        }
        let b = self.download(url, max)?;
        self.put(url, &b);
        Ok((b, false))
    }
}

pub struct Loaded {
    /// The theme with imported variables merged in (what the shell should use).
    pub theme: Theme,
    pub background: Option<Decoded>,
    pub home: Option<Decoded>,
    pub downloaded: usize,
    pub from_cache: usize,
    pub errors: Vec<String>,
}

/// Resolve a theme's remote parts. With `network == false` only the cache is used.
///
/// Only URLs written in the theme file itself are fetched for images (the set the user
/// consented to); an imported stylesheet can add colour variables but cannot make us
/// download further files.
pub fn load_remote(src: &str, cache: &AssetCache, network: bool) -> Result<Loaded, String> {
    let local = parse_theme(src)?;
    let (mut downloaded, mut from_cache, mut errors) = (0, 0, Vec::new());
    let mut note = |r: Result<(Vec<u8>, bool), String>, what: &str, errors: &mut Vec<String>| -> Option<Vec<u8>> {
        match r {
            Ok((b, cached)) => {
                if cached { from_cache += 1 } else { downloaded += 1 }
                Some(b)
            }
            Err(e) => {
                errors.push(format!("{what}: {e}"));
                None
            }
        }
    };

    let mut css = Vec::new();
    for u in local.imports.iter().take(MAX_IMPORTS) {
        if let Some(b) = note(cache.fetch(u, MAX_CSS_BYTES, network), u, &mut errors) {
            match String::from_utf8(b) {
                Ok(t) => css.push(t),
                Err(_) => errors.push(format!("{u}: not text")),
            }
        }
    }
    let mut theme = parse_theme_with(src, &css)?;
    // Image URLs come from the theme file only.
    theme.background_image = local.background_image.clone();
    theme.home_image = local.home_image.clone();
    theme.imports = local.imports.clone();
    if css.len() == local.imports.len().min(MAX_IMPORTS) {
        theme.warnings.retain(|w| !w.starts_with("@import not loaded yet"));
    }

    let mut image = |u: &Option<String>, max_dim: u32, errors: &mut Vec<String>| -> Option<Decoded> {
        let u = u.as_ref()?;
        let bytes = note(cache.fetch(u, MAX_IMAGE_BYTES, network), u, errors)?;
        if sniff(&bytes).is_none() {
            errors.push(format!("{u}: not a PNG, JPEG, GIF or WebP image"));
            return None;
        }
        match decode_image(&bytes, max_dim) {
            Ok(d) => Some(d),
            Err(e) => {
                errors.push(format!("{u}: {e}"));
                None
            }
        }
    };
    let background = image(&local.background_image, 1600, &mut errors);
    let home = image(&local.home_image, 256, &mut errors);
    Ok(Loaded { theme, background, home, downloaded, from_cache, errors })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_policy() {
        assert!(is_allowed_url("https://i.imgur.com/a.png", false));
        assert!(!is_allowed_url("http://i.imgur.com/a.png", false), "no plain http");
        assert!(!is_allowed_url("https://user:pw@example.com/a.png", false), "no credentials");
        assert!(!is_allowed_url("https://localhost/a.png", false));
        assert!(!is_allowed_url("https://127.0.0.1/a.png", false));
        assert!(!is_allowed_url("https://192.168.1.1/a.png", false), "private network");
        assert!(!is_allowed_url("https://10.0.0.5/a.png", false));
        assert!(!is_allowed_url("https://169.254.169.254/latest/meta-data", false), "cloud metadata");
        assert!(!is_allowed_url("https://[::1]/a.png", false));
        assert!(!is_allowed_url("file:///etc/passwd", false));
        assert!(!is_allowed_url("ftp://example.com/a.png", false));
        assert!(!is_allowed_url("not a url", false));
        assert!(is_allowed_url("http://127.0.0.1:8080/a.png", true), "loopback only when explicitly enabled");
        assert!(!is_allowed_url("http://192.168.1.1/a.png", true));
    }

    #[test]
    fn fingerprints_are_order_independent_and_change_with_urls() {
        let a = fingerprint(&["https://a/1".into(), "https://b/2".into()]);
        assert_eq!(a, fingerprint(&["https://b/2".into(), "https://a/1".into(), "https://a/1".into()]));
        assert_ne!(a, fingerprint(&["https://a/1".into(), "https://b/3".into()]));
    }
}
