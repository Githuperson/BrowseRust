# Background network suspension

Per-tab `NetworkActivityController` (never per origin). Classes: navigation, document, subresource, api, polling, analytics, prefetch, streaming, media, download. Default mode Smart Pause: navigation/download allowed; document, subresource, api, polling, streaming, media **held** (bounded: 64 per tab, 10 min TTL, oldest idempotent evicted, POST never silently dropped); analytics and prefetch cancelled. Deep Sleep cancels polling instead of holding. A new tab may finish its first load (30 s grace). `user_initiated` is set only from the browser's own navigation record, never from page data. On foreground, held requests are released in order; stale duplicate polls are dropped.

## Measured (real Servo, software GL, local server; sandbox numbers, not benchmarks)
| Window | fetch | XHR | analytics |
|---|---|---|---|
| foreground, 3 s | 15 | 15 | 15 |
| background Smart Pause, 4 s | 0 | 0 | 0 (8 held) |
| background, engine throttling only, 4 s | 4 | 4 | 4 |
| active same-origin tab meanwhile, 4 s | 20 fetch | | |

After returning: polling resumed, HTML requested once, same JS nonce (no reload).

## Not done / limits
- Keep-alive: nothing closes pooled connections on tab switch, but this is **not tested**; test server used `Connection: close`.
- WebSocket/SSE: existing connections cannot be paused (see SERVO_INTEGRATION). `stream.rs` has the bounded queue and plan, not wired to the engine. New SSE reconnects are held if they pass `load_web_resource`. No zero-incoming-data guarantee is possible.
- Service workers and background workers: untested.
- "Refresh content when returning": implemented as `reload()`, off by default; not tested.
