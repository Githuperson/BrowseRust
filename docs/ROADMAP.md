# Roadmap
1. (DONE, basic) egui+winit shell: Servo content blit via `OffscreenRenderingContext::render_to_parent_callback` (egui_glow 0.37 uses glow 0.17 / winit 0.30.13, matching Servo). Tabs, toolbar, address bar, state indicators, memory panel.
2. Wire frozen-state tests (timers, idle freeze) against real Servo; test WebSocket/SSE/workers.
3. Benchmark harness (RSS/PSS, requests/min, tab-switch latency) and comparisons.
4. Wire storage into the shell (record visits, suggestions, bookmark star), test the keyring on macOS, downloads, permission prompts, private browsing, sandbox.
5. Extensions (MV3 compatibility layer), PDF, devtools. None started.


Decision rule: use a proven dependency for security-sensitive or solved problems (SQLite, OS keyring, egui/winit for UI); write our own only for the memory, lifecycle and network-control core. `rusqlite` is pinned to 0.38 because Servo links its own `libsqlite3-sys`.

## Shell status
Done: vector-icon UI with light/dark palettes and accent colour, Chrome-style merged tabs with favicons and lifecycle dots, pill address bar (security chip, star, zoom chip, history suggestions), Arc-style collapsible sidebar (spaces, pinned tiles, bookmarks), command palette (Ctrl+K, fuzzy), downloads popover (progress, cancel, show in folder, dangerous-file warning), find-in-page (Ctrl+F, own DOM-marking implementation because Servo lacks `window.find`), page zoom (Ctrl +/-/0), full screen (F11, untested), toasts, permission bar, new-tab page, tab groups, cursor icons, clipboard actions, session restore.
Verified by screenshots under Xvfb (Linux, software GL). Not verified: macOS look/feel, HiDPI, F11, drag-reorder, group collapse, keyboard-only navigation, accessibility (egui AccessKit not wired), IME.
Still missing: certificate error pages, printing, PDF viewer, devtools, extensions, password autofill UI, bookmark import/export UI, per-tab mute (flag only), pinned/sidebar vertical tab list.

## epaint patch
`vendor/epaint` carries a 2-line fix (see its BROWSERUST_PATCH.md). Without it egui text panics on any multi-core machine because Servo's feature unification enables vello_cpu multithreading.

## Downloads (done at shell level; see docs/SERVO_INTEGRATION.md)
Servo 0.7.0 has no download API. Options, cheapest first: (1) in `load_web_resource` for user-initiated main-frame navigations, probe the response headers ourselves (Content-Disposition / non-renderable MIME) and, for a download, cancel Servo's load and stream the file with a browser-owned HTTP client, with progress UI, `sanitize_filename` and `assess_download`; (2) a small upstream Servo patch exposing a download delegate callback. See docs/BENCHMARKS.md for why switching engines was not chosen.
