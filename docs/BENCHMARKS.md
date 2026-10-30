# Engine comparison: Servo 0.7.0 vs WebKitGTK (via `wry`)

Question: would a different engine be faster/lighter than Servo while offering downloads?

## Method (bench/run.sh, bench/server.py)
Same local server and page for both engines: each "heavy" page builds a 250,000-object JS array, then beacons `/loaded`. N tabs opened from the command line under Xvfb with software GL; wait for all beacons, settle 8 s, then sum **PSS over the engine's whole process tree** (browser + web/content + network processes). One run each, Linux x86_64, 4 vCPU VM, release builds (Servo with LTO off).

| Engine | blank tab | 6 heavy tabs | per extra heavy tab | processes (6 tabs) | pages built after |
|---|---|---|---|---|---|
| Servo 0.7.0 shell (full browser UI, multiprocess) | 203 MiB | 401 MiB | ~33 MiB | 7 | 0.2 s |
| WebKitGTK 2.52 via wry (bare windows, no UI) | 159 MiB | 487 MiB | ~55 MiB | 8 | 0.2 s |

## Reading it honestly
- On this workload Servo was **lighter per tab** and **no slower** (JS build time is equal; this does not measure rendering or real-site loading).
- WebKit's lower blank-tab figure is *not* apples to apples: the Servo number includes our whole egui UI, a GL context and jemalloc; the WebKit spike has no browser UI at all.
- Single run, synthetic page, Linux. **WKWebView on macOS (the case that matters for the M1) was not measured** and shares system frameworks, so it may behave very differently.
- Page-load speed on real sites, CPU, and rendering speed were not compared.

## Cost of switching to a system WebView (from API review, not measured)
Gains: downloads (wry has download-started/completed handlers: compiles, **not run**), a far more mature web platform.
Losses: per-request interception for http(s) (the basis of the network suspension feature), per-tab memory attribution (processes are owned by the system), control over visibility/throttling, and the engine becomes Apple's/GNOME's, not ours. It also contradicts the original "no WebKit/Chromium" requirement.

## Conclusion
The data does not justify replacing Servo for speed or weight. The real gap is downloads; that is better solved inside this architecture (see ROADMAP) than by swapping engines.
