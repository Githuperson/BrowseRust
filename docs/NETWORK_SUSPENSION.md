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

## Streaming (real Servo, measured)
| | before stream gate | with stream gate |
|---|---|---|
| page JS receives SSE/WS in background (5 s) | +29 / +29 | **+0 / +0** (58 queued, cap 256) |
| after return | live | backlog replayed in order, then live; SSE not reconnected |
| stream faster than the cap | unbounded in page | queue stays at 256, socket closed, page gets `error` and reconnects |

Frozen state (real Servo): idle background tabs freeze on the timer, hold requests, and return with the same JS context and one document request.
Keep-alive: 0 requests in the background; after return 17-18 requests used 1-2 new connections, so reuse is **mostly** but not fully preserved. Not tuned.

## Not done / limits
- The server keeps sending: the gate hides data from the page, it does not stop TCP traffic. No zero-incoming-data guarantee is possible.
- Workers / service workers: not gated, untested.
- "Refresh content when returning": implemented as `reload()`, off by default; untested.
