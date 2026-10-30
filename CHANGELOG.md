# Changelog

## 0.1.0-beta.1 - 2026-10-30

First public beta.

### Added
- Servo 0.7.0 integration behind an engine-agnostic `Engine` trait; browser shell (egui + winit) with Chrome-style tabs, Arc-style sidebar, command palette, find, zoom, downloads popover, permission bar, new-tab page.
- Tab lifecycle (Active / Paused / Frozen / Discarded), per-tab network activity control, WebSocket/SSE delivery gate, memory governor with adaptive budgets and pressure handling, jemalloc tuned to return memory to the OS.
- Background modes (Normal, Smart Pause, Deep Sleep, Strict Offline) with per-site exceptions and opt-in refresh-on-return.
- Shell-level downloads (Servo has no download API): header probe with cookie-safe redirects, streaming to `.part`, cancel, no overwrite.
- History/bookmarks (SQLite), session restore with crash detection, private mode, popup blocking, persistent permissions, favicons, tab groups, workspaces.
- Custom browser themes from CSS-style files (BetterDiscord-compatible variables), with consent-gated download of imported stylesheets and background/home images (animated GIF supported), live reload.
- Multiprocess content by default.

### Known issues
See the README. Notably: no OS sandbox on Apple Silicon, macOS untested, limited site compatibility, no PDF viewer/devtools/extensions.

### Verification
Unit and integration tests pass on Linux x86_64 (including end-to-end tests against the real engine). Behaviour was also checked by driving the running window under a virtual display. Nothing has been run on macOS or Windows.
