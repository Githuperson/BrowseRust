//! End-to-end test against the REAL Servo engine (software GL, no window).
//!
//! A local HTTP server counts what the pages actually send. Servo can only be
//! initialised once per process, so this file holds a single scenario.
//!
//! Run with:  cargo test -p servo-integration --test background_tabs -- --nocapture

use browser_app::Browser;
use browser_core::{ByteSize, Clock, SystemClock, TabId, TabState};
use browser_memory::probe::platform_probe;
use browser_settings::{BackgroundMode, Settings, SiteMode, SiteSettings};
use servo::{EventLoopWaker, RenderingContext, ServoBuilder, SoftwareRenderingContext};
use servo_integration::{ServoEngine, ServoEngineConfig};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use url::Url;

type Hits = Arc<Mutex<HashMap<String, usize>>>;

fn page(name: &str) -> String {
    format!(
        r#"<!doctype html><title>page {name}</title><body>{name}
<script>
window.__nonce = Math.random().toString(36).slice(2);
setInterval(function(){{ fetch('/fetch_{name}?'+Date.now()).catch(function(){{}}); }}, 200);
setInterval(function(){{ var x=new XMLHttpRequest(); x.open('GET','/xhr_{name}?'+Date.now()); x.send(); }}, 200);
setInterval(function(){{ fetch('/{name}/collect',{{method:'POST',body:'x'}}).catch(function(){{}}); }}, 200);
</script></body>"#
    )
}

fn start_server() -> (u16, Hits) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let hits: Hits = Arc::default();
    let h = hits.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut s) = stream else { continue };
            let h = h.clone();
            std::thread::spawn(move || {
                let mut buf = [0u8; 4096];
                let n = s.read(&mut buf).unwrap_or(0);
                let req = String::from_utf8_lossy(&buf[..n]).to_string();
                let path = req.split_whitespace().nth(1).unwrap_or("/").split('?').next().unwrap_or("/").to_string();
                *h.lock().unwrap().entry(path.clone()).or_default() += 1;
                let (ctype, body) = match path.as_str() {
                    "/a" => ("text/html", page("a")),
                    "/b" => ("text/html", page("b")),
                    _ => ("text/plain", "ok".to_string()),
                };
                let resp = format!("HTTP/1.1 200 OK\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\n\r\n{body}", body.len());
                let _ = s.write_all(resp.as_bytes());
            });
        }
    });
    (port, hits)
}

#[derive(Clone)]
struct NoopWaker;
impl EventLoopWaker for NoopWaker {
    fn clone_box(&self) -> Box<dyn EventLoopWaker> {
        Box::new(self.clone())
    }
    fn wake(&self) {}
}

fn count(h: &Hits, path: &str) -> usize {
    h.lock().unwrap().get(path).copied().unwrap_or(0)
}

fn pump(b: &mut Browser<ServoEngine>, for_: Duration) {
    let end = Instant::now() + for_;
    while Instant::now() < end {
        b.engine.spin();
        b.tick();
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn wait_until(b: &mut Browser<ServoEngine>, what: &str, timeout: Duration, mut cond: impl FnMut() -> bool) {
    let end = Instant::now() + timeout;
    while Instant::now() < end {
        b.engine.spin();
        b.tick();
        if cond() {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("timed out waiting for: {what}");
}

fn nonce(b: &mut Browser<ServoEngine>, tab: TabId) -> String {
    let out: Rc<std::cell::RefCell<Option<String>>> = Rc::default();
    let o = out.clone();
    b.engine.webview(tab).unwrap().evaluate_javascript("window.__nonce", move |r| {
        if let Ok(servo::JSValue::String(s)) = r {
            *o.borrow_mut() = Some(s);
        }
    });
    let end = Instant::now() + Duration::from_secs(10);
    while out.borrow().is_none() && Instant::now() < end {
        b.engine.spin();
        std::thread::sleep(Duration::from_millis(10));
    }
    let n = out.borrow().clone();
    n.expect("nonce evaluation")
}

/// Poll counts for the three request kinds a page makes, over a measured window.
fn window(b: &mut Browser<ServoEngine>, h: &Hits, name: &str, secs: u64) -> (usize, usize, usize) {
    let (f0, x0, c0) = (count(h, &format!("/fetch_{name}")), count(h, &format!("/xhr_{name}")), count(h, &format!("/{name}/collect")));
    pump(b, Duration::from_secs(secs));
    (count(h, &format!("/fetch_{name}")) - f0, count(h, &format!("/xhr_{name}")) - x0, count(h, &format!("/{name}/collect")) - c0)
}

#[test]
fn background_tabs_pause_network_keep_state_and_resume_in_place() {
    rustls::crypto::aws_lc_rs::default_provider().install_default().ok();
    let (port, hits) = start_server();
    let base = format!("http://127.0.0.1:{port}");

    let ctx: Rc<dyn RenderingContext> = Rc::new(SoftwareRenderingContext::new((800u32, 600u32).into()).expect("software GL context"));
    let _ = ctx.make_current();
    let servo = ServoBuilder::default().event_loop_waker(Box::new(NoopWaker)).build();

    let clock: Arc<dyn Clock> = Arc::new(SystemClock);
    let engine = ServoEngine::new(servo, ctx, clock.clone(), ServoEngineConfig::new(1.0, Box::new(|_| false), Box::new(|| {})));
    let mut settings = Settings::default();
    // Freeze timings are long; this test exercises WARM + network policy.
    settings.background_mode = BackgroundMode::SmartPause;
    let popups = Rc::new(std::cell::RefCell::new(browser_security::PopupBlocker::new(clock.clone())));
    let mut b = Browser::new(engine, settings, clock, platform_probe(), popups);

    // ---- Tab A loads and polls in the foreground.
    let a = b.open_tab(Some(Url::parse(&format!("{base}/a")).unwrap()), true);
    wait_until(&mut b, "tab A to start polling", Duration::from_secs(60), || count(&hits, "/fetch_a") >= 3 && count(&hits, "/xhr_a") >= 3);
    let a_nonce = nonce(&mut b, a);
    let fg_rate = window(&mut b, &hits, "a", 3);
    println!("A foreground over 3s            : fetch={} xhr={} analytics={}", fg_rate.0, fg_rate.1, fg_rate.2);
    assert!(fg_rate.0 >= 3, "foreground polling should be running: {fg_rate:?}");
    assert_eq!(count(&hits, "/a"), 1);

    // ---- Switch to tab B on the SAME origin. A becomes BACKGROUND_WARM.
    let bt = b.open_tab(Some(Url::parse(&format!("{base}/b")).unwrap()), true);
    assert_eq!(b.state_of(a), Some(TabState::BackgroundWarm));
    wait_until(&mut b, "tab B to start polling", Duration::from_secs(60), || count(&hits, "/fetch_b") >= 3);
    pump(&mut b, Duration::from_millis(1500)); // let in-flight requests drain
    let b_before = count(&hits, "/fetch_b");
    let a_bg = window(&mut b, &hits, "a", 4);
    let b_after = count(&hits, "/fetch_b");
    println!("A background (Smart Pause) 4s   : fetch={} xhr={} analytics={}  | held now: {}", a_bg.0, a_bg.1, a_bg.2, b.engine.held_requests(a));
    println!("B foreground meanwhile (4s)     : fetch={}", b_after - b_before);
    assert_eq!(a_bg, (0, 0, 0), "a Smart-Paused background tab must send no polling/XHR/analytics requests");
    assert!(b.engine.held_requests(a) > 0, "requests are held, not lost");
    assert!(b_after - b_before >= 4, "the active tab on the same origin is unaffected (got {})", b_after - b_before);

    // ---- Return to A: resumes in place. No document reload.
    b.select_tab(a);
    assert_eq!(b.state_of(a), Some(TabState::Foreground));
    let resumed = window(&mut b, &hits, "a", 3);
    println!("A after returning, 3s           : fetch={} xhr={}", resumed.0, resumed.1);
    assert!(resumed.0 >= 2 && resumed.1 >= 2, "polling resumes: {resumed:?}");
    assert_eq!(count(&hits, "/a"), 1, "the HTML document was NOT requested again");
    assert_eq!(nonce(&mut b, a), a_nonce, "same JS context: page was not reloaded");
    assert_eq!(b.state_of(bt), Some(TabState::BackgroundWarm));

    // ---- Comparison: the same background tab with engine throttling only (site = Always active).
    b.settings.sites.insert("127.0.0.1".into(), SiteSettings { mode: Some(SiteMode::AlwaysActive), refresh_on_return: false });
    b.set_background_mode(BackgroundMode::SmartPause); // reinstall policy with new site rules
    b.select_tab(bt); // A goes background under "Normal"
    pump(&mut b, Duration::from_millis(1500));
    let normal_bg = window(&mut b, &hits, "a", 4);
    println!("A background (Normal/throttle)  : fetch={} xhr={} analytics={}  (engine throttling only)", normal_bg.0, normal_bg.1, normal_bg.2);

    // ---- Memory attribution sanity.
    pump(&mut b, Duration::from_secs(6));
    let m = b.engine.servo_memory();
    println!("servo memory report: processes={} explicit={} (shared {}) resident={} documents={}", m.processes, ByteSize(m.explicit_total), ByteSize(m.shared), ByteSize(m.resident), m.by_url.len());
    for (path, bytes) in &m.largest_shared {
        println!("  shared: {path} -> {}", ByteSize(*bytes));
    }
    for (u, bytes) in &m.by_url {
        println!("  {u} -> {}", ByteSize(*bytes));
    }
}
