# Local patch to epaint 0.37.0

Adds `ctx.flush()` before `ctx.render(..)` in `src/text/font_face.rs` (two sites).

Why: vello_cpu documents that multi-threaded rendering must call `flush()` before
rasterizing (a no-op in single-threaded mode). Servo's `servo-canvas` enables vello_cpu's
default (multithreading) features, and cargo feature unification applies them to epaint
too, so on any multi-core machine egui text rendering panics with
"attempted to rasterize before flushing". Remove this patch once epaint fixes it upstream.
