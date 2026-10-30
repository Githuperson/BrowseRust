//! Browser-chrome themes from CSS-style files.
//!
//! The interface is drawn natively, so a theme is not applied as real CSS: the
//! loader reads **custom properties** from `:root` (and `prefers-color-scheme`
//! variants), plus the `@name/@author/…` metadata comment used by popular theme
//! formats (e.g. BetterDiscord `*.theme.css`), and maps them to the palette.
//!
//! Safety: nothing is ever fetched. `@import` and `url(...)` values are ignored
//! and reported in [`Theme::warnings`]. Input size is capped and parsing never panics.

mod color;
mod images;
mod parse;
mod remote;
mod store;

pub use color::{Rgb, parse_color};
pub use images::{Decoded, Frame, decode_image};
pub use parse::{Theme, UiColors, parse_theme, parse_theme_with};
pub use remote::{AssetCache, Loaded, fingerprint, is_allowed_url, load_remote};
pub use store::{ThemeEntry, ThemeStore};

/// Largest theme file accepted.
pub const MAX_THEME_BYTES: usize = 256 * 1024;
