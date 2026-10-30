# Security

## Implemented and verified
- **Navigation policy** (unit tested): `javascript:` never; page-initiated `file:`/`data:` blocked; only the exact URL the browser asked for is allowed through `request_navigation`.
- **Permissions** (verified end to end with real Servo): requests with no stored decision are held and shown in a permission bar (Allow/Block, Remember). Decisions persist per exact origin in `permissions.json`; a remembered Allow resolved the page's promise on the next run without prompting. Private mode keeps decisions in memory only. Opaque origins are denied.
  - Servo 0.7.0 keeps geolocation, WebRTC/getUserMedia, async clipboard and Bluetooth **disabled**, so pages cannot ask for them. Only notifications and the Permissions API are enabled (`browser_preferences()`).
- **Popups**: blocked unless a real user gesture (click/key forwarded by the shell) occurred within 1.5 s.
- **Private browsing** (`--private`; verified): localStorage and cookies written in one private run were absent in the next, while a normal profile kept them. History, bookmarks, permissions and session are in-memory/not written. Implemented with Servo's `temporary_storage` plus in-memory stores.
- **Process isolation**: multiprocess is the default (`--single-process` opts out). Verified: tabs of a site run in a separate content process; tab switching and lifecycle still work.
- **Downloads**: filename sanitising and dangerous-extension flagging exist (tested) but **Servo 0.7.0 has no download API**, so there is no downloads manager and nothing is saved to disk.
- **Passwords**: stored only in the OS credential store via `keyring` (untested here: no keychain in the sandbox).
- Favicons are size-capped (<= 256x256) and deduplicated.

## Relied on from Servo, not independently verified
Same-origin policy, CORS, CSP, TLS validation, cookie flags.

## NOT available / NOT working
- **OS sandbox**: `--sandbox` exists, but (a) in Servo 0.7.0 the gaol sandbox is compiled only for non-ARM targets, so on **Apple Silicon content processes are unsandboxed**; (b) on this Linux container the sandboxed content process produced a blank, endlessly loading page and a hung exit. It must not be considered working.
- Mixed-content rules (`mixed.rs`) are not wired into the request path.
- No certificate-error UI; Servo's default behaviour applies (`ignore_certificate_errors` is left false).
