use browser_downloads::{DownloadManager, JobState, ProbeResult, RequestContext, filename_for, probe};
use parking_lot::Mutex;
use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};
use url::Url;

type Seen = Arc<Mutex<HashMap<String, String>>>; // path -> Cookie header seen ("" if none)

/// Tiny HTTP server. `redirect_to` is the base URL of another server for /redir.
fn server(redirect_to: Option<String>) -> (u16, Seen) {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    let seen: Seen = Arc::default();
    let s2 = seen.clone();
    std::thread::spawn(move || {
        for st in l.incoming().flatten() {
            let (seen, redirect_to) = (s2.clone(), redirect_to.clone());
            std::thread::spawn(move || {
                let mut st = st;
                let mut buf = [0u8; 4096];
                let n = st.read(&mut buf).unwrap_or(0);
                let req = String::from_utf8_lossy(&buf[..n]).to_string();
                let path = req.split_whitespace().nth(1).unwrap_or("/").to_string();
                let cookie = req.lines().find(|l| l.to_ascii_lowercase().starts_with("cookie:")).map(|l| l[7..].trim().to_string()).unwrap_or_default();
                seen.lock().insert(path.clone(), cookie);
                let head = |status: &str, extra: &str, len: usize| format!("HTTP/1.1 {status}\r\n{extra}Content-Length: {len}\r\nConnection: close\r\n\r\n");
                match path.as_str() {
                    "/file" => {
                        let body = vec![b'x'; 300_000];
                        let _ = st.write_all(head("200 OK", "Content-Type: application/octet-stream\r\nContent-Disposition: attachment; filename=\"../../evil.txt\"\r\n", body.len()).as_bytes());
                        let _ = st.write_all(&body);
                    }
                    "/html" => {
                        let _ = st.write_all(head("200 OK", "Content-Type: text/html\r\n", 5).as_bytes());
                        let _ = st.write_all(b"<hi/>");
                    }
                    "/bin" | "/after" => {
                        let _ = st.write_all(head("200 OK", "Content-Type: application/zip\r\n", 4).as_bytes());
                        let _ = st.write_all(b"PK\x03\x04");
                    }
                    "/redir" => {
                        let loc = format!("{}/after", redirect_to.unwrap_or_default());
                        let _ = st.write_all(head("302 Found", &format!("Location: {loc}\r\n"), 0).as_bytes());
                    }
                    "/err" => {
                        let _ = st.write_all(head("404 Not Found", "Content-Type: application/zip\r\n", 0).as_bytes());
                    }
                    "/slow" => {
                        let total = 20_000_000usize;
                        let _ = st.write_all(head("200 OK", "Content-Type: application/octet-stream\r\n", total).as_bytes());
                        let chunk = vec![0u8; 64 * 1024];
                        for _ in 0..(total / chunk.len()) {
                            if st.write_all(&chunk).is_err() {
                                return;
                            }
                            std::thread::sleep(Duration::from_millis(40));
                        }
                    }
                    _ => {
                        let _ = st.write_all(head("404 Not Found", "", 0).as_bytes());
                    }
                }
            });
        }
    });
    (port, seen)
}

fn ctx(port: u16, path: &str, cookie: Option<&str>) -> RequestContext {
    RequestContext { url: Url::parse(&format!("http://127.0.0.1:{port}{path}")).unwrap(), cookie: cookie.map(str::to_string), referer: None, user_agent: Some("BrowseRust-test".into()) }
}
fn tmpdir(n: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("browserust-dl-{n}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    d
}
fn wait(m: &DownloadManager, id: u64, want: JobState) -> browser_downloads::JobView {
    let end = Instant::now() + Duration::from_secs(20);
    loop {
        let v = m.snapshot().into_iter().find(|j| j.id == id).unwrap();
        if v.state == want || Instant::now() > end {
            return v;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}
fn no_cookies() -> Arc<dyn Fn(&Url) -> Option<String> + Send + Sync> {
    Arc::new(|_| None)
}

#[test]
fn probe_classifies_html_downloads_redirects_and_errors() {
    let (port, _) = server(None);
    let nc = |_: &Url| None;
    assert_eq!(probe(&ctx(port, "/html", None), &nc), ProbeResult::Render);
    assert_eq!(probe(&ctx(port, "/err", None), &nc), ProbeResult::Render, "error pages render");
    match probe(&ctx(port, "/file", None), &nc) {
        ProbeResult::Download { filename, total, .. } => {
            assert_eq!(filename, "evil.txt", "path traversal removed");
            assert_eq!(total, Some(300_000));
        }
        other => panic!("{other:?}"),
    }
    assert!(matches!(probe(&ctx(port, "/bin", None), &nc), ProbeResult::Download { .. }));
    assert_eq!(probe(&ctx(1, "/x", None), &nc), ProbeResult::Failed, "connection refused -> Failed, caller proceeds");
    let _ = filename_for;
}

#[test]
fn downloads_a_file_with_progress_and_never_overwrites() {
    let (port, _) = server(None);
    let dir = tmpdir("full");
    let m = DownloadManager::new(dir.clone());
    let a = m.start(ctx(port, "/file", None), "evil.txt".into(), Some(300_000), no_cookies());
    let v = wait(&m, a, JobState::Done);
    assert_eq!(v.state, JobState::Done, "{:?}", v.error);
    assert_eq!(v.received, 300_000);
    assert_eq!(std::fs::read(dir.join("evil.txt")).unwrap().len(), 300_000);
    assert!(!dir.join("evil.txt.part").exists());
    let b = m.start(ctx(port, "/file", None), "evil.txt".into(), None, no_cookies());
    let v2 = wait(&m, b, JobState::Done);
    assert_eq!(v2.dest.unwrap().file_name().unwrap(), "evil (1).txt", "second download gets a new name");
    assert!(dir.join("evil.txt").exists() && dir.join("evil (1).txt").exists());
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn cancel_stops_the_transfer_and_leaves_no_partial_file() {
    let (port, _) = server(None);
    let dir = tmpdir("cancel");
    let m = DownloadManager::new(dir.clone());
    let id = m.start(ctx(port, "/slow", None), "big.bin".into(), None, no_cookies());
    let end = Instant::now() + Duration::from_secs(10);
    while m.snapshot()[0].received < 200_000 && Instant::now() < end {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(m.active_count(), 1);
    m.cancel(id);
    let v = wait(&m, id, JobState::Cancelled);
    assert_eq!(v.state, JobState::Cancelled);
    assert!(v.received < 20_000_000);
    let leftovers: Vec<_> = std::fs::read_dir(&dir).map(|d| d.flatten().collect()).unwrap_or_default();
    assert!(leftovers.is_empty(), "no .part or partial file left: {leftovers:?}");
    m.clear_finished();
    assert!(m.snapshot().is_empty());
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn http_errors_fail_the_job_without_creating_files() {
    let (port, _) = server(None);
    let dir = tmpdir("err");
    let m = DownloadManager::new(dir.clone());
    let id = m.start(ctx(port, "/err", None), "x.zip".into(), None, no_cookies());
    let v = wait(&m, id, JobState::Failed);
    assert_eq!(v.state, JobState::Failed);
    assert!(v.error.unwrap().contains("404"));
    assert!(std::fs::read_dir(&dir).map(|d| d.count()).unwrap_or(0) == 0);
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn cookies_are_sent_to_their_own_origin_and_never_forwarded_across_a_redirect() {
    let (port_b, seen_b) = server(None);
    let (port_a, seen_a) = server(Some(format!("http://127.0.0.1:{port_b}")));
    let nc = |_: &Url| None; // origin B has no cookies of its own
    let r = probe(&ctx(port_a, "/redir", Some("session=SECRET")), &nc);
    assert!(matches!(r, ProbeResult::Download { .. }), "{r:?}");
    assert_eq!(seen_a.lock().get("/redir").map(String::as_str), Some("session=SECRET"), "origin A received its cookie");
    assert_eq!(seen_b.lock().get("/after").map(String::as_str), Some(""), "origin B must NOT receive A's cookie");

    // And when B does have its own cookie, that one (and only that one) is used.
    let own = |u: &Url| (u.port() == Some(port_b)).then(|| "b=1".to_string());
    let _ = probe(&ctx(port_a, "/redir", Some("session=SECRET")), &own);
    assert_eq!(seen_b.lock().get("/after").map(String::as_str), Some("b=1"));
}
