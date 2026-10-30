use browser_themes::{AssetCache, Rgb, load_remote};
use std::collections::HashMap;
use std::io::{Cursor, Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};

type Hits = Arc<Mutex<HashMap<String, usize>>>;

fn png() -> Vec<u8> {
    let img = image::RgbaImage::from_pixel(64, 32, image::Rgba([9, 99, 199, 255]));
    let mut out = Vec::new();
    img.write_to(&mut Cursor::new(&mut out), image::ImageFormat::Png).unwrap();
    out
}

fn gif() -> Vec<u8> {
    let mut out = Vec::new();
    {
        let mut enc = image::codecs::gif::GifEncoder::new(&mut out);
        for i in 0..3u8 {
            let img = image::RgbaImage::from_pixel(16, 16, image::Rgba([i * 80, 0, 0, 255]));
            enc.encode_frame(image::Frame::from_parts(img, 0, 0, image::Delay::from_numer_denom_ms(60, 1))).unwrap();
        }
    }
    out
}

fn server() -> (u16, Hits) {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    let hits: Hits = Arc::default();
    let h2 = hits.clone();
    std::thread::spawn(move || {
        for st in l.incoming().flatten() {
            let hits = h2.clone();
            std::thread::spawn(move || {
                let mut st = st;
                let mut buf = [0u8; 4096];
                let n = st.read(&mut buf).unwrap_or(0);
                let req = String::from_utf8_lossy(&buf[..n]).to_string();
                let path = req.split_whitespace().nth(1).unwrap_or("/").to_string();
                *hits.lock().unwrap().entry(path.clone()).or_default() += 1;
                let (status, extra, body): (&str, String, Vec<u8>) = match path.as_str() {
                    "/base.css" => ("200 OK", String::new(), format!(":root{{--accent: 10, 200, 100; --background-solid-darker: #050505; --home-image: url('http://127.0.0.1:{port}/smuggled.png')}} .discord-only .x{{color:red}} .y{{color:blue}}").into_bytes()),
                    "/bg.png" => ("200 OK", String::new(), png()),
                    "/hero.gif" => ("200 OK", String::new(), gif()),
                    "/evil.svg" => ("200 OK", String::new(), b"<svg xmlns='http://www.w3.org/2000/svg'><script>alert(1)</script></svg>".to_vec()),
                    "/huge.png" => ("200 OK", String::new(), vec![0x89; 9 * 1024 * 1024]),
                    "/to-private.png" => ("302 Found", "Location: http://10.0.0.1/x.png\r\n".into(), vec![]),
                    _ => ("404 Not Found", String::new(), b"nf".to_vec()),
                };
                let _ = st.write_all(format!("HTTP/1.1 {status}\r\n{extra}Content-Length: {}\r\nConnection: close\r\n\r\n", body.len()).as_bytes());
                let _ = st.write_all(&body);
            });
        }
    });
    (port, hits)
}

fn cache(name: &str, allow_loopback: bool) -> AssetCache {
    let d = std::env::temp_dir().join(format!("browserust-theme-cache-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    let mut c = AssetCache::new(d);
    c.allow_loopback = allow_loopback;
    c
}

fn theme_src(port: u16) -> String {
    format!(
        "/** @name Remote Test */\n@import url('http://127.0.0.1:{port}/base.css');\n:root {{ --background-solid: #161921; --background-image: url('http://127.0.0.1:{port}/bg.png'); --home-image: url('http://127.0.0.1:{port}/hero.gif'); }}"
    )
}

fn count(h: &Hits, p: &str) -> usize {
    h.lock().unwrap().get(p).copied().unwrap_or(0)
}

#[test]
fn nothing_is_fetched_without_permission_and_everything_works_with_it() {
    let (port, hits) = server();
    let c = cache("main", true);
    let src = theme_src(port);

    // 1. No permission: zero requests, local colours only, clear errors.
    let l = load_remote(&src, &c, false).unwrap();
    assert!(hits.lock().unwrap().is_empty(), "no network without consent: {:?}", hits.lock().unwrap());
    assert!(l.background.is_none() && l.home.is_none());
    assert_eq!(l.errors.len(), 3);
    assert_ne!(l.theme.colors.unwrap().accent, Rgb(10, 200, 100));
    assert!(l.theme.warnings.iter().any(|w| w.contains("@import not loaded yet")));

    // 2. With permission: stylesheet merged, images decoded.
    let l = load_remote(&src, &c, true).unwrap();
    assert_eq!((l.downloaded, l.from_cache), (3, 0), "{:?}", l.errors);
    let col = l.theme.colors.unwrap();
    assert_eq!(col.accent, Rgb(10, 200, 100), "accent comes from the imported stylesheet");
    assert_eq!(col.strip, Rgb(5, 5, 5), "imported --background-solid-darker");
    assert_eq!(col.bar, Rgb(0x16, 0x19, 0x21), "the theme's own variables win");
    let bg = l.background.unwrap();
    assert_eq!((bg.width, bg.height), (64, 32));
    assert_eq!(l.home.unwrap().frames.len(), 3, "animated GIF keeps its frames");
    assert!(!l.theme.warnings.iter().any(|w| w.contains("@import not loaded yet")));
    assert!(l.theme.warnings.iter().any(|w| w.contains("other rule(s) ignored")), "{:?}", l.theme.warnings);

    // 3. The imported stylesheet could not make us download another file.
    assert_eq!(count(&hits, "/smuggled.png"), 0);

    // 4. Cached now: no further requests, even with the network disabled.
    let before: usize = hits.lock().unwrap().values().sum();
    let l = load_remote(&src, &c, false).unwrap();
    assert_eq!((l.downloaded, l.from_cache), (0, 3));
    assert!(l.background.is_some() && l.home.is_some());
    assert_eq!(hits.lock().unwrap().values().sum::<usize>(), before);
}

#[test]
fn hostile_assets_are_refused() {
    let (port, _hits) = server();
    let c = cache("evil", true);
    let mk = |bg: &str| format!(":root{{--bp-bar:#101010; --background-image:url('http://127.0.0.1:{port}{bg}')}}");

    let svg = load_remote(&mk("/evil.svg"), &c, true).unwrap();
    assert!(svg.background.is_none() && svg.errors.iter().any(|e| e.contains("not a PNG")), "{:?}", svg.errors);

    let huge = load_remote(&mk("/huge.png"), &c, true).unwrap();
    assert!(huge.background.is_none() && huge.errors.iter().any(|e| e.contains("larger than")), "{:?}", huge.errors);

    let redir = load_remote(&mk("/to-private.png"), &c, true).unwrap();
    assert!(redir.background.is_none() && redir.errors.iter().any(|e| e.contains("blocked URL")), "redirect to a private IP: {:?}", redir.errors);

    let missing = load_remote(&mk("/nope.png"), &c, true).unwrap();
    assert!(missing.errors.iter().any(|e| e.contains("404")));
    // The theme's colours still work when its images fail.
    assert_eq!(missing.theme.colors.unwrap().bar, Rgb(0x10, 0x10, 0x10));
}

#[test]
fn loopback_is_refused_unless_a_test_enables_it() {
    let (port, hits) = server();
    let c = cache("strict", false);
    let l = load_remote(&format!(":root{{--bp-bar:#101010; --background-image:url('http://127.0.0.1:{port}/bg.png')}}"), &c, true).unwrap();
    assert!(l.background.is_none());
    assert!(hits.lock().unwrap().is_empty(), "a theme cannot make the browser call local services");
}
