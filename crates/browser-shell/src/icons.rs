//! Vector icons drawn with the painter (no icon font, no images).

use egui::{Color32, Painter, Pos2, Response, Sense, Shape, Stroke, Ui, Vec2, pos2, vec2};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Icon {
    Back,
    Forward,
    Reload,
    Stop,
    Home,
    Menu,
    Star,
    StarFilled,
    Download,
    Plus,
    Close,
    Sidebar,
    Search,
    Sound,
    Muted,
    Pin,
    Folder,
    Check,
    Up,
    Down,
    Zoom,
}

/// Draw `icon` centred at `c`, fitting a `s`x`s` box.
pub fn paint(p: &Painter, icon: Icon, c: Pos2, s: f32, col: Color32) {
    let u = s / 16.0;
    let st = Stroke::new((1.6 * u).max(1.2), col);
    let pt = |x: f32, y: f32| pos2(c.x + (x - 8.0) * u, c.y + (y - 8.0) * u);
    let line = |pts: &[(f32, f32)]| p.add(Shape::line(pts.iter().map(|&(x, y)| pt(x, y)).collect(), st));
    match icon {
        Icon::Back => {
            line(&[(10.5, 3.0), (5.5, 8.0), (10.5, 13.0)]);
        }
        Icon::Forward => {
            line(&[(5.5, 3.0), (10.5, 8.0), (5.5, 13.0)]);
        }
        Icon::Reload => {
            // 3/4 circle with an arrow head
            let n = 22;
            let pts: Vec<Pos2> = (0..=n)
                .map(|i| {
                    let a = -0.6 + (i as f32 / n as f32) * 4.7;
                    pt(8.0 + 5.0 * a.cos(), 8.0 + 5.0 * a.sin())
                })
                .collect();
            p.add(Shape::line(pts, st));
            line(&[(11.8, 1.8), (12.6, 5.2), (9.2, 5.6)]);
        }
        Icon::Stop | Icon::Close => {
            line(&[(4.0, 4.0), (12.0, 12.0)]);
            line(&[(12.0, 4.0), (4.0, 12.0)]);
        }
        Icon::Home => {
            line(&[(2.5, 8.0), (8.0, 2.8), (13.5, 8.0)]);
            line(&[(4.2, 7.2), (4.2, 13.0), (11.8, 13.0), (11.8, 7.2)]);
        }
        Icon::Menu => {
            for y in [3.5, 8.0, 12.5] {
                p.circle_filled(pt(8.0, y), 1.3 * u.max(0.8), col);
            }
        }
        Icon::Star | Icon::StarFilled => {
            let mut pts = Vec::new();
            for i in 0..10 {
                let r = if i % 2 == 0 { 6.2 } else { 2.6 };
                let a = -std::f32::consts::FRAC_PI_2 + i as f32 * std::f32::consts::PI / 5.0;
                pts.push(pt(8.0 + r * a.cos(), 8.4 + r * a.sin()));
            }
            if icon == Icon::StarFilled {
                p.add(Shape::convex_polygon(pts, col, Stroke::NONE));
            } else {
                let mut closed = pts.clone();
                closed.push(pts[0]);
                p.add(Shape::line(closed, st));
            }
        }
        Icon::Download => {
            line(&[(8.0, 2.5), (8.0, 10.0)]);
            line(&[(4.5, 7.0), (8.0, 10.5), (11.5, 7.0)]);
            line(&[(3.0, 13.0), (13.0, 13.0)]);
        }
        Icon::Plus => {
            line(&[(8.0, 3.0), (8.0, 13.0)]);
            line(&[(3.0, 8.0), (13.0, 8.0)]);
        }
        Icon::Sidebar => {
            p.rect_stroke(egui::Rect::from_two_pos(pt(2.0, 3.0), pt(14.0, 13.0)), 2.0 * u, st, egui::StrokeKind::Middle);
            line(&[(6.0, 3.0), (6.0, 13.0)]);
        }
        Icon::Search => {
            p.circle_stroke(pt(7.0, 7.0), 4.2 * u, st);
            line(&[(10.2, 10.2), (13.5, 13.5)]);
        }
        Icon::Sound => {
            p.add(Shape::convex_polygon(vec![pt(2.5, 6.0), pt(5.5, 6.0), pt(9.0, 3.0), pt(9.0, 13.0), pt(5.5, 10.0), pt(2.5, 10.0)], col, Stroke::NONE));
            line(&[(11.2, 5.5), (12.6, 8.0), (11.2, 10.5)]);
        }
        Icon::Muted => {
            p.add(Shape::convex_polygon(vec![pt(2.5, 6.0), pt(5.5, 6.0), pt(9.0, 3.0), pt(9.0, 13.0), pt(5.5, 10.0), pt(2.5, 10.0)], col, Stroke::NONE));
            line(&[(11.0, 5.5), (14.0, 10.5)]);
            line(&[(14.0, 5.5), (11.0, 10.5)]);
        }
        Icon::Pin => {
            line(&[(8.0, 2.5), (8.0, 9.0)]);
            line(&[(4.5, 9.0), (11.5, 9.0)]);
            line(&[(8.0, 9.0), (8.0, 14.0)]);
        }
        Icon::Folder => {
            line(&[(2.0, 4.5), (6.0, 4.5), (7.5, 6.0), (14.0, 6.0), (14.0, 12.5), (2.0, 12.5), (2.0, 4.5)]);
        }
        Icon::Check => {
            line(&[(3.0, 8.5), (6.5, 12.0), (13.0, 4.5)]);
        }
        Icon::Up => {
            line(&[(3.5, 10.0), (8.0, 5.5), (12.5, 10.0)]);
        }
        Icon::Down => {
            line(&[(3.5, 6.0), (8.0, 10.5), (12.5, 6.0)]);
        }
        Icon::Zoom => {
            p.circle_stroke(pt(7.0, 7.0), 4.2 * u, st);
            line(&[(10.2, 10.2), (13.5, 13.5)]);
            line(&[(5.2, 7.0), (8.8, 7.0)]);
        }
    }
}

/// A square icon button with an animated hover pill. `badge` draws a small count.
pub fn button(ui: &mut Ui, pal: &crate::theme::Palette, icon: Icon, tip: &str, enabled: bool, active: bool, badge: Option<u32>) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(32.0, 32.0), if enabled { Sense::click() } else { Sense::hover() });
    let t = ui.ctx().animate_bool_with_time(resp.id, resp.hovered() && enabled, 0.12);
    if t > 0.0 || active {
        let fill = if active { pal.accent.gamma_multiply(0.28) } else { pal.hover.gamma_multiply(t) };
        ui.painter().rect_filled(rect, 8.0, fill);
    }
    let col = if !enabled { pal.subtle.gamma_multiply(0.45) } else if active { pal.accent } else { pal.text };
    paint(ui.painter(), icon, rect.center(), 16.0, col);
    if let Some(n) = badge.filter(|n| *n > 0) {
        let c = rect.right_top() + Vec2::new(-6.0, 7.0);
        ui.painter().circle_filled(c, 7.0, pal.accent);
        ui.painter().text(c, egui::Align2::CENTER_CENTER, n.to_string(), egui::FontId::proportional(9.5), Color32::WHITE);
    }
    resp.on_hover_text(tip)
}
