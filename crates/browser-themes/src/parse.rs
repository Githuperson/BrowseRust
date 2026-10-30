use crate::color::{Rgb, parse_color};
use std::collections::HashMap;

/// The palette a theme resolves to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UiColors {
    /// Behind the tabs (darkest layer in a dark theme).
    pub strip: Rgb,
    /// Toolbar and selected tab.
    pub bar: Rgb,
    /// Inputs and chips.
    pub surface: Rgb,
    pub text: Rgb,
    pub subtle: Rgb,
    pub border: Rgb,
    pub hover: Rgb,
    pub accent: Rgb,
    /// Chosen from the bar's luminance; the shell switches egui's base style to match.
    pub dark: bool,
}

#[derive(Clone, Debug, Default)]
pub struct Theme {
    pub name: String,
    pub author: String,
    pub version: String,
    pub description: String,
    pub source: String,
    /// Colours from `:root` (no media query).
    pub colors: Option<UiColors>,
    /// `@media (prefers-color-scheme: dark)` / `light` overrides, if the theme has them.
    pub dark_variant: Option<UiColors>,
    pub light_variant: Option<UiColors>,
    /// Things that were deliberately not honoured (remote imports, images, other rules).
    pub warnings: Vec<String>,
}

impl Theme {
    /// Colours to use given the system's current appearance.
    pub fn resolve(&self, system_dark: bool) -> Option<UiColors> {
        if system_dark { self.dark_variant.or(self.colors).or(self.light_variant) } else { self.light_variant.or(self.colors).or(self.dark_variant) }
    }
}

type Vars = HashMap<String, String>;

/// Remove `/* ... */` comments (not inside strings). Unterminated comments run to the end.
fn strip_comments(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    let (mut i, mut quote) = (0, None::<u8>);
    let mut start = 0;
    while i < b.len() {
        match quote {
            Some(q) => {
                if b[i] == b'\\' {
                    i += 1;
                } else if b[i] == q {
                    quote = None;
                }
                i += 1;
            }
            None if b[i] == b'"' || b[i] == b'\'' => {
                quote = Some(b[i]);
                i += 1;
            }
            None if b[i] == b'/' && b.get(i + 1) == Some(&b'*') => {
                out.push_str(&s[start..i]);
                i += 2;
                while i + 1 < b.len() && !(b[i] == b'*' && b[i + 1] == b'/') {
                    i += 1;
                }
                i = (i + 2).min(b.len());
                start = i;
            }
            None => i += 1,
        }
    }
    out.push_str(&s[start.min(s.len())..]);
    out
}

/// Metadata from the leading `/** @key value */` block.
fn metadata(src: &str, t: &mut Theme) {
    let Some(start) = src.find("/*") else { return };
    let Some(len) = src[start..].find("*/") else { return };
    for line in src[start + 2..start + len].lines() {
        let line = line.trim().trim_start_matches('*').trim();
        let Some(rest) = line.strip_prefix('@') else { continue };
        let (k, v) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
        let v = v.trim().to_string();
        match k {
            "name" => t.name = v,
            "author" => t.author = v,
            "version" => t.version = v,
            "description" => t.description = v,
            "source" | "website" => t.source = v,
            _ => {}
        }
    }
}

/// Split a block body on top-level `;` (ignoring `;` inside parentheses or strings).
fn split_decls(body: &str) -> Vec<&str> {
    let (mut out, mut depth, mut quote, mut start) = (Vec::new(), 0i32, None::<char>, 0);
    let mut prev = '\0';
    for (i, c) in body.char_indices() {
        match quote {
            Some(q) => {
                if c == q && prev != '\\' {
                    quote = None;
                }
            }
            None => match c {
                '"' | '\'' => quote = Some(c),
                '(' => depth += 1,
                ')' => depth = (depth - 1).max(0),
                ';' if depth == 0 => {
                    out.push(&body[start..i]);
                    start = i + 1;
                }
                _ => {}
            },
        }
        prev = c;
    }
    out.push(&body[start..]);
    out
}

/// Index of the `}` matching the `{` at `open`, or `None`.
fn matching_brace(s: &str, open: usize) -> Option<usize> {
    let (mut depth, mut quote) = (0i32, None::<char>);
    let mut prev = '\0';
    for (i, c) in s[open..].char_indices() {
        match quote {
            Some(q) => {
                if c == q && prev != '\\' {
                    quote = None;
                }
            }
            None => match c {
                '"' | '\'' => quote = Some(c),
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(open + i);
                    }
                }
                _ => {}
            },
        }
        prev = c;
    }
    None
}

#[derive(Default)]
struct Collected {
    base: Vars,
    dark: Vars,
    light: Vars,
    warnings: Vec<String>,
}

fn scan(css: &str, variant: Option<bool>, out: &mut Collected, depth: u32) {
    let mut i = 0;
    while i < css.len() {
        let rest = &css[i..];
        let rest_trim = rest.trim_start();
        i += rest.len() - rest_trim.len();
        if rest_trim.is_empty() {
            break;
        }
        let semi = rest_trim.find(';');
        let open = rest_trim.find('{');
        match (semi, open) {
            // Statement at-rule: `@import ...;`
            (Some(s), o) if o.is_none_or(|o| s < o) => {
                let stmt = rest_trim[..s].trim();
                if stmt.starts_with("@import") {
                    out.warnings.push("@import ignored: nothing is fetched from the network".into());
                } else if !stmt.is_empty() {
                    out.warnings.push(format!("ignored: {}", truncate(stmt)));
                }
                i += s + 1;
            }
            (_, Some(o)) => {
                let prelude = rest_trim[..o].trim();
                let abs_open = i + (rest.len() - rest_trim.len()).saturating_sub(rest.len() - rest_trim.len()) + o;
                let _ = abs_open;
                let Some(close) = matching_brace(rest_trim, o) else {
                    out.warnings.push("unterminated block ignored".into());
                    break;
                };
                let body = &rest_trim[o + 1..close];
                let low = prelude.to_ascii_lowercase();
                if low.starts_with("@media") && depth < 4 {
                    let v = if low.contains("prefers-color-scheme") && low.contains("dark") {
                        Some(true)
                    } else if low.contains("prefers-color-scheme") && low.contains("light") {
                        Some(false)
                    } else {
                        None
                    };
                    if v.is_some() || variant.is_some() {
                        scan(body, v.or(variant), out, depth + 1);
                    } else {
                        out.warnings.push(format!("ignored: {}", truncate(prelude)));
                    }
                } else if low.starts_with('@') {
                    out.warnings.push(format!("ignored: {}", truncate(prelude)));
                } else if prelude.split(',').any(|s| matches!(s.trim().to_ascii_lowercase().as_str(), ":root" | "html" | ":host")) {
                    let target = match variant {
                        Some(true) => &mut out.dark,
                        Some(false) => &mut out.light,
                        None => &mut out.base,
                    };
                    for d in split_decls(body) {
                        if let Some((k, v)) = d.split_once(':') {
                            let k = k.trim();
                            if k.starts_with("--") && !k.is_empty() {
                                target.insert(k.to_string(), v.trim().trim_end_matches("!important").trim().to_string());
                            }
                        }
                    }
                } else {
                    out.warnings.push(format!("rule not applicable to the browser UI: {}", truncate(prelude)));
                }
                i += o + (close - o) + 1;
            }
            _ => break,
        }
    }
}

fn truncate(s: &str) -> String {
    let s: String = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if s.chars().count() > 70 { format!("{}...", s.chars().take(70).collect::<String>()) } else { s }
}

/// Resolve `var(--x)` / `var(--x, fallback)` references (bounded depth, cycle-safe).
fn resolve(vars: &Vars, name: &str, depth: u32) -> Option<String> {
    if depth > 8 {
        return None;
    }
    let mut v = vars.get(name)?.clone();
    for _ in 0..8 {
        let Some(start) = v.find("var(") else { break };
        let after = &v[start + 4..];
        let mut level = 1;
        let mut end = None;
        for (i, c) in after.char_indices() {
            match c {
                '(' => level += 1,
                ')' => {
                    level -= 1;
                    if level == 0 {
                        end = Some(i);
                        break;
                    }
                }
                _ => {}
            }
        }
        let end = end?;
        let inner = &after[..end];
        let (refname, fallback) = inner.split_once(',').map(|(a, b)| (a.trim(), Some(b.trim()))).unwrap_or((inner.trim(), None));
        let value = resolve(vars, refname, depth + 1).or_else(|| fallback.map(str::to_string))?;
        v = format!("{}{}{}", &v[..start], value, &after[end + 1..]);
    }
    Some(v)
}

/// First variable among `names` that resolves to a colour.
fn pick(vars: &Vars, names: &[&str]) -> Option<Rgb> {
    names.iter().find_map(|n| resolve(vars, n, 0).and_then(|v| parse_color(&v)))
}

/// Variable names, most specific first: native `--bp-*`, then common conventions
/// (BetterDiscord themes, Discord/Catppuccin/Solarized-style names).
const BAR: &[&str] = &["--bp-bar", "--background-solid", "--background-primary", "--bg-primary", "--background", "--bg", "--base", "--bg-color"];
const SURFACE: &[&str] = &["--bp-surface", "--background-solid-dark", "--background-secondary", "--bg-secondary", "--surface", "--mantle"];
const STRIP: &[&str] = &["--bp-strip", "--background-solid-darker", "--background-tertiary", "--bg-tertiary", "--crust"];
const TEXT: &[&str] = &["--bp-text", "--text-normal", "--text", "--foreground", "--fg", "--text-color"];
const SUBTLE: &[&str] = &["--bp-subtle", "--text-muted", "--text-secondary", "--subtext"];
const BORDER: &[&str] = &["--bp-border", "--border", "--border-color", "--background-modifier-accent"];
const HOVER: &[&str] = &["--bp-hover", "--background-modifier-hover", "--hover", "--bg-hover"];
const ACCENT: &[&str] = &["--bp-accent", "--accent", "--accent-color", "--brand", "--brand-experiment", "--primary", "--blue"];

fn colors_from(vars: &Vars, inherit: Option<&Vars>) -> Option<UiColors> {
    // Variant blocks overlay the base block.
    let merged: Vars = match inherit {
        Some(base) => base.iter().chain(vars.iter()).map(|(k, v)| (k.clone(), v.clone())).collect(),
        None => vars.clone(),
    };
    let accent_found = pick(&merged, ACCENT);
    let bar_found = pick(&merged, BAR);
    if bar_found.is_none() && accent_found.is_none() && pick(&merged, SURFACE).is_none() && pick(&merged, STRIP).is_none() {
        return None;
    }
    // Anchor on whichever background layer the theme gave us.
    let anchor = bar_found.or_else(|| pick(&merged, SURFACE)).or_else(|| pick(&merged, STRIP)).unwrap_or(Rgb(0x26, 0x28, 0x2D));
    let dark = anchor.luminance() < 0.4;
    let text = pick(&merged, TEXT).unwrap_or(if dark { Rgb(0xE8, 0xEA, 0xED) } else { Rgb(0x20, 0x21, 0x24) });
    let bar = bar_found.unwrap_or(anchor);
    let (surface_d, strip_d) = if dark { (Rgb::BLACK, Rgb::BLACK) } else { (Rgb::BLACK, Rgb::BLACK) };
    let surface = pick(&merged, SURFACE).unwrap_or_else(|| if dark { bar.mix(surface_d, 0.25) } else { bar.mix(surface_d, 0.04) });
    let strip = pick(&merged, STRIP).unwrap_or_else(|| if dark { bar.mix(strip_d, 0.38) } else { bar.mix(strip_d, 0.09) });
    let accent = accent_found.unwrap_or(Rgb(0x4F, 0x7C, 0xFF));
    Some(UiColors {
        strip,
        bar,
        surface,
        text,
        subtle: pick(&merged, SUBTLE).unwrap_or_else(|| text.mix(bar, 0.45)),
        border: pick(&merged, BORDER).unwrap_or_else(|| bar.mix(text, 0.14)),
        hover: pick(&merged, HOVER).unwrap_or_else(|| bar.mix(text, 0.09)),
        accent,
        dark,
    })
}

/// Parse a theme file. Never panics; returns `Err` only for oversized input.
pub fn parse_theme(src: &str) -> Result<Theme, String> {
    if src.len() > crate::MAX_THEME_BYTES {
        return Err(format!("theme file is larger than {} KiB", crate::MAX_THEME_BYTES / 1024));
    }
    let mut t = Theme::default();
    metadata(src, &mut t);
    let css = strip_comments(src);
    let mut c = Collected::default();
    scan(&css, None, &mut c, 0);

    // `url(...)` values are never loaded.
    let mut image_vars: Vec<&String> = c.base.iter().chain(c.dark.iter()).chain(c.light.iter()).filter(|(_, v)| v.contains("url(")).map(|(k, _)| k).collect();
    image_vars.sort();
    image_vars.dedup();
    for k in image_vars {
        c.warnings.push(format!("{k}: url(...) ignored: images are never fetched by themes"));
    }

    t.colors = colors_from(&c.base, None);
    t.dark_variant = if c.dark.is_empty() { None } else { colors_from(&c.dark, Some(&c.base)) };
    t.light_variant = if c.light.is_empty() { None } else { colors_from(&c.light, Some(&c.base)) };
    if t.colors.is_none() && t.dark_variant.is_none() && t.light_variant.is_none() {
        c.warnings.push("no usable colour variables found (expected --bp-bar / --background-solid / --accent ...)".into());
    }
    c.warnings.dedup();
    t.warnings = c.warnings;
    Ok(t)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Same structure as a BetterDiscord "Dark Matter" style theme.
    const DISCORD_STYLE: &str = r#"/**
 * @name Dark Matter
 * @author Someone
 * @version 3.0.0
 * @description A cold, dark & frosty theme.
 * @source https://example.com/darkmatter
*/

@import url('https://example.com/src/base.css');

/* Variables */
:root {
    --avatar-size: 32px;
    --background-image: url('https://example.com/bg.png');
    --background-solid: #161921;
    --background-solid-dark: #101218;
    --background-solid-darker: #0c0e12;
    --accent: 37, 172, 232;
    --accent-alt: 29, 101, 134;
}
"#;

    #[test]
    fn reads_a_betterdiscord_style_theme() {
        let t = parse_theme(DISCORD_STYLE).unwrap();
        assert_eq!((t.name.as_str(), t.author.as_str(), t.version.as_str()), ("Dark Matter", "Someone", "3.0.0"));
        assert_eq!(t.description, "A cold, dark & frosty theme.");
        let c = t.colors.unwrap();
        assert_eq!(c.bar, Rgb(0x16, 0x19, 0x21));
        assert_eq!(c.surface, Rgb(0x10, 0x12, 0x18));
        assert_eq!(c.strip, Rgb(0x0c, 0x0e, 0x12));
        assert_eq!(c.accent, Rgb(37, 172, 232), "bare r, g, b triple");
        assert!(c.dark);
        assert!(c.text.luminance() > 0.7, "derived light text for a dark theme");
        assert!(c.subtle != c.bar && c.border != c.bar && c.hover != c.bar);
    }

    #[test]
    fn remote_content_is_reported_not_loaded() {
        let t = parse_theme(DISCORD_STYLE).unwrap();
        assert!(t.warnings.iter().any(|w| w.contains("@import ignored")), "{:?}", t.warnings);
        assert!(t.warnings.iter().any(|w| w.contains("--background-image") && w.contains("never fetched")), "{:?}", t.warnings);
    }

    #[test]
    fn native_variables_override_conventions_and_var_references_resolve() {
        let t = parse_theme(":root{--base:#202020; --bp-bar:#303030; --accent:#ff0000; --bp-accent:var(--brand); --brand:#00ff00}").unwrap();
        let c = t.colors.unwrap();
        assert_eq!(c.bar, Rgb(0x30, 0x30, 0x30));
        assert_eq!(c.accent, Rgb(0, 255, 0));
        let f = parse_theme(":root{--bp-bar:#111111; --bp-accent:var(--missing, #123456)}").unwrap();
        assert_eq!(f.colors.unwrap().accent, Rgb(0x12, 0x34, 0x56), "fallback used");
    }

    #[test]
    fn light_themes_get_dark_text_and_a_darker_strip() {
        let c = parse_theme(":root{--bp-bar:#ffffff; --bp-accent:#0a84ff}").unwrap().colors.unwrap();
        assert!(!c.dark);
        assert!(c.text.luminance() < 0.1);
        assert!(c.strip.luminance() < c.bar.luminance());
    }

    #[test]
    fn color_scheme_media_blocks_become_variants() {
        let t = parse_theme(":root{--bp-accent:#ff8800} @media (prefers-color-scheme: dark){:root{--bp-bar:#101010}} @media (prefers-color-scheme: light){:root{--bp-bar:#fafafa}}").unwrap();
        assert!(t.resolve(true).unwrap().dark);
        assert!(!t.resolve(false).unwrap().dark);
        assert_eq!(t.resolve(true).unwrap().accent, Rgb(0xff, 0x88, 0), "variants inherit the base block");
    }

    #[test]
    fn unrelated_rules_are_ignored_and_reported() {
        let t = parse_theme(".sidebar { color: red } @font-face { src: url(x) } :root{--bp-bar:#222}").unwrap();
        assert!(t.colors.is_some());
        assert!(t.warnings.iter().any(|w| w.contains(".sidebar")));
        assert!(t.warnings.iter().any(|w| w.contains("@font-face")));
    }

    #[test]
    fn a_file_with_no_colours_is_reported() {
        let t = parse_theme("body { color: red }").unwrap();
        assert!(t.colors.is_none());
        assert!(t.warnings.iter().any(|w| w.contains("no usable colour")));
    }

    #[test]
    fn hostile_input_never_panics() {
        for s in [
            "", "{", "}", ":root{", ":root{--a", ":root{--a:", ":root{--a:var(", "/* unterminated", "@import", "@media{", ":root{--a:var(--a)}",
            ":root{--a:var(--b);--b:var(--a)}", "💥{💥:💥}", "\"unterminated", ":root{--x:'}'; --bp-bar:#101010}", "}}}{{{;;;",
        ] {
            let _ = parse_theme(s);
        }
        let t = parse_theme(":root{--bp-bar:var(--bp-bar)}").unwrap();
        assert!(t.colors.is_none(), "self reference does not loop");
        let big = "a".repeat(crate::MAX_THEME_BYTES + 1);
        assert!(parse_theme(&big).is_err());
    }

    #[test]
    fn braces_inside_strings_do_not_confuse_the_scanner() {
        let t = parse_theme(":root{--label:'}{'; --bp-bar:#101010}").unwrap();
        assert_eq!(t.colors.unwrap().bar, Rgb(0x10, 0x10, 0x10));
    }
}
