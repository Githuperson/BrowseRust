//! Colour palette and global egui style.

use egui::{Color32, CornerRadius, Stroke};

#[derive(Clone, Copy)]
pub struct Palette {
    /// Behind the tabs.
    pub strip: Color32,
    /// Toolbar and the selected tab (they visually merge, like Chrome).
    pub bar: Color32,
    /// Inputs, chips, popovers.
    pub surface: Color32,
    pub text: Color32,
    pub subtle: Color32,
    pub border: Color32,
    pub hover: Color32,
    pub accent: Color32,
}

pub fn palette(dark: bool, accent: Color32) -> Palette {
    if dark {
        Palette {
            strip: Color32::from_rgb(0x17, 0x18, 0x1B),
            bar: Color32::from_rgb(0x26, 0x28, 0x2D),
            surface: Color32::from_rgb(0x1B, 0x1C, 0x20),
            text: Color32::from_rgb(0xE8, 0xEA, 0xED),
            subtle: Color32::from_rgb(0x9A, 0xA0, 0xA6),
            border: Color32::from_rgb(0x3A, 0x3C, 0x42),
            hover: Color32::from_rgb(0x34, 0x36, 0x3C),
            accent,
        }
    } else {
        Palette {
            strip: Color32::from_rgb(0xDE, 0xE1, 0xE6),
            bar: Color32::from_rgb(0xFF, 0xFF, 0xFF),
            surface: Color32::from_rgb(0xF1, 0xF3, 0xF4),
            text: Color32::from_rgb(0x20, 0x21, 0x24),
            subtle: Color32::from_rgb(0x5F, 0x63, 0x68),
            border: Color32::from_rgb(0xDA, 0xDC, 0xE0),
            hover: Color32::from_rgb(0xE8, 0xEA, 0xED),
            accent,
        }
    }
}

pub fn apply(ctx: &egui::Context, accent: Color32) {
    ctx.all_styles_mut(|st| {
        let dark = st.visuals.dark_mode;
        let p = palette(dark, accent);
        let v = &mut st.visuals;
        v.panel_fill = p.bar;
        v.window_fill = p.bar;
        v.extreme_bg_color = p.surface;
        v.faint_bg_color = p.hover;
        v.window_corner_radius = CornerRadius::same(14);
        v.menu_corner_radius = CornerRadius::same(12);
        v.window_stroke = Stroke::new(1.0, p.border);
        v.selection.bg_fill = accent.gamma_multiply(0.45);
        v.selection.stroke = Stroke::new(1.0, accent);
        v.hyperlink_color = accent;
        v.override_text_color = Some(p.text);
        for w in [&mut v.widgets.inactive, &mut v.widgets.hovered, &mut v.widgets.active, &mut v.widgets.open] {
            w.corner_radius = CornerRadius::same(8);
        }
        v.widgets.inactive.bg_fill = p.surface;
        v.widgets.inactive.weak_bg_fill = p.surface;
        v.widgets.hovered.bg_fill = p.hover;
        v.widgets.hovered.weak_bg_fill = p.hover;
        v.widgets.active.bg_fill = accent.gamma_multiply(0.6);
        v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, p.border);
        st.spacing.item_spacing = egui::vec2(8.0, 6.0);
        st.spacing.button_padding = egui::vec2(10.0, 5.0);
        st.spacing.menu_margin = egui::Margin::same(8);
        st.spacing.window_margin = egui::Margin::same(12);
    });
}
