# Security

Implemented and unit tested (browser-side): navigation scheme policy (javascript:/page-initiated file:/data: blocked), mixed-content rules, URL-credential spoof indicator, per-origin permissions (unknown = denied), popup blocking by user gesture, download filename sanitising and dangerous-extension flag. Permission requests from Servo are denied by default; there is no prompt UI yet.

Relied on from Servo, not independently verified: same-origin policy, CORS, CSP, TLS validation, cookie flags.

Not done: process isolation / sandbox enabling (Servo's `multiprocess` is compiled in, not enabled), private browsing, password storage, `mixed.rs` is not wired into the request path.
