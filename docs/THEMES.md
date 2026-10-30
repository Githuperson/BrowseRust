# Custom browser themes

Drop a `.css` file into `<profile>/themes/` (the folder is created with two starter themes on first run; **Menu > Custom theme > Open themes folder** opens it). Pick a theme from the menu or the command palette (Ctrl+K, "Theme: ..."). Saving the file applies the change immediately.

## What is read
The browser chrome is drawn natively, so a theme is **not** executed as CSS. The loader reads:
- the leading `/** @name @author @version @description @source */` comment (BetterDiscord style);
- custom properties (`--name: value`) in `:root` (or `html`);
- `@media (prefers-color-scheme: dark|light) { :root { ... } }` blocks, used when the system appearance matches (variants inherit the base block).

Colours may be `#rgb`, `#rrggbb`, `rgb()`, `hsl()` or a bare `r, g, b` triple (as in `--accent: 37, 172, 232`). Alpha is ignored. `var(--x)` and `var(--x, fallback)` resolve (bounded depth).

## Variables
| Role | Native name (wins) | Also understood |
|---|---|---|
| Toolbar and selected tab | `--bp-bar` | `--background-solid`, `--background-primary`, `--bg`, `--base` |
| Inputs, address bar, chips | `--bp-surface` | `--background-solid-dark`, `--background-secondary`, `--surface` |
| Behind the tabs | `--bp-strip` | `--background-solid-darker`, `--background-tertiary`, `--crust` |
| Text | `--bp-text` | `--text-normal`, `--text`, `--foreground` |
| Secondary text/icons | `--bp-subtle` | `--text-muted`, `--text-secondary` |
| Borders | `--bp-border` | `--border`, `--border-color` |
| Hover fill | `--bp-hover` | `--background-modifier-hover`, `--hover` |
| Accent | `--bp-accent` | `--accent`, `--brand`, `--primary` |

Anything missing is derived from the bar colour (dark or light is decided from its luminance), so a theme with only `--bp-bar` and `--bp-accent` is complete. A theme's accent replaces the accent-colour setting while it is selected.

## Safety
Nothing is fetched. `@import` and `url(...)` (images) are ignored and listed as warnings (hover a theme in the menu). Other rules (class selectors, `@font-face`, ...) are ignored and listed. Files over 256 KiB are rejected. Parsing never panics (tested on hostile input).

## Verified
Unit tests (16) plus your uploaded **Dark Matter** file: its name/author/version were read and it resolved to bar `#161921`, surface `#101218`, strip `#0c0e12`, accent `37,172,232`. On a running window: selecting it recoloured the UI, and editing the file while the browser ran recoloured it live.

## Limits
- Discord-specific selectors and the theme's background images/GIFs are not applied (the browser has no such elements; images are never fetched).
- Fonts, sizes, spacing and shapes are not themeable yet; colours only.
- Colours only light/dark per theme: no separate per-site themes.
- Tested on Linux only.
