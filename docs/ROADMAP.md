# Roadmap
1. (DONE, basic) egui+winit shell: Servo content blit via `OffscreenRenderingContext::render_to_parent_callback` (egui_glow 0.37 uses glow 0.17 / winit 0.30.13, matching Servo). Tabs, toolbar, address bar, state indicators, memory panel.
2. Wire frozen-state tests (timers, idle freeze) against real Servo; test WebSocket/SSE/workers.
3. Benchmark harness (RSS/PSS, requests/min, tab-switch latency) and comparisons.
4. Wire storage into the shell (record visits, suggestions, bookmark star), test the keyring on macOS, downloads, permission prompts, private browsing, sandbox.
5. Extensions (MV3 compatibility layer), PDF, devtools. None started.


Decision rule: use a proven dependency for security-sensitive or solved problems (SQLite, OS keyring, egui/winit for UI); write our own only for the memory, lifecycle and network-control core. `rusqlite` is pinned to 0.38 because Servo links its own `libsqlite3-sys`.

## Known shell gaps
No favicon textures (a state dot is shown instead), no new-tab page (opens about:blank), tab groups exist in the model but have no UI, no permission prompt UI (requests are denied), no downloads UI, no cursor-icon forwarding, no IME, clipboard shortcuts are not forwarded to pages, mute is a flag only (Servo exposes no per-tab mute), extensions toolbar absent. Drag-reorder is implemented but untested.

## epaint patch
`vendor/epaint` carries a 2-line fix (see its BROWSERUST_PATCH.md). Without it egui text panics on any multi-core machine because Servo's feature unification enables vello_cpu multithreading.
