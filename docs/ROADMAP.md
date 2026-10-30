# Roadmap
1. (DONE, basic) egui+winit shell: Servo content blit via `OffscreenRenderingContext::render_to_parent_callback` (egui_glow 0.37 uses glow 0.17 / winit 0.30.13, matching Servo). Tabs, toolbar, address bar, state indicators, memory panel.
2. Wire frozen-state tests (timers, idle freeze) against real Servo; test WebSocket/SSE/workers.
3. Benchmark harness (RSS/PSS, requests/min, tab-switch latency) and comparisons.
4. Wire storage into the shell (record visits, suggestions, bookmark star), test the keyring on macOS, downloads, permission prompts, private browsing, sandbox.
5. Extensions (MV3 compatibility layer), PDF, devtools. None started.


Decision rule: use a proven dependency for security-sensitive or solved problems (SQLite, OS keyring, egui/winit for UI); write our own only for the memory, lifecycle and network-control core. `rusqlite` is pinned to 0.38 because Servo links its own `libsqlite3-sys`.

## Known shell gaps
Done since first shell: favicons, native new-tab page (most visited + bookmarks), permission bar, tab-group UI (context menu + coloured bar), cursor icons, clipboard actions (Ctrl+C/X/V forwarded as editing actions; untested with a real selection), multiprocess default.
Still missing: downloads (engine has no API), certificate error pages, find-in-page, page zoom, printing, PDF viewer, devtools, IME, extensions, tab-group collapse/rename, per-tab mute (flag only), drag-reorder untested, import/export UI for bookmarks (storage supports it), password manager UI/autofill, macOS/Windows verification.

## epaint patch
`vendor/epaint` carries a 2-line fix (see its BROWSERUST_PATCH.md). Without it egui text panics on any multi-core machine because Servo's feature unification enables vello_cpu multithreading.

## Downloads (engine gap)
Servo 0.7.0 has no download API. Options, cheapest first: (1) in `load_web_resource` for user-initiated main-frame navigations, probe the response headers ourselves (Content-Disposition / non-renderable MIME) and, for a download, cancel Servo's load and stream the file with a browser-owned HTTP client, with progress UI, `sanitize_filename` and `assess_download`; (2) a small upstream Servo patch exposing a download delegate callback. See docs/BENCHMARKS.md for why switching engines was not chosen.
