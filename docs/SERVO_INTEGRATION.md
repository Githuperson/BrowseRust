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

## Limits (not hidden)
- `document.visibilityState` stays "visible" in Servo (upstream TODO), so pages get no `visibilitychange`.
- No per-WebView image-cache release (`release_tab_images = false`).
- No embedder hook to pause WebSocket/SSE delivery (`pause_stream_delivery = false`). Whether WebSocket handshakes reach `load_web_resource` is **untested**.
- Script is throttled, not suspended (`suspend_script = false`).
- No embedder stop-loading API; `stop` runs `window.stop()`.
- Discard restores only the current URL and scroll; Servo has no API to inject session history.
- Embedding needs `servo-allocator` with `use-jemalloc`: with the system allocator the memory reporter hit UB in `malloc_usable_size` (arithmetic overflow in servo-script-bindings/mem.rs) and aborted.
