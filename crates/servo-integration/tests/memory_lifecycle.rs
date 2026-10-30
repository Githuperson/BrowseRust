//! Real-Servo test of the allocator tuning, FROZEN state, DISCARD/RESTORE and
//! memory behaviour. Run: cargo test -p servo-integration --test memory_lifecycle -- --nocapture

mod common;
use browser_core::TabState;
use browser_settings::Settings;
use common::*;
use std::sync::Arc;
use std::time::Duration;
use url::Url;

fn heavy(id: &str) -> String {
    format!(
        r#"<!doctype html><title>heavy {id}</title><body style="height:5000px">{id}
<script>
window.__nonce = Math.random().toString(36).slice(2);
window.big = []; for (var i=0;i<250000;i++) window.big.push({{i:i,s:'row-'+i+'-{id}'}});
setInterval(function(){{ fetch('/poll_{id}?'+Date.now()).catch(function(){{}}); }}, 200);
</script></body>"#
    )
}

#[test]
fn allocator_frozen_discard_restore_and_memory() {
    // ---- allocator is configured and live
    assert_eq!(servo_integration::opt_dirty_decay_ms(), Some(3000), "malloc_conf not applied");
    assert_eq!(servo_integration::opt_background_thread(), Some(true));

    let router: Router = Arc::new(|p: &str| p.strip_prefix("/h").map(|id| ("text/html", heavy(id))));
    let (port, hits) = start_server(router);
    let base = format!("http://127.0.0.1:{port}");

    let mut s = Settings::default();
    s.timings.freeze_after = Duration::from_secs(3);
    let mut b = new_browser(s);
    let url = |i: u32| Url::parse(&format!("{base}/h{i}")).unwrap();

    pump(&mut b, Duration::from_secs(2));
    let idle = uss_mib(&b);
    println!("idle browser (no tabs yet)      : {idle:.0} MiB unique");

    // ---- open 6 heavy tabs, each becomes foreground in turn
    let mut tabs = vec![];
    for i in 0..6u32 {
        let t = b.open_tab(Some(url(i)), true);
        wait_until(&mut b, "heavy page poll", Duration::from_secs(90), || count(&hits, &format!("/poll_{i}")) >= 2);
        tabs.push(t);
    }
    // scroll tab 0 so we can verify scroll restore later
    b.select_tab(tabs[0]);
    eval_string(&mut b, tabs[0], "window.scrollTo(0, 1234); 'ok'");
    pump(&mut b, Duration::from_millis(500));
    let nonce0 = eval_string(&mut b, tabs[0], "window.__nonce");
    b.select_tab(tabs[5]);
    let six = uss_mib(&b);
    println!("6 heavy tabs open               : {six:.0} MiB unique  (~{:.0} MiB/tab)", (six - idle) / 6.0);

    // ---- FROZEN: idle background tabs freeze, with no reload
    pump(&mut b, Duration::from_secs(5));
    let frozen = tabs.iter().filter(|t| b.state_of(**t) == Some(TabState::BackgroundFrozen)).count();
    println!("tabs frozen after idle timeout  : {frozen}/5 background");
    assert_eq!(frozen, 5, "all background tabs freeze after the timeout");
    assert_eq!(b.state_of(tabs[5]), Some(TabState::Foreground));
    let held_total: usize = tabs[..5].iter().map(|t| b.engine.held_requests(*t)).sum();
    println!("requests held across frozen tabs: {held_total}");
    assert!(held_total > 0);

    // returning to a frozen tab: same JS context, no document reload
    b.select_tab(tabs[0]);
    pump(&mut b, Duration::from_secs(1));
    assert_eq!(eval_string(&mut b, tabs[0], "window.__nonce"), nonce0, "frozen tab returned without reload");
    assert_eq!(count(&hits, "/h0"), 1);
    b.select_tab(tabs[5]);
    pump(&mut b, Duration::from_millis(300));

    // ---- DISCARD 4 background tabs (user-requested) and measure the release
    b.set_pinned(tabs[1], false);
    for t in &tabs[..4] {
        b.discard_tab(*t);
    }
    assert!(tabs[..4].iter().all(|t| b.state_of(*t) == Some(TabState::Discarded)));
    pump(&mut b, Duration::from_secs(8)); // let jemalloc's background thread purge (decay 3 s)
    let discarded = uss_mib(&b);
    println!("after discarding 4 tabs (+8 s)  : {discarded:.0} MiB unique  (released {:.0} MiB)", six - discarded);

    // ---- RESTORE tab 0: the only transition that loads the page again
    b.select_tab(tabs[0]);
    wait_until(&mut b, "restored page to load", Duration::from_secs(60), || count(&hits, "/h0") >= 2);
    pump(&mut b, Duration::from_secs(1));
    let y = eval_string(&mut b, tabs[0], "String(Math.round(window.scrollY))");
    println!("restored tab scrollY            : {y} (saved 1234)");
    assert_eq!(count(&hits, "/h0"), 2, "discard+restore reloads exactly once");
    assert_ne!(eval_string(&mut b, tabs[0], "window.__nonce"), nonce0, "restore is a fresh page (documented)");

    // ---- close everything but one, memory returns
    let keep = tabs[5];
    b.select_tab(keep);
    for t in tabs.iter().filter(|t| **t != keep) {
        b.close_tab(*t);
    }
    pump(&mut b, Duration::from_secs(8));
    let after_close = uss_mib(&b);
    println!("after closing 5 tabs (+8 s)     : {after_close:.0} MiB unique");
    if let Some((alloc, res)) = servo_integration::allocator_stats() {
        println!("jemalloc: allocated {:.0} MiB, resident {:.0} MiB", alloc as f64 / 1048576.0, res as f64 / 1048576.0);
    }
    assert!(discarded < six - 20.0, "discarding 4 heavy tabs should release memory (before {six:.0}, after {discarded:.0})");
    assert!(after_close < six, "closing tabs should lower memory");
}
