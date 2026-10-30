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

## Remote files: imports and images (with your permission)
Many themes (e.g. BetterDiscord's Dark Matter) `@import` a stylesheet and reference images. These are supported, but **never silently**:
- When you select such a theme, a bar lists how many files it wants and from which hosts (hover for the full URLs): **Allow / Not now / Never for themes**.
- Approval is stored per theme together with a fingerprint of its URLs. If the file is edited to reference different URLs, you are asked again.
- Settings (menu > Custom theme > Theme images): master switch for theme downloads; show the background on the new-tab page and/or sidebar; show the home image; background darkening; "Forget approvals and clear downloaded files".
- What is applied: variables from the imported stylesheet are merged into the palette (the theme's own variables win); `--background-image` (or `--bp-background-image`) becomes a centre-cropped, dimmed backdrop on the new-tab page and sidebar; `--home-image` (or `--bp-home-image`) replaces the title on the new-tab page and **animates if it is a GIF** (up to 64 frames).
- Downloaded files are cached in `<profile>/themes/.cache` (64 MiB cap), so the theme works offline afterwards.

### Rules the loader enforces (tested)
- `https` only; no credentials in URLs; never `localhost`, private (10.x, 192.168.x, 172.16-31.x), link-local (e.g. cloud metadata 169.254.169.254) or other internal addresses; every redirect hop is re-checked; no cookies or auth are sent.
- Size caps: stylesheets 256 KiB, images 8 MiB, source images up to 8192x8192 (then downscaled to 1600 px / 256 px for the home image), at most 4 imports. Images are accepted by **magic bytes** (PNG, JPEG, GIF, WebP); **SVG is refused**.
- Imported stylesheets can add colour variables but **cannot trigger further downloads**: images are fetched only from URLs written in the theme file you approved. Imports of imports are not followed.
- Only `:root` variables are used from imported CSS; the Discord selectors in it (and any other rules) are counted and ignored.

## Safety (local parsing)
Parsing never panics (tested on hostile input); files over 256 KiB are rejected; unused `url(...)` variables are ignored and listed as warnings (hover a theme in the menu).

## Verified
- 23 unit tests + 3 integration tests (local server): nothing is requested without permission; with it, the stylesheet merges, images decode and GIF frames are kept; SVG, oversized files, redirects to private IPs and loopback are refused; a stylesheet cannot smuggle in an extra download; cached files load with the network off.
- On a running window with your real Dark Matter file (URLs rewritten to a local server): consent bar shown before any request; after **Allow** the background, sidebar backdrop and an animating GIF hero appeared; relaunching with the server down loaded from cache without a prompt; editing the theme's image URL asked again.
- **Not verified:** the real hosts (`discordstyles.github.io`, `i.imgur.com`) were unreachable from the test sandbox, so the real `base.css` and images were never fetched or inspected. Their actual contents (and how much of them maps onto this UI) are unknown.

## Limits
- Discord-specific selectors in a theme do not apply (the browser has no such elements). Only the colour variables, background image and home image are used.
- The background shows on the new-tab page and sidebar, not behind the toolbar or tab strip (those stay opaque so text stays legible).
- Fonts, sizes, spacing and shapes are not themeable yet; colours only.
- Colours only light/dark per theme: no separate per-site themes.
- Tested on Linux only.
