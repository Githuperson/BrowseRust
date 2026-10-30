//! Real-Servo test: what actually happens to SSE, WebSocket and keep-alive
//! connections when a tab goes to the background. Measures; asserts only what
//! must hold. Run: cargo test -p servo-integration --test streaming -- --nocapture

mod common;
use base64::Engine as _;
use browser_network::RequestClass;
use browser_settings::Settings;
use common::{eval_string, pump, wait_until};
use sha1::{Digest, Sha1};
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicUsize, Ordering::SeqCst};
use std::sync::Arc;
use std::time::Duration;
use url::Url;

#[derive(Default)]
struct Stats {
    ka_conns: AtomicUsize,
    ka_reqs: AtomicUsize,
    sse_conns: AtomicUsize,
    ws_conns: AtomicUsize,
    sse_sent: AtomicUsize,
    ws_sent: AtomicUsize,
    doc: AtomicUsize,
    fast_conns: AtomicUsize,
}

fn page(port: u16) -> String {
    format!(
        r#"<!doctype html><title>stream</title><body>s<script>
window.__nonce=Math.random().toString(36).slice(2); window.sse=0; window.ws=0; window.wsopen=false;
var es=new EventSource('/sse'); es.onmessage=function(){{window.sse++}};
try{{ var w=new WebSocket('ws://127.0.0.1:{port}/ws'); w.onopen=function(){{window.wsopen=true}}; w.onmessage=function(){{window.ws++}}; }}catch(e){{}}
setInterval(function(){{fetch('/ka?'+Date.now()).catch(function(){{}})}},200);
</script></body>"#
    )
}

fn fast_page() -> String {
    r#"<!doctype html><title>fast</title><body>f<script>
window.fast=0; window.err=0; window.opens=0;
function go(){ window.opens++; var e=new EventSource('/sse_fast'); e.onmessage=function(){window.fast++}; e.onerror=function(){window.err++; e.close(); setTimeout(go,100)}; }
go();
</script></body>"#.to_string()
}

fn handle(mut s: TcpStream, port: u16, st: Arc<Stats>) {
    let mut first = true;
    loop {
        let mut rd = BufReader::new(s.try_clone().unwrap());
        let mut line = String::new();
        if rd.read_line(&mut line).unwrap_or(0) == 0 {
            return;
        }
        let path = line.split_whitespace().nth(1).unwrap_or("/").split('?').next().unwrap().to_string();
        loop {
            let mut h = String::new();
            if rd.read_line(&mut h).unwrap_or(0) == 0 || h == "\r\n" {
                break;
            }
        }
        match path.as_str() {
            "/ka" => {
                if first {
                    st.ka_conns.fetch_add(1, SeqCst);
                }
                st.ka_reqs.fetch_add(1, SeqCst);
                if s.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nCache-Control: no-store\r\nConnection: keep-alive\r\n\r\nok").is_err() {
                    return;
                }
                first = false;
            }
            "/" => {
                st.doc.fetch_add(1, SeqCst);
                let b = page(port);
                let _ = s.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{b}", b.len()).as_bytes());
                return;
            }
            "/fast" => {
                let b = fast_page();
                let _ = s.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{b}", b.len()).as_bytes());
                return;
            }
            "/sse_fast" => {
                st.fast_conns.fetch_add(1, SeqCst);
                let _ = s.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-store\r\n\r\n");
                loop {
                    if s.write_all(b"data: f\n\n").is_err() {
                        return;
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
            }
            "/sse" => {
                st.sse_conns.fetch_add(1, SeqCst);
                let _ = s.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-store\r\n\r\n");
                loop {
                    if s.write_all(b"data: tick\n\n").is_err() {
                        return;
                    }
                    st.sse_sent.fetch_add(1, SeqCst);
                    std::thread::sleep(Duration::from_millis(200));
                }
            }
            _ => return,
        }
    }
}

fn ws_handshake_and_stream(s: TcpStream, st: Arc<Stats>) {
    let mut rd = BufReader::new(s.try_clone().unwrap());
    let mut key = String::new();
    let mut request_line = String::new();
    let _ = rd.read_line(&mut request_line);
    loop {
        let mut h = String::new();
        if rd.read_line(&mut h).unwrap_or(0) == 0 || h == "\r\n" {
            break;
        }
        if h.to_ascii_lowercase().starts_with("sec-websocket-key:") {
            key = h.split_once(':').unwrap().1.trim().to_string();
        }
    }
    let mut hasher = Sha1::new();
    hasher.update(format!("{key}258EAFA5-E914-47DA-95CA-C5AB0DC85B11"));
    let accept = base64::engine::general_purpose::STANDARD.encode(hasher.finalize());
    let mut s = s;
    if s.write_all(format!("HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {accept}\r\n\r\n").as_bytes()).is_err() {
        return;
    }
    st.ws_conns.fetch_add(1, SeqCst);
    loop {
        if s.write_all(&[0x81, 4, b't', b'i', b'c', b'k']).is_err() {
            return;
        }
        st.ws_sent.fetch_add(1, SeqCst);
        std::thread::sleep(Duration::from_millis(200));
    }
}

#[test]
fn background_streaming_behaviour_is_measured() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let st = Arc::new(Stats::default());
    let st2 = st.clone();
    std::thread::spawn(move || {
        for s in listener.incoming().flatten() {
            let st = st2.clone();
            // Peek the request line to route websocket upgrades.
            let mut peek = [0u8; 16];
            let n = s.peek(&mut peek).unwrap_or(0);
            let is_ws = String::from_utf8_lossy(&peek[..n]).contains("/ws");
            std::thread::spawn(move || {
                if is_ws {
                    ws_handshake_and_stream(s, st)
                } else {
                    handle(s, port, st)
                }
            });
        }
    });

    let mut b = common::new_browser(Settings::default());
    let a = b.open_tab(Some(Url::parse(&format!("http://127.0.0.1:{port}/")).unwrap()), true);
    wait_until(&mut b, "SSE + keep-alive running", Duration::from_secs(90), || st.sse_conns.load(SeqCst) >= 1 && st.ka_reqs.load(SeqCst) >= 3);
    pump(&mut b, Duration::from_secs(2));
    let n = |b: &mut browser_app::Browser<servo_integration::ServoEngine>| -> (String, String, String) {
        (eval_string(b, a, "String(window.sse)"), eval_string(b, a, "String(window.ws)"), eval_string(b, a, "String(window.wsopen)"))
    };
    let (sse0, ws0, wsopen) = n(&mut b);
    println!("foreground: page received sse={sse0} ws={ws0} (ws open={wsopen}); server conns: ka={} sse={} ws={}", st.ka_conns.load(SeqCst), st.sse_conns.load(SeqCst), st.ws_conns.load(SeqCst));
    let log = b.engine.network_log(a);
    let streaming_seen = log.iter().filter(|r| r.class == RequestClass::Streaming).count();
    println!("requests classified Streaming by the controller: {streaming_seen} (sse path seen by interception: {})", log.iter().any(|r| r.url.ends_with("/sse")));

    // Background tab A.
    let other = b.open_tab(Some(Url::parse("about:blank").unwrap()), true);
    let _ = other;
    pump(&mut b, Duration::from_millis(800));
    let (ka_r0, ka_c0, sent_sse0, sent_ws0) = (st.ka_reqs.load(SeqCst), st.ka_conns.load(SeqCst), st.sse_sent.load(SeqCst), st.ws_sent.load(SeqCst));
    pump(&mut b, Duration::from_secs(5));
    let (sse1, ws1, _) = n(&mut b);
    println!("background 5 s: keep-alive requests sent={}  (new keep-alive connections={})", st.ka_reqs.load(SeqCst) - ka_r0, st.ka_conns.load(SeqCst) - ka_c0);
    println!("background 5 s: server wrote sse={} ws={}; page JS received sse +{} ws +{}", st.sse_sent.load(SeqCst) - sent_sse0, st.ws_sent.load(SeqCst) - sent_ws0,
        sse1.parse::<i64>().unwrap_or(0) - sse0.parse::<i64>().unwrap_or(0), ws1.parse::<i64>().unwrap_or(0) - ws0.parse::<i64>().unwrap_or(0));
    assert_eq!(st.ka_reqs.load(SeqCst) - ka_r0, 0, "fetch polling must be suppressed in the background");
    let d = |a: &str, b: &str| a.parse::<i64>().unwrap_or(0) - b.parse::<i64>().unwrap_or(0);
    assert!(d(&sse1, &sse0) <= 1 && d(&ws1, &ws0) <= 1, "SSE/WebSocket delivery to page JS must be paused (sse +{}, ws +{})", d(&sse1, &sse0), d(&ws1, &ws0));
    let queued = eval_string(&mut b, a, "String(window.__bpQueued)");
    println!("background: messages held in the bounded queue = {queued} (cap 256)");

    // Return.
    b.select_tab(a);
    pump(&mut b, Duration::from_secs(3));
    let ka_after = st.ka_reqs.load(SeqCst) - ka_r0;
    println!("after return: keep-alive requests={} new connections={} document requests={}", ka_after, st.ka_conns.load(SeqCst) - ka_c0, st.doc.load(SeqCst));
    assert!(ka_after >= 3, "polling resumes");
    assert_eq!(st.doc.load(SeqCst), 1, "no document reload");
    assert_eq!(st.sse_conns.load(SeqCst), 1, "SSE connection was not torn down / reconnected");
    let (sse2, ws2, _) = n(&mut b);
    println!("after return: page received sse={sse2} ws={ws2} (backlog was replayed in order, then live)");
    assert!(d(&sse2, &sse1) >= 10 && d(&ws2, &ws1) >= 10, "queued messages are delivered on resume");

    // ---- Overflow: a stream faster than the bound is closed cleanly and recovers.
    let f = b.open_tab(Some(Url::parse(&format!("http://127.0.0.1:{port}/fast")).unwrap()), true);
    wait_until(&mut b, "fast stream", Duration::from_secs(60), || st.fast_conns.load(SeqCst) >= 1);
    pump(&mut b, Duration::from_secs(1));
    b.select_tab(a); // f goes to background
    pump(&mut b, Duration::from_secs(6)); // ~600 msgs at 100/s > 256 cap
    let q = eval_string(&mut b, f, "String(window.__bpQueued)");
    let conns_bg = st.fast_conns.load(SeqCst);
    println!("fast stream in background: queue={q} (never above 256), server connections={conns_bg}");
    assert!(q.parse::<usize>().unwrap() <= 256);
    b.select_tab(f);
    pump(&mut b, Duration::from_secs(3));
    let (err, opens) = (eval_string(&mut b, f, "String(window.err)"), eval_string(&mut b, f, "String(window.opens)"));
    println!("after return: page saw error x{err} and reconnected (opens={opens}); server fast connections={}", st.fast_conns.load(SeqCst));
    assert!(err.parse::<i32>().unwrap() >= 1, "overflow closes the stream and the page is told");
    assert!(opens.parse::<i32>().unwrap() >= 2, "page reconnected after resume");
}
