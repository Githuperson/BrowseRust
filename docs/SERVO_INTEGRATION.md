# Servo integration (servo 0.7.0, crates.io, published 2026-10-05)

Used as an unmodified dependency; no fork. Everything below was read in the 0.7.0 sources.

| Need | Servo API | Verified behaviour |
|---|---|---|
| Background tab | `WebView::hide()` / `show()` | `hide` -> `SetWebViewHidden` -> pipelines `SetThrottled(true)`: timers slowed, rAF/animation skipped, painting stopped. WebView, DOM and JS heap stay alive. |
| Per-tab request control | `WebViewDelegate::load_web_resource` | Called per WebView. Holding the `WebResourceLoad` leaves the fetch pending (async wait, no timeout); dropping it resumes (`DoNotIntercept`); `intercept(..).cancel()` fails it. Each fetch has its own `FetchContext`, so a held request does not block other tabs. |
| Discard | drop `WebView` | `CloseWebView` + painter removal. |
| Memory | `Servo::create_memory_report` | Per **process**, not per WebView. Only `Explicit*` kinds are additive; `vsize`/`resident` are global. Tabs are attributed by `url(...)` path. |
| Caches | `network_manager().clear_cache()` | Used for Moderate/Aggressive trim. |
| Page state | `WebView::evaluate_javascript` | Scroll position and form-dirty flag. |
| Tunables not yet used | prefs `js_mem_*`, `network_http_cache_size`, `session_history_max_length` | Not changed from defaults: no measurement justifies it yet. |

## Stream gate (`stream_gate.js`, injected with `UserContentManager` + `UserScript`)
Wraps `WebSocket`/`EventSource` at document start. While a tab is paused, `message` events are intercepted before page listeners and queued (hard caps: 256 messages, 256 KiB). On resume they are replayed in order. On overflow the socket is closed and the page receives an `error` on resume so its own reconnect logic runs. Measured on real Servo: +0 messages reach the page in the background (vs +29 before), backlog replayed on return, overflow capped at 256 and recovered.
Limits: it hides delivery from page JS, it does **not** stop bytes arriving on the socket; main-thread sockets only (not in Workers); `class X extends WebSocket` loses its subclass prototype; a page can call `__bpPause` on itself.

## Allocator
`servo-allocator` with `use-jemalloc` is required (system allocator crashes Servo's reporter). `allocator.rs` sets `background_thread:true,dirty_decay_ms:3000,muzzy_decay_ms:3000` through the `_rjem_malloc_conf` symbol and exposes `opt_dirty_decay_ms()` so tests confirm it is live. jemalloc on macOS/aarch64 is **untested**.

## Limits (not hidden)
- `document.visibilityState` stays "visible" in Servo (upstream TODO), so pages get no `visibilitychange`.
- No per-WebView image-cache release (`release_tab_images = false`).
- No embedder hook to pause WebSocket/SSE delivery. Worked around at the JS layer (see below). WebSocket handshakes **and** SSE requests do reach `load_web_resource` (measured), so new/reconnecting streams are held like any request.
- Script is throttled, not suspended (`suspend_script = false`).
- No embedder stop-loading API; `stop` runs `window.stop()`.
- Discard restores only the current URL and scroll; Servo has no API to inject session history.
- Embedding needs `servo-allocator` with `use-jemalloc`: with the system allocator the memory reporter hit UB in `malloc_usable_size` (arithmetic overflow in servo-script-bindings/mem.rs) and aborted.
