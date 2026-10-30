# Memory management

- Budget: 45% of RAM (about 3.6 GB on 8 GB), capped by `own usage + available - 12% reserve`, floor 384 MiB; per-tab soft = 10% of global (min 96 MiB), hard = 2x soft; cache/GPU budgets derived and clamped. Overrides win.
- Probes: Linux `/proc/meminfo`, `smaps_rollup` (PSS), PSI. macOS `sysctl` + `proc_pid_rusage` `ri_phys_footprint`, `kern.memorystatus_vm_pressure_level`: **type-checked for aarch64-apple-darwin only, never run**.
- Governor rules (unit tested): foreground never targeted; audible/loading/dirty-form/pinned protected from auto-discard; discards rate-limited and stop when the projection fits; time alone never discards.
- Effective on Servo today: freeze = throttle + stricter network + HTTP cache clear on trim; discard = real WebView teardown.
- Not effective yet: per-tab image release, GPU budget enforcement.
## Measured (real Servo, debug build, software GL, one sandbox VM: relative only)
6 heavy tabs (250k-object JS heap each): idle 157 MiB -> 359 MiB (~34 MiB/tab) unique. Discarding 4 tabs released 83 MiB after jemalloc's 3 s decay (275 MiB). Closing 5 tabs: 255 MiB. jemalloc itself held 39 MiB allocated / 50 MiB resident at the end; the remainder is the JS runtime, software GL and the binary, not controlled by the allocator. Discard + restore reloaded the page once and restored scroll (1234).

- **No cross-browser benchmarks exist.** There is no measured claim of being more efficient than any browser. RAM/CPU/tab-switch comparisons against Chrome/Firefox/Safari are not done.
