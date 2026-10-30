# Third-party software

BrowseRust links many open-source crates; this lists the main ones. Run `cargo tree` or a license tool (e.g. `cargo about`) against `Cargo.lock` for the complete, exact list before redistributing binaries.

| Component | License | Notes |
|---|---|---|
| [Servo](https://github.com/servo/servo) 0.7.0 and its crates (SpiderMonkey via `mozjs`, WebRender, Stylo, ...) | MPL-2.0 (SpiderMonkey MPL-2.0) | Used unmodified from crates.io. |
| egui / egui_glow / egui-winit / winit | MIT or Apache-2.0 | |
| `epaint` 0.37.0 | MIT or Apache-2.0 | **Vendored with a 2-line patch** in `vendor/epaint` (see `vendor/epaint/BROWSERUST_PATCH.md`). |
| jemalloc (`tikv-jemallocator`) | MIT / Apache-2.0 (jemalloc BSD-2-Clause) | |
| SQLite (bundled via `rusqlite`) | Public domain | |
| `ureq`, `rustls`, `image`, `keyring`, `parking_lot`, `serde`, ... | MIT and/or Apache-2.0 | |

BrowseRust's own code is MIT (see `LICENSE`).
