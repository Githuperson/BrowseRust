/// An opaque sRGB colour. Alpha in theme files is accepted and ignored.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rgb(pub u8, pub u8, pub u8);

impl Rgb {
    pub const BLACK: Rgb = Rgb(0, 0, 0);
    pub const WHITE: Rgb = Rgb(255, 255, 255);

    /// Linear blend: `t = 0` is `self`, `t = 1` is `other`.
    pub fn mix(self, other: Rgb, t: f32) -> Rgb {
        let m = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * t.clamp(0.0, 1.0)).round() as u8;
        Rgb(m(self.0, other.0), m(self.1, other.1), m(self.2, other.2))
    }

    /// Relative luminance (WCAG), 0 = black, 1 = white.
    pub fn luminance(self) -> f32 {
        let c = |v: u8| {
            let s = v as f32 / 255.0;
            if s <= 0.03928 { s / 12.92 } else { ((s + 0.055) / 1.055).powf(2.4) }
        };
        0.2126 * c(self.0) + 0.7152 * c(self.1) + 0.0722 * c(self.2)
    }
}

fn num(s: &str) -> Option<f32> {
    let s = s.trim();
    if let Some(p) = s.strip_suffix('%') { p.trim().parse::<f32>().ok().map(|v| v / 100.0 * 255.0) } else { s.parse::<f32>().ok() }
}

fn byte(v: f32) -> u8 {
    v.round().clamp(0.0, 255.0) as u8
}

fn hsl(h: f32, s: f32, l: f32) -> Rgb {
    let (s, l) = (s.clamp(0.0, 1.0), l.clamp(0.0, 1.0));
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let hp = (h.rem_euclid(360.0)) / 60.0;
    let x = c * (1.0 - (hp % 2.0 - 1.0).abs());
    let (r, g, b) = match hp as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = l - c / 2.0;
    Rgb(byte((r + m) * 255.0), byte((g + m) * 255.0), byte((b + m) * 255.0))
}

/// Parse a CSS colour: `#rgb`, `#rgba`, `#rrggbb`, `#rrggbbaa`, `rgb()/rgba()` (comma
/// or space syntax), `hsl()/hsla()`, a few named colours, and the bare triple
/// `37, 172, 232` / `37 172 232` that many themes use inside `rgba(var(--x), a)`.
pub fn parse_color(input: &str) -> Option<Rgb> {
    let s = input.trim().trim_end_matches("!important").trim();
    if s.is_empty() {
        return None;
    }
    if let Some(h) = s.strip_prefix('#') {
        if !h.chars().all(|c| c.is_ascii_hexdigit()) {
            return None;
        }
        let d = |i: usize| u8::from_str_radix(&h[i..i + 1], 16).ok().map(|v| v * 17);
        let p = |i: usize| u8::from_str_radix(&h[i..i + 2], 16).ok();
        return match h.len() {
            3 | 4 => Some(Rgb(d(0)?, d(1)?, d(2)?)),
            6 | 8 => Some(Rgb(p(0)?, p(2)?, p(4)?)),
            _ => None,
        };
    }
    let lower = s.to_ascii_lowercase();
    for (name, c) in [("white", Rgb::WHITE), ("black", Rgb::BLACK), ("transparent", Rgb::BLACK), ("red", Rgb(255, 0, 0)), ("blue", Rgb(0, 0, 255)), ("green", Rgb(0, 128, 0))] {
        if lower == name {
            return Some(c);
        }
    }
    if let Some(open) = lower.find('(') {
        let name = lower[..open].trim();
        let inner = lower[open + 1..].strip_suffix(')')?;
        let parts: Vec<&str> = inner.split(|c: char| c == ',' || c == '/' || c.is_whitespace()).filter(|p| !p.is_empty()).collect();
        return match name {
            "rgb" | "rgba" if parts.len() >= 3 => Some(Rgb(byte(num(parts[0])?), byte(num(parts[1])?), byte(num(parts[2])?))),
            "hsl" | "hsla" if parts.len() >= 3 => {
                let h = parts[0].trim_end_matches("deg").parse::<f32>().ok()?;
                let pct = |p: &str| p.trim_end_matches('%').parse::<f32>().ok().map(|v| v / 100.0);
                Some(hsl(h, pct(parts[1])?, pct(parts[2])?))
            }
            _ => None,
        };
    }
    // Bare triple.
    let parts: Vec<&str> = s.split(|c: char| c == ',' || c.is_whitespace()).filter(|p| !p.is_empty()).collect();
    if parts.len() == 3 && parts.iter().all(|p| p.chars().all(|c| c.is_ascii_digit())) {
        let v: Vec<u16> = parts.iter().filter_map(|p| p.parse().ok()).collect();
        if v.len() == 3 && v.iter().all(|&x| x <= 255) {
            return Some(Rgb(v[0] as u8, v[1] as u8, v[2] as u8));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats() {
        assert_eq!(parse_color("#161921"), Some(Rgb(0x16, 0x19, 0x21)));
        assert_eq!(parse_color("#fff"), Some(Rgb(255, 255, 255)));
        assert_eq!(parse_color("#11223344"), Some(Rgb(0x11, 0x22, 0x33)));
        assert_eq!(parse_color("rgb(10, 20, 30)"), Some(Rgb(10, 20, 30)));
        assert_eq!(parse_color("rgba(10 20 30 / 50%)"), Some(Rgb(10, 20, 30)));
        assert_eq!(parse_color("37, 172, 232"), Some(Rgb(37, 172, 232)), "bare triple");
        assert_eq!(parse_color("37 172 232"), Some(Rgb(37, 172, 232)));
        assert_eq!(parse_color("hsl(0, 100%, 50%)"), Some(Rgb(255, 0, 0)));
        assert_eq!(parse_color("hsl(120deg 100% 25%)"), Some(Rgb(0, 128, 0)));
    }

    #[test]
    fn rejects_garbage_without_panicking() {
        for s in ["", "#", "#ggg", "#12345", "rgb(", "rgb(1,2)", "300, 1, 1", "url('x.png')", "1px solid red", "var(--x)", "💥"] {
            let _ = parse_color(s);
        }
        assert_eq!(parse_color("#12345"), None);
        assert_eq!(parse_color("300, 1, 1"), None);
        assert_eq!(parse_color("url('https://x/y.png')"), None);
    }

    #[test]
    fn luminance_and_mix() {
        assert!(Rgb(0x16, 0x19, 0x21).luminance() < 0.05);
        assert!(Rgb(250, 250, 250).luminance() > 0.9);
        assert_eq!(Rgb::BLACK.mix(Rgb::WHITE, 0.5), Rgb(128, 128, 128));
    }
}
