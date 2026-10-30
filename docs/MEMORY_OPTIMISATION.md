# Memory management

- Budget: 45% of RAM (about 3.6 GB on 8 GB), capped by `own usage + available - 12% reserve`, floor 384 MiB; per-tab soft = 10% of global (min 96 MiB), hard = 2x soft; cache/GPU budgets derived and clamped. Overrides win.
- Probes: Linux `/proc/meminfo`, `smaps_rollup` (PSS), PSI. macOS `sysctl` + `proc_pid_rusage` `ri_phys_footprint`, `kern.memorystatus_vm_pressure_level`: **type-checked for aarch64-apple-darwin only, never run**.
- Governor rules (unit tested): foreground never targeted; audible/loading/dirty-form/pinned protected from auto-discard; discards rate-limited and stop when the projection fits; time alone never discards.
- Effective on Servo today: freeze = throttle + stricter network + HTTP cache clear on trim; discard = real WebView teardown.
- Not effective yet: per-tab image release, GPU budget enforcement.
- **No benchmarks exist.** There is no measured claim of being more efficient than any browser. RAM/CPU/tab-switch comparisons against Chrome/Firefox/Safari are not done.
