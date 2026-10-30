# Architecture

Rust workspace. Servo is the only rendering engine and is reachable only through `servo-integration`.

| Crate | Role | Status |
|---|---|---|
| `browser-core` | ids, injectable clock, atomic file writes, `TabState` | done, tested |
| `browser-settings` | background modes, per-site exceptions, timings, memory overrides | done, tested |
| `browser-memory` | adaptive budgets, ledger, OS probes (Linux, macOS), pressure hysteresis, bounded caches, `MemoryGovernor` | done, tested |
| `browser-tabs` | tab strip (pin, group, workspace, undo-close) and 4-state `TabLifecycleManager` | done, tested |
| `browser-network` | request classifier, policy table, per-tab bounded hold queue, `NetworkHub`, stream buffers, monitor | done, tested |
| `browser-security` | navigation policy, mixed content, permissions, popup gesture gate, download safety | done, tested |
| `browser-session` | discard snapshots, session file, crash marker | done, tested |
| `browser-engine` | `Engine` trait + `MockEngine` | done |
| `browser-app` | `Browser` orchestrator, omnibox resolver | done, tested with `MockEngine` |
| `servo-integration` | `ServoEngine` over Servo 0.7.0 | done; one end-to-end test against real Servo |
| `browser-storage` | history + bookmarks on SQLite (frecency suggestions, Netscape-HTML import/export, in-memory for private mode); `SecretStore` trait with OS-keyring impl | storage tested; **keyring backend compiles but is untested (no keychain here)** |
| UI / shell (egui + winit) | tab strip, toolbar, content blit | **not built yet** |
| extensions, downloads manager, devtools, password manager UI/autofill, PDF | | **not built** (see ROADMAP) |

Data flow: the shell calls `Browser`; `TabLifecycleManager` returns `LifecycleCommand`s; `Browser::execute` turns them into `Engine` calls. `MemoryGovernor` is a pure function from a snapshot to actions, applied through the same state machine, which vetoes anything that would hit the foreground tab.
