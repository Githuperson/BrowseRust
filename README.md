# BrowseRust

A desktop web browser written in Rust, rendering with [Servo](https://servo.org) (0.7.0, unmodified), built around one idea: **tabs should stay alive without behaving as if they are being used.**

> **BETA (0.1.0-beta.1).** Expect rough edges. Read [Known limitations](#known-limitations-beta) before relying on it. It is not hardened for everyday use.

## What it does differently

Background tabs keep their page (DOM, JavaScript state, scroll, forms) but stop generating work:

- **Four tab states** - Active, Paused (warm), Frozen, Discarded - with a small coloured dot on each tab. Returning to a Paused/Frozen tab never reloads it; it resumes in place. Only *Discarded* tabs (severe memory pressure or your choice) reload.
- **Per-tab network control.** In the background, polling `fetch`/XHR, analytics and prefetch requests are held or dropped *for that tab only* (an active tab on the same site is unaffected), and resume when you return.
- **WebSocket / SSE gate.** While a tab is paused, incoming messages are held in a bounded queue (256 messages / 256 KiB) instead of reaching the page; overflow closes the stream cleanly so the page reconnects.
- **Memory governor.** Budgets adapt to available RAM; under pressure it trims caches, freezes idle tabs, and only then discards - never the foreground tab, a tab playing audio, loading, or with unsaved form input.
- **Modes:** Normal, Smart Pause (default), Deep Sleep, Strict Offline, with per-site exceptions and an opt-in "Refresh content when returning to this tab".

See [docs/NETWORK_SUSPENSION.md](docs/NETWORK_SUSPENSION.md) and [docs/MEMORY_OPTIMISATION.md](docs/MEMORY_OPTIMISATION.md) for the measured behaviour and its limits.

## Browser features

Tabs (pin, group, duplicate, drag-reorder, reopen closed), spaces, collapsible sidebar, address bar with history suggestions, bookmarks and history (SQLite), downloads manager, find in page, page zoom, command palette (Ctrl+K), permission prompts, popup blocking, private mode, session restore, native new-tab page, light/dark themes and **custom themes from CSS-style files** ([docs/THEMES.md](docs/THEMES.md)).

| Shortcut | Action | Shortcut | Action |
|---|---|---|---|
| Ctrl+T / Ctrl+W | New / close tab | Ctrl+K | Command palette |
| Ctrl+Shift+T | Reopen closed tab | Ctrl+F | Find in page |
| Ctrl+L | Focus address bar | Ctrl + / - / 0 | Zoom |
| Ctrl+R | Reload | Ctrl+B | Toggle sidebar |
| Alt+Left / Right | Back / forward | Ctrl+J | Downloads |
| Ctrl+Tab | Next tab | F11 | Full screen |

(Use Cmd instead of Ctrl on macOS.)

## Platform status

| Platform | Status |
|---|---|
| Linux x86_64 (tested on Ubuntu 24.04, X11, software GL) | Builds, runs, tested |
| macOS Apple Silicon | **Not tested.** The macOS memory probe compiles for `aarch64-apple-darwin` but has never run. |
| Windows | Not attempted |

## Build and run

Requires Rust 1.88+, a C/C++ toolchain, `clang`, `cmake`, `python3`, `pkg-config`; on Linux also fontconfig/freetype/X11/xkbcommon-x11 and EGL development packages. The first build compiles Servo and takes a long time (about 12 minutes on 4 cores without LTO) and several GB of disk.

```bash
cargo build --release -p browser-shell
./target/release/browserust --help
./target/release/browserust https://example.com
```

Options: `--profile <dir>`, `--private`, `--single-process` (debugging), `--version`. Data lives in `~/.local/share/browserust` (Linux) or `~/Library/Application Support/browserust` (macOS); downloads go to `~/Downloads`.

Tests: `cargo test --workspace` (includes end-to-end tests that run the real Servo engine against a local server; they need a software-GL capable environment).

## Known limitations (beta)

- **No OS sandbox.** Servo's sandbox is not compiled for ARM, so on Apple Silicon web content processes are **not sandboxed**; on x86_64 Linux `--sandbox` did not work in testing. Content does run in separate processes by default.
- **Servo's web compatibility is limited** compared with Chrome/Firefox/Safari. Many sites will render incorrectly or not work.
- Missing: PDF viewer, developer tools, printing, extensions, certificate-error pages, password manager UI and autofill, IME, accessibility, per-tab mute (flag only).
- **Downloads cost an extra request:** every page navigation you start reaches the server twice (a header check, then the real load). POST navigations, iframes and JavaScript-initiated downloads are not handled.
- Streaming gate hides messages from page JS; it does not stop bytes arriving on the socket, and does not cover Workers.
- Per-tab memory attribution is approximate (by document URL). **There are no cross-browser benchmarks**; no claim is made that this uses less memory than any other browser. See [docs/BENCHMARKS.md](docs/BENCHMARKS.md) for the one limited comparison that exists.
- Theme downloads (images, imported CSS) ask permission first, but the real Dark Matter assets were never fetched during testing (network blocked in the test environment).
- Passwords: the OS-keychain code compiles but was never run.

## Project layout and docs

`crates/` (engine-agnostic core: `browser-memory`, `browser-tabs`, `browser-network`, `browser-security`, `browser-storage`, `browser-downloads`, `browser-themes`; `servo-integration` is the only crate that names Servo; `browser-shell` is the egui/winit app). Start with [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md), [docs/SERVO_INTEGRATION.md](docs/SERVO_INTEGRATION.md), [docs/SECURITY.md](docs/SECURITY.md) and [docs/ROADMAP.md](docs/ROADMAP.md).

## License

MIT for BrowseRust's own code (see [LICENSE](LICENSE)). Servo is MPL-2.0; other dependencies carry their own licenses - see [THIRD_PARTY.md](THIRD_PARTY.md).
