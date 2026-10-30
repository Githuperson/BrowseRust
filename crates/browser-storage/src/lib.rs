//! Persistent browser data. SQLite (via `rusqlite`) for history and bookmarks;
//! the OS credential store (via `keyring`) for passwords. Private browsing uses
//! `Store::in_memory()`, which never touches disk.

pub mod passwords;

use rusqlite::{Connection, params};
use std::path::Path;
use url::Url;

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("sqlite: {0}")]
    Sql(#[from] rusqlite::Error),
}
type Result<T> = std::result::Result<T, StoreError>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HistoryEntry {
    pub url: String,
    pub title: String,
    pub visits: u32,
    pub last_visit: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Bookmark {
    pub id: i64,
    pub url: String,
    pub title: String,
    pub folder: String,
}

pub struct Store {
    db: Connection,
}

/// Only these URLs are ever recorded (no about:, file:, data:, javascript:).
fn recordable(u: &str) -> bool {
    Url::parse(u).is_ok_and(|u| matches!(u.scheme(), "http" | "https"))
}

impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(d) = path.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        Self::init(Connection::open(path)?)
    }

    pub fn in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(db: Connection) -> Result<Self> {
        db.execute_batch(
            "CREATE TABLE IF NOT EXISTS history(url TEXT PRIMARY KEY, title TEXT NOT NULL, visits INTEGER NOT NULL, last_visit INTEGER NOT NULL);
             CREATE TABLE IF NOT EXISTS bookmarks(id INTEGER PRIMARY KEY, url TEXT NOT NULL UNIQUE, title TEXT NOT NULL, folder TEXT NOT NULL);",
        )?;
        Ok(Self { db })
    }

    /// Record a visit; `now` is unix seconds.
    pub fn record_visit(&self, url: &str, title: &str, now: i64) -> Result<()> {
        if !recordable(url) {
            return Ok(());
        }
        self.db.execute(
            "INSERT INTO history(url,title,visits,last_visit) VALUES(?1,?2,1,?3)
             ON CONFLICT(url) DO UPDATE SET visits=visits+1, last_visit=?3, title=CASE WHEN ?2<>'' THEN ?2 ELSE title END",
            params![url, title, now],
        )?;
        Ok(())
    }

    pub fn recent(&self, limit: usize) -> Result<Vec<HistoryEntry>> {
        self.query_history("SELECT url,title,visits,last_visit FROM history ORDER BY last_visit DESC LIMIT ?1", params![limit as i64])
    }

    /// Frecency-ranked matches for the address bar: substring match on url/title,
    /// score = visits weighted by recency.
    pub fn suggest(&self, text: &str, now: i64, limit: usize) -> Result<Vec<HistoryEntry>> {
        let t = text.trim();
        if t.is_empty() {
            return Ok(vec![]);
        }
        let like = format!("%{}%", t.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_"));
        let mut v = self.query_history(
            "SELECT url,title,visits,last_visit FROM history WHERE url LIKE ?1 ESCAPE '\\' OR title LIKE ?1 ESCAPE '\\'",
            params![like],
        )?;
        let score = |e: &HistoryEntry| {
            let age_days = ((now - e.last_visit).max(0) as f64) / 86400.0;
            e.visits as f64 / (1.0 + age_days / 7.0)
        };
        v.sort_by(|a, b| score(b).total_cmp(&score(a)));
        v.truncate(limit);
        Ok(v)
    }

    pub fn clear_history(&self) -> Result<()> {
        self.db.execute("DELETE FROM history", [])?;
        Ok(())
    }

    pub fn delete_history_entry(&self, url: &str) -> Result<()> {
        self.db.execute("DELETE FROM history WHERE url=?1", params![url])?;
        Ok(())
    }

    fn query_history(&self, sql: &str, p: impl rusqlite::Params) -> Result<Vec<HistoryEntry>> {
        let mut st = self.db.prepare(sql)?;
        let rows = st.query_map(p, |r| Ok(HistoryEntry { url: r.get(0)?, title: r.get(1)?, visits: r.get(2)?, last_visit: r.get(3)? }))?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    // ---- bookmarks
    pub fn add_bookmark(&self, url: &str, title: &str, folder: &str) -> Result<()> {
        if !recordable(url) {
            return Ok(());
        }
        self.db.execute(
            "INSERT INTO bookmarks(url,title,folder) VALUES(?1,?2,?3) ON CONFLICT(url) DO UPDATE SET title=?2, folder=?3",
            params![url, title, folder],
        )?;
        Ok(())
    }

    pub fn remove_bookmark(&self, url: &str) -> Result<()> {
        self.db.execute("DELETE FROM bookmarks WHERE url=?1", params![url])?;
        Ok(())
    }

    pub fn is_bookmarked(&self, url: &str) -> Result<bool> {
        Ok(self.db.query_row("SELECT COUNT(*) FROM bookmarks WHERE url=?1", params![url], |r| r.get::<_, i64>(0))? > 0)
    }

    pub fn bookmarks(&self) -> Result<Vec<Bookmark>> {
        let mut st = self.db.prepare("SELECT id,url,title,folder FROM bookmarks ORDER BY folder, title")?;
        let rows = st.query_map([], |r| Ok(Bookmark { id: r.get(0)?, url: r.get(1)?, title: r.get(2)?, folder: r.get(3)? }))?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    /// Export as the Netscape bookmark HTML format every browser imports.
    pub fn export_html(&self) -> Result<String> {
        let esc = |s: &str| s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;");
        let mut out = String::from("<!DOCTYPE NETSCAPE-Bookmark-file-1>\n<META HTTP-EQUIV=\"Content-Type\" CONTENT=\"text/html; charset=UTF-8\">\n<TITLE>Bookmarks</TITLE>\n<DL><p>\n");
        for b in self.bookmarks()? {
            out.push_str(&format!("<DT><A HREF=\"{}\" FOLDER=\"{}\">{}</A>\n", esc(&b.url), esc(&b.folder), esc(&b.title)));
        }
        out.push_str("</DL><p>\n");
        Ok(out)
    }

    /// Import links from a Netscape-format file (ours or another browser's).
    /// Returns the number imported; non-http(s) links are skipped.
    pub fn import_html(&self, html: &str) -> Result<usize> {
        let unesc = |s: &str| s.replace("&quot;", "\"").replace("&lt;", "<").replace("&gt;", ">").replace("&amp;", "&");
        let mut n = 0;
        let lower = html.to_ascii_lowercase();
        let mut pos = 0;
        while let Some(i) = lower[pos..].find("<a ") {
            let start = pos + i;
            let Some(gt) = lower[start..].find('>') else { break };
            let tag = &html[start..start + gt];
            let end = lower[start + gt..].find("</a>").map(|e| start + gt + e).unwrap_or(html.len());
            let title = unesc(html[start + gt + 1..end].trim());
            let attr = |name: &str| {
                let l = tag.to_ascii_lowercase();
                let k = format!("{name}=\"");
                l.find(&k).and_then(|p| tag[p + k.len()..].split('"').next()).map(unesc)
            };
            if let Some(href) = attr("href") {
                if recordable(&href) {
                    self.add_bookmark(&href, &title, &attr("folder").unwrap_or_default())?;
                    n += 1;
                }
            }
            pos = end.max(start + 1);
        }
        Ok(n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn visits_accumulate_and_only_web_urls_are_recorded() {
        let s = Store::in_memory().unwrap();
        s.record_visit("https://a.test/", "A", 100).unwrap();
        s.record_visit("https://a.test/", "", 200).unwrap();
        s.record_visit("file:///etc/passwd", "x", 100).unwrap();
        s.record_visit("javascript:alert(1)", "x", 100).unwrap();
        let r = s.recent(10).unwrap();
        assert_eq!(r.len(), 1);
        assert_eq!((r[0].visits, r[0].last_visit, r[0].title.as_str()), (2, 200, "A"), "empty title keeps the old one");
    }

    #[test]
    fn suggestions_rank_by_frecency_and_escape_wildcards() {
        let s = Store::in_memory().unwrap();
        let day = 86400;
        let now = 100 * day;
        s.record_visit("https://old.test/rust", "old", now - 90 * day).unwrap();
        for _ in 0..3 {
            s.record_visit("https://new.test/rust", "new", now).unwrap();
        }
        s.record_visit("https://x.test/100%", "pct", now).unwrap();
        let r = s.suggest("rust", now, 5).unwrap();
        assert_eq!(r[0].url, "https://new.test/rust");
        assert_eq!(r.len(), 2);
        assert_eq!(s.suggest("%", now, 5).unwrap().len(), 1, "% is literal, not a wildcard");
        assert!(s.suggest("  ", now, 5).unwrap().is_empty());
    }

    #[test]
    fn history_can_be_cleared_per_entry_and_entirely() {
        let s = Store::in_memory().unwrap();
        s.record_visit("https://a.test/", "A", 1).unwrap();
        s.record_visit("https://b.test/", "B", 1).unwrap();
        s.delete_history_entry("https://a.test/").unwrap();
        assert_eq!(s.recent(10).unwrap().len(), 1);
        s.clear_history().unwrap();
        assert!(s.recent(10).unwrap().is_empty());
    }

    #[test]
    fn bookmarks_add_update_remove_and_round_trip_html() {
        let s = Store::in_memory().unwrap();
        s.add_bookmark("https://a.test/?x=1&y=2", "A <b>&co", "Work").unwrap();
        s.add_bookmark("https://a.test/?x=1&y=2", "A renamed", "Work").unwrap();
        s.add_bookmark("javascript:alert(1)", "bad", "").unwrap();
        assert_eq!(s.bookmarks().unwrap().len(), 1);
        assert!(s.is_bookmarked("https://a.test/?x=1&y=2").unwrap());
        s.add_bookmark("https://b.test/", "Quote\"d", "").unwrap();
        let html = s.export_html().unwrap();
        let t = Store::in_memory().unwrap();
        assert_eq!(t.import_html(&html).unwrap(), 2);
        let mut got = t.bookmarks().unwrap();
        got.sort_by(|a, b| a.url.cmp(&b.url));
        assert_eq!(got[0].title, "A renamed");
        assert_eq!(got[0].folder, "Work");
        assert_eq!(got[1].title, "Quote\"d");
        s.remove_bookmark("https://b.test/").unwrap();
        assert!(!s.is_bookmarked("https://b.test/").unwrap());
    }

    #[test]
    fn import_skips_unsafe_links() {
        let s = Store::in_memory().unwrap();
        let n = s.import_html("<DT><A HREF=\"javascript:x\">j</A><DT><A HREF=\"https://ok.test/\">ok</A>").unwrap();
        assert_eq!(n, 1);
    }

    #[test]
    fn persistent_store_survives_reopen() {
        let p = std::env::temp_dir().join(format!("browserust-store-{}", std::process::id())).join("data.db");
        {
            Store::open(&p).unwrap().record_visit("https://a.test/", "A", 5).unwrap();
        }
        assert_eq!(Store::open(&p).unwrap().recent(5).unwrap().len(), 1);
        std::fs::remove_dir_all(p.parent().unwrap()).ok();
    }
}
