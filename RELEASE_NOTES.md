# BrowseRust 0.1.0-beta.1

**This is a beta.** It runs and the core ideas are tested, but it is rough, has limited site compatibility (it is built on Servo), and is **not sandboxed on Apple Silicon**. Please read the known limitations before use.

## Highlights
- Background tabs stay alive without behaving active: no reload on return, per-tab network suppression, a bounded WebSocket/SSE gate, and a memory governor that freezes then (only under pressure) discards tabs.
- Native UI with Chrome-style tabs, Arc-style sidebar, command palette (Ctrl+K), downloads, find in page, zoom, permission prompts, private mode, session restore.
- Custom themes from CSS-style files, including BetterDiscord-style themes (remote images/imports only after you approve them).

## Install
Download the archive for your platform, unpack, run `./browserust`. **Binaries are unsigned**: on macOS you may need `xattr -d com.apple.quarantine browserust`. Or build from source (see README).

## Not verified in this release
- macOS and Windows were not tested (macOS Apple Silicon is the main target but unverified).
- Linux x86_64 was tested under a virtual display with software rendering only; no real GPU.
- No memory benchmark against Chrome, Firefox or Safari exists; do not assume it uses less memory.

Full details: README.md and CHANGELOG.md.
