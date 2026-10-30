//! Real-Servo test of shell-level downloads.
//! Run: cargo test -p servo-integration --test downloads_e2e -- --nocapture

mod common;
use browser_downloads::{DownloadManager, JobState};
use browser_settings::Settings;
use common::*;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use url::Url;

fn serve() -> (u16, Arc<Mutex<Vec<String>>>) {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    let log: Arc<Mutex<Vec<String>>> = Arc::default();
    let log2 = log.clone();
    std::thread::spawn(move || {
        for st in l.incoming().flatten() {
            let log = log2.clone();
            std::thread::spawn(move || {
                let mut st = st;
                let mut buf = [0u8; 8192];
                let n = st.read(&mut buf).unwrap_or(0);
                let req = String::from_utf8_lossy(&buf[..n]).to_string();
                let path = req.split_whitespace().nth(1).unwrap_or("/").to_string();
                let has_cookie = req.to_ascii_lowercase().contains("cookie: sid=abc");
                log.lock().unwrap().push(format!("{path} cookie={has_cookie}"));
                let html = |title: &str| format!("<!doctype html><title>{title}</title><body>{title}</body>");
                let (status, headers, body): (&str, String, Vec<u8>) = match path.as_str() {
                    "/page" | "/page2" => ("200 OK", "Content-Type: text/html\r\n".into(), html(path.trim_start_matches('/')).into_bytes()),
                    "/login" => ("200 OK", "Content-Type: text/html\r\nSet-Cookie: sid=abc; Path=/\r\n".into(), html("logged-in").into_bytes()),
                    "/file.zip" => ("200 OK", "Content-Type: application/zip\r\n".into(), vec![7u8; 200_000]),
                    "/secret.zip" if has_cookie => ("200 OK", "Content-Type: application/octet-stream\r\nContent-Disposition: attachment; filename=\"secret.bin\"\r\n".into(), vec![9u8; 50_000]),
                    "/secret.zip" => ("403 Forbidden", "Content-Type: text/html\r\n".into(), html("denied").into_bytes()),
                    _ => ("404 Not Found", "Content-Type: text/plain\r\n".into(), b"nf".to_vec()),
                };
                let _ = st.write_all(format!("HTTP/1.1 {status}\r\n{headers}Content-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\n\r\n", body.len()).as_bytes());
                let _ = st.write_all(&body);
            });
        }
    });
    (port, log)
}

#[test]
fn attachments_become_downloads_and_normal_pages_still_load() {
    let (port, log) = serve();
    let dir = std::env::temp_dir().join(format!("browserust-e2e-dl-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let mgr = Arc::new(DownloadManager::new(dir.clone()));
    let mut b = new_browser_with(Settings::default(), Some(mgr.clone()));
    let base = format!("http://127.0.0.1:{port}");

    let a = b.open_tab(Some(Url::parse(&format!("{base}/page")).unwrap()), true);
    wait_for(&mut b, "page to load", Duration::from_secs(60), |b| b.tabs.strip.tab(a).is_some_and(|t| t.title == "page"));

    // 1. Navigating to a zip: becomes a download; the page stays.
    b.navigate_input(&format!("127.0.0.1:{port}/file.zip")).unwrap();
    wait_until(&mut b, "download to finish", Duration::from_secs(30), || mgr.snapshot().first().is_some_and(|j| j.state == JobState::Done));
    let j = &mgr.snapshot()[0];
    println!("download 1: {} -> {:?} ({} bytes, {:?})", j.filename, j.dest, j.received, j.state);
    assert_eq!(j.filename, "file.zip");
    assert_eq!(std::fs::read(dir.join("file.zip")).unwrap().len(), 200_000);
    pump(&mut b, Duration::from_secs(1));
    let t = b.tabs.strip.tab(a).unwrap();
    println!("tab after download: title={:?} url={:?}", t.title, t.url.as_ref().map(|u| u.as_str()));
    assert_eq!(t.title, "page", "the current page stays visible after a download");

    // 2. A normal page still navigates (probe must not break rendering).
    b.navigate_input(&format!("127.0.0.1:{port}/page2")).unwrap();
    wait_for(&mut b, "page2 to load", Duration::from_secs(30), |b| b.tabs.strip.tab(a).is_some_and(|t| t.title == "page2"));
    assert_eq!(mgr.snapshot().len(), 1, "no spurious download for HTML");

    // 3. Authenticated download: cookie set by the page is used by our client.
    b.navigate_input(&format!("127.0.0.1:{port}/login")).unwrap();
    wait_for(&mut b, "login page", Duration::from_secs(30), |b| b.tabs.strip.tab(a).is_some_and(|t| t.title == "logged-in"));
    pump(&mut b, Duration::from_millis(500));
    b.navigate_input(&format!("127.0.0.1:{port}/secret.zip")).unwrap();
    wait_until(&mut b, "secret download", Duration::from_secs(30), || mgr.snapshot().len() == 2 && mgr.snapshot()[1].state != JobState::Running);
    let s = &mgr.snapshot()[1];
    println!("download 2: {} state={:?} err={:?}", s.filename, s.state, s.error);
    assert_eq!(s.state, JobState::Done, "cookie from Servo's jar was sent: {:?}", s.error);
    assert_eq!(s.filename, "secret.bin", "Content-Disposition name used");
    assert_eq!(std::fs::read(dir.join("secret.bin")).unwrap().len(), 50_000);

    let l = log.lock().unwrap();
    println!("server log: {l:?}");
    let _ = std::fs::remove_dir_all(dir);
}
