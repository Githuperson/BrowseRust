#![allow(dead_code)]
//! Shared harness for real-Servo integration tests. Each test binary runs one
//! scenario because Servo can be initialised once per process.

use browser_app::Browser;
use browser_core::{Clock, SystemClock, TabId};
use browser_memory::probe::platform_probe;
use browser_settings::Settings;
use servo::{EventLoopWaker, RenderingContext, ServoBuilder, SoftwareRenderingContext};
use servo_integration::{ServoEngine, ServoEngineConfig};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub type Hits = Arc<Mutex<HashMap<String, usize>>>;
/// path -> (content-type, body). `None` = 404-ish "ok".
pub type Router = Arc<dyn Fn(&str) -> Option<(&'static str, String)> + Send + Sync>;

pub fn start_server(router: Router) -> (u16, Hits) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let hits: Hits = Arc::default();
    let h = hits.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut s) = stream else { continue };
            let (h, router) = (h.clone(), router.clone());
            std::thread::spawn(move || {
                let mut buf = [0u8; 4096];
                let n = s.read(&mut buf).unwrap_or(0);
                let req = String::from_utf8_lossy(&buf[..n]).to_string();
                let path = req.split_whitespace().nth(1).unwrap_or("/").split('?').next().unwrap_or("/").to_string();
                *h.lock().unwrap().entry(path.clone()).or_default() += 1;
                let (ctype, body) = router(&path).unwrap_or(("text/plain", "ok".into()));
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

pub fn new_browser(settings: Settings) -> Browser<ServoEngine> {
    rustls::crypto::aws_lc_rs::default_provider().install_default().ok();
    let ctx: Rc<dyn RenderingContext> = Rc::new(SoftwareRenderingContext::new((800u32, 600u32).into()).expect("software GL"));
    let _ = ctx.make_current();
    let servo = ServoBuilder::default().event_loop_waker(Box::new(NoopWaker)).build();
    let clock: Arc<dyn Clock> = Arc::new(SystemClock);
    let engine = ServoEngine::new(servo, ctx, clock.clone(), ServoEngineConfig::new(1.0, Box::new(|_| false), Box::new(|| {})));
    Browser::new(engine, settings, clock, platform_probe())
}

pub fn count(h: &Hits, path: &str) -> usize {
    h.lock().unwrap().get(path).copied().unwrap_or(0)
}

pub fn pump(b: &mut Browser<ServoEngine>, d: Duration) {
    let end = Instant::now() + d;
    while Instant::now() < end {
        b.engine.spin();
        b.tick();
        std::thread::sleep(Duration::from_millis(10));
    }
}

pub fn wait_until(b: &mut Browser<ServoEngine>, what: &str, timeout: Duration, mut cond: impl FnMut() -> bool) {
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

pub fn eval_string(b: &mut Browser<ServoEngine>, tab: TabId, js: &str) -> String {
    let out: Rc<std::cell::RefCell<Option<String>>> = Rc::default();
    let o = out.clone();
    b.engine.webview(tab).expect("webview").evaluate_javascript(js, move |r| {
        *o.borrow_mut() = Some(match r {
            Ok(servo::JSValue::String(s)) => s,
            Ok(v) => format!("{v:?}"),
            Err(e) => format!("ERR {e:?}"),
        });
    });
    let end = Instant::now() + Duration::from_secs(15);
    while out.borrow().is_none() && Instant::now() < end {
        b.engine.spin();
        std::thread::sleep(Duration::from_millis(10));
    }
    let r = out.borrow().clone();
    r.expect("js evaluation timed out")
}

pub fn uss_mib(b: &Browser<ServoEngine>) -> f64 {
    let _ = b;
    let p = platform_probe().process().expect("process memory");
    p.best_unique().0 as f64 / (1024.0 * 1024.0)
}
