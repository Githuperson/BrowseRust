//! The native interface, drawn with egui (immediate mode, no web UI).

use browser_app::Browser;
use browser_core::{TabId, TabState};
use browser_security::{SecurityLevel, security_level};
use browser_settings::{BackgroundMode, Density, SiteMode, SiteSettings, Theme};
use browser_storage::Store;
use egui::{Align2, Color32, CornerRadius, FontId, Rect, Sense, Stroke, StrokeKind, Vec2};
use servo_integration::ServoEngine;
use std::time::Duration;
use url::Url;

#[derive(Default)]
pub struct UiState {
    pub address: String,
    pub focus_address: bool,
    pub show_memory: bool,
    pub show_history: bool,
    pub show_bookmarks: bool,
    pub status: String,
    last_selected: Option<TabId>,
    last_url: Option<String>,
    tab_rects: Vec<(TabId, Rect)>,
    new_workspace: String,
}

pub fn accent(b: &Browser<ServoEngine>) -> Color32 {
    let a = b.settings.appearance.accent;
    Color32::from_rgb(a[0], a[1], a[2])
}

fn state_color(s: TabState, accent: Color32) -> Color32 {
    match s {
        TabState::Foreground => accent,
        TabState::BackgroundWarm => Color32::from_rgb(0xE5, 0xA0, 0x2B),
        TabState::BackgroundFrozen => Color32::from_rgb(0x6C, 0x8E, 0xBF),
        TabState::Discarded => Color32::from_gray(120),
    }
}

pub fn apply_theme(ctx: &egui::Context, b: &Browser<ServoEngine>) {
    ctx.set_theme(match b.settings.appearance.theme {
        Theme::Light => egui::ThemePreference::Light,
        Theme::Dark => egui::ThemePreference::Dark,
        Theme::System => egui::ThemePreference::System,
    });
    let acc = accent(b);
    ctx.all_styles_mut(|st| {
        st.visuals.selection.bg_fill = acc.gamma_multiply(0.5);
        st.visuals.hyperlink_color = acc;
        st.visuals.widgets.hovered.corner_radius = CornerRadius::same(8);
        st.visuals.widgets.inactive.corner_radius = CornerRadius::same(8);
        st.visuals.widgets.active.corner_radius = CornerRadius::same(8);
    });
}

fn density_px(d: Density) -> (f32, f32) {
    // (tab strip height, toolbar height)
    match d {
        Density::Compact => (32.0, 34.0),
        Density::Comfortable => (38.0, 42.0),
        Density::Spacious => (44.0, 48.0),
    }
}

/// Draw the whole UI. `paint_content` is called with the rectangle reserved for
/// web content so the shell can attach the Servo paint callback.
pub fn show(ui: &mut egui::Ui, st: &mut UiState, b: &mut Browser<ServoEngine>, store: &Store, now: i64, paint_content: impl FnOnce(&egui::Painter, Rect)) -> Rect {
    let acc = accent(b);
    let (tab_h, bar_h) = density_px(b.settings.appearance.density);
    let selected = b.tabs.strip.selected;

    // Keep the address bar in sync with the page unless the user is typing.
    let cur_url = selected.and_then(|t| b.tabs.strip.tab(t)).and_then(|t| t.url.as_ref()).map(Url::to_string);
    if selected != st.last_selected || (cur_url != st.last_url && !ui.ctx().memory(|m| m.has_focus(egui::Id::new("address")))) {
        st.address = cur_url.clone().filter(|u| u != "about:blank").unwrap_or_default();
        st.last_selected = selected;
    }
    st.last_url = cur_url;

    // ---------------------------------------------------------------- tabs
    egui::Panel::top("tabs").exact_size(tab_h + 8.0).show(ui, |ui| {
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            workspace_menu(ui, st, b);
            tab_strip(ui, st, b, tab_h, acc);
            if ui.add(egui::Button::new("+").min_size(Vec2::splat(tab_h - 6.0))).on_hover_text("New tab (Ctrl+T)").clicked() {
                let home = b.settings.home_page.clone();
                b.open_tab(Url::parse(&home).ok(), true);
                st.focus_address = true;
            }
        });
    });

    // ------------------------------------------------------------- toolbar
    egui::Panel::top("toolbar").exact_size(bar_h).show(ui, |ui| {
        ui.horizontal_centered(|ui| {
            let (can_back, can_fwd, loading) = selected
                .and_then(|t| b.engine.webview(t))
                .map(|w| (w.can_go_back(), w.can_go_forward(), b.tabs.strip.selected.and_then(|t| b.tabs.strip.tab(t)).is_some_and(|t| t.loading)))
                .unwrap_or_default();
            if ui.add_enabled(can_back, egui::Button::new("<")).on_hover_text("Back (Alt+Left)").clicked() {
                b.go_back();
            }
            if ui.add_enabled(can_fwd, egui::Button::new(">")).on_hover_text("Forward (Alt+Right)").clicked() {
                b.go_forward();
            }
            if loading {
                if ui.button("x").on_hover_text("Stop").clicked() {
                    b.stop();
                }
            } else if ui.button("R").on_hover_text("Reload (Ctrl+R)").clicked() {
                b.reload();
            }
            if ui.button("H").on_hover_text("Home").clicked() {
                let home = b.settings.home_page.clone();
                let _ = b.navigate_input(&home);
            }
            address_bar(ui, st, b, store, now);
            menu_button(ui, st, b, store);
        });
    });

    // -------------------------------------------------------------- status
    egui::Panel::bottom("status").exact_size(22.0).show(ui, |ui| {
        ui.horizontal_centered(|ui| {
            let st_txt = selected.and_then(|t| b.tabs.strip.tab(t)).map(|t| t.state.label()).unwrap_or("");
            ui.label(egui::RichText::new(&st.status).small());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if let Some(r) = b.memory_report() {
                    ui.label(egui::RichText::new(format!("{}  |  pressure: {:?}  |  budget {}", r.browser_total, r.pressure, r.budget.global_limit)).small());
                }
                let blocked = selected.map(|t| b.popups.borrow().blocked_count(t)).unwrap_or(0);
                if blocked > 0 {
                    ui.label(egui::RichText::new(format!("{blocked} popup(s) blocked")).small().color(Color32::from_rgb(0xE5, 0x7A, 0x2B)));
                }
                let _ = st_txt;
            });
        });
    });

    windows(ui.ctx(), st, b, store, now);

    // ------------------------------------------------------------- content
    let mut content = Rect::NOTHING;
    egui::CentralPanel::default().frame(egui::Frame::NONE).show(ui, |ui| {
        let (rect, _resp) = ui.allocate_exact_size(ui.available_size(), Sense::hover());
        content = rect;
        paint_content(ui.painter(), rect);
    });
    content
}

fn workspace_menu(ui: &mut egui::Ui, st: &mut UiState, b: &mut Browser<ServoEngine>) {
    let active = b.tabs.strip.active_workspace;
    let name = b.tabs.strip.workspaces.iter().find(|w| w.id == active).map(|w| w.name.clone()).unwrap_or_default();
    ui.menu_button(egui::RichText::new(name).strong(), |ui| {
        let list: Vec<_> = b.tabs.strip.workspaces.iter().map(|w| (w.id, w.name.clone())).collect();
        for (id, n) in list {
            if ui.selectable_label(id == active, n).clicked() {
                b.tabs.strip.active_workspace = id;
                let first = b.tabs.strip.visible().next().map(|t| t.id);
                match first {
                    Some(t) => b.select_tab(t),
                    None => {
                        b.open_tab(None, true);
                    }
                }
                ui.close();
            }
        }
        ui.separator();
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut st.new_workspace).hint_text("New workspace").desired_width(110.0));
            if ui.button("Add").clicked() && !st.new_workspace.trim().is_empty() {
                let id = b.tabs.strip.add_workspace(st.new_workspace.trim());
                b.tabs.strip.active_workspace = id;
                b.open_tab(None, true);
                st.new_workspace.clear();
                ui.close();
            }
        });
    });
}

fn tab_strip(ui: &mut egui::Ui, st: &mut UiState, b: &mut Browser<ServoEngine>, h: f32, acc: Color32) {
    let tabs: Vec<(TabId, String, TabState, bool, bool, bool, bool)> = b
        .tabs
        .strip
        .visible()
        .map(|t| (t.id, t.display_title().to_string(), t.state, t.pinned, t.loading, t.muted, t.audible))
        .collect();
    let n = tabs.len().max(1) as f32;
    let pinned_n = tabs.iter().filter(|t| t.3).count() as f32;
    let avail = (ui.available_width() - 48.0 - pinned_n * 44.0).max(60.0);
    let w = (avail / (n - pinned_n).max(1.0)).clamp(70.0, 220.0);
    let selected = b.tabs.strip.selected;
    st.tab_rects.clear();
    let dark = ui.visuals().dark_mode;
    let mut action: Option<TabAction> = None;
    ui.spacing_mut().item_spacing.x = 4.0;
    for (id, title, state, pinned, loading, muted, audible) in tabs {
        let size = Vec2::new(if pinned { 40.0 } else { w }, h);
        let (rect, resp) = ui.allocate_exact_size(size, Sense::click_and_drag());
        st.tab_rects.push((id, rect));
        let is_sel = selected == Some(id);
        let fill = match (is_sel, resp.hovered(), dark) {
            (true, _, true) => Color32::from_gray(52),
            (true, _, false) => Color32::from_gray(255),
            (false, true, true) => Color32::from_gray(40),
            (false, true, false) => Color32::from_gray(228),
            (false, false, true) => Color32::from_gray(30),
            (false, false, false) => Color32::from_gray(214),
        };
        let p = ui.painter();
        p.rect_filled(rect, CornerRadius { nw: 10, ne: 10, sw: 3, se: 3 }, fill);
        if is_sel {
            p.rect_stroke(rect, CornerRadius { nw: 10, ne: 10, sw: 3, se: 3 }, Stroke::new(1.5, acc), StrokeKind::Inside);
        }
        // State indicator dot: the "unobtrusive" lifecycle indicator.
        let dot_c = rect.left_center() + Vec2::new(12.0, 0.0);
        let col = state_color(state, acc);
        if state == TabState::Discarded {
            p.circle_stroke(dot_c, 4.0, Stroke::new(1.5, col));
        } else {
            p.circle_filled(dot_c, 4.0, col);
        }
        if !pinned {
            let text_rect = Rect::from_min_max(rect.left_top() + Vec2::new(24.0, 0.0), rect.right_bottom() - Vec2::new(24.0, 0.0));
            let color = if is_sel { ui.visuals().strong_text_color() } else { ui.visuals().text_color() };
            let mut label = title.clone();
            if audible && !muted {
                label.insert_str(0, "(sound) ");
            }
            if muted {
                label.insert_str(0, "(muted) ");
            }
            p.with_clip_rect(text_rect).text(text_rect.left_center(), Align2::LEFT_CENTER, label, FontId::proportional(13.0), color);
            if loading {
                ui.put(Rect::from_center_size(rect.right_center() - Vec2::new(30.0, 0.0), Vec2::splat(12.0)), egui::Spinner::new().size(12.0));
            }
            if resp.hovered() || is_sel {
                let cr = Rect::from_center_size(rect.right_center() - Vec2::new(12.0, 0.0), Vec2::splat(16.0));
                let cresp = ui.interact(cr, resp.id.with("close"), Sense::click());
                if cresp.hovered() {
                    ui.painter().rect_filled(cr, CornerRadius::same(4), ui.visuals().widgets.hovered.bg_fill);
                }
                ui.painter().text(cr.center(), Align2::CENTER_CENTER, "x", FontId::proportional(12.0), ui.visuals().text_color());
                if cresp.clicked() {
                    action = Some(TabAction::Close(id));
                }
            }
        }
        if action.is_none() {
            if resp.clicked() {
                action = Some(TabAction::Select(id));
            } else if resp.middle_clicked() {
                action = Some(TabAction::Close(id));
            }
        }
        if resp.drag_stopped() {
            if let Some(pos) = ui.input(|i| i.pointer.interact_pos()) {
                if let Some(target) = st.tab_rects.iter().position(|(_, r)| r.x_range().contains(pos.x)).or_else(|| (pos.x > rect.max.x).then(|| st.tab_rects.len().saturating_sub(1))) {
                    action = Some(TabAction::Move(id, target));
                }
            }
        }
        resp.on_hover_text(format!("{title}\nState: {}", state.label())).context_menu(|ui| {
            if ui.button("Duplicate").clicked() {
                action = Some(TabAction::Duplicate(id));
                ui.close();
            }
            if ui.button(if pinned { "Unpin" } else { "Pin" }).clicked() {
                action = Some(TabAction::Pin(id, !pinned));
                ui.close();
            }
            if ui.button(if muted { "Unmute" } else { "Mute" }).clicked() {
                action = Some(TabAction::Mute(id, !muted));
                ui.close();
            }
            if state.is_background() && ui.button("Discard (free memory)").clicked() {
                action = Some(TabAction::Discard(id));
                ui.close();
            }
            ui.separator();
            if ui.button("Close").clicked() {
                action = Some(TabAction::Close(id));
                ui.close();
            }
            if ui.button("Reopen closed tab").clicked() {
                action = Some(TabAction::Reopen);
                ui.close();
            }
        });
    }
    if let Some(a) = action {
        match a {
            TabAction::Select(t) => b.select_tab(t),
            TabAction::Close(t) => b.close_tab(t),
            TabAction::Duplicate(t) => {
                b.duplicate_tab(t);
            }
            TabAction::Pin(t, p) => b.set_pinned(t, p),
            TabAction::Mute(t, m) => b.set_muted(t, m),
            TabAction::Discard(t) => b.discard_tab(t),
            TabAction::Reopen => b.reopen_closed_tab(),
            TabAction::Move(t, i) => {
                let global = b.tabs.strip.visible().nth(i).and_then(|x| b.tabs.strip.index_of(x.id)).unwrap_or(i);
                b.tabs.strip.move_tab(t, global);
            }
        }
    }
}

enum TabAction {
    Select(TabId),
    Close(TabId),
    Duplicate(TabId),
    Pin(TabId, bool),
    Mute(TabId, bool),
    Discard(TabId),
    Reopen,
    Move(TabId, usize),
}

fn address_bar(ui: &mut egui::Ui, st: &mut UiState, b: &mut Browser<ServoEngine>, store: &Store, now: i64) {
    let sel_url = b.tabs.strip.selected.and_then(|t| b.tabs.strip.tab(t)).and_then(|t| t.url.clone());
    if let Some(u) = &sel_url {
        let (txt, col) = match security_level(u) {
            SecurityLevel::Secure => ("Secure", Color32::from_rgb(0x2E, 0xA0, 0x4F)),
            SecurityLevel::NotSecure => ("Not secure", Color32::from_rgb(0xD9, 0x6A, 0x1E)),
            SecurityLevel::Suspicious => ("Suspicious URL", Color32::from_rgb(0xD0, 0x2B, 0x2B)),
            SecurityLevel::Local => ("Local", Color32::GRAY),
            SecurityLevel::Internal => ("", Color32::GRAY),
        };
        if !txt.is_empty() {
            ui.label(egui::RichText::new(txt).small().color(col));
        }
    }
    let id = egui::Id::new("address");
    let width = (ui.available_width() - 120.0).max(120.0);
    let resp = ui.add(egui::TextEdit::singleline(&mut st.address).id(id).hint_text("Search or enter address").desired_width(width).margin(egui::Margin::symmetric(10, 6)));
    if st.focus_address {
        resp.request_focus();
        ui.memory_mut(|m| m.request_focus(id));
        st.focus_address = false;
    }
    if resp.gained_focus() {
        if let Some(mut te) = egui::TextEdit::load_state(ui.ctx(), id) {
            te.cursor.set_char_range(Some(egui::text::CCursorRange::two(egui::text::CCursor::new(0), egui::text::CCursor::new(st.address.chars().count()))));
            te.store(ui.ctx(), id);
        }
    }
    let enter = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
    if enter {
        match b.navigate_input(&st.address) {
            Ok(()) => st.status.clear(),
            Err(e) => st.status = format!("Blocked: {e}"),
        }
    }
    // Suggestions from history (frecency-ranked) while typing.
    if resp.has_focus() && !st.address.trim().is_empty() {
        let sugg = store.suggest(&st.address, now, 6).unwrap_or_default();
        if !sugg.is_empty() {
            let pos = resp.rect.left_bottom() + Vec2::new(0.0, 4.0);
            egui::Area::new(egui::Id::new("suggest")).order(egui::Order::Foreground).fixed_pos(pos).show(ui.ctx(), |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    ui.set_min_width(resp.rect.width());
                    for s in sugg {
                        let label = if s.title.is_empty() { s.url.clone() } else { format!("{}  -  {}", s.title, s.url) };
                        if ui.selectable_label(false, label).clicked() {
                            st.address = s.url.clone();
                            let _ = b.navigate_input(&s.url);
                        }
                    }
                });
            });
        }
    }
    // Bookmark toggle.
    if let Some(u) = sel_url.filter(|u| matches!(u.scheme(), "http" | "https")) {
        let on = store.is_bookmarked(u.as_str()).unwrap_or(false);
        if ui.button(if on { "Saved" } else { "Save" }).on_hover_text("Bookmark this page").clicked() {
            if on {
                let _ = store.remove_bookmark(u.as_str());
            } else {
                let title = b.tabs.strip.selected.and_then(|t| b.tabs.strip.tab(t)).map(|t| t.display_title().to_string()).unwrap_or_default();
                let _ = store.add_bookmark(u.as_str(), &title, "");
            }
        }
    }
}

fn menu_button(ui: &mut egui::Ui, st: &mut UiState, b: &mut Browser<ServoEngine>, store: &Store) {
    ui.menu_button("...", |ui| {
        ui.label(egui::RichText::new("Inactive tabs").strong());
        let cur = b.settings.background_mode;
        for (m, label) in [
            (BackgroundMode::Normal, "Normal"),
            (BackgroundMode::SmartPause, "Smart Pause (default)"),
            (BackgroundMode::DeepSleep, "Deep Sleep"),
            (BackgroundMode::StrictOffline, "Strict Offline"),
        ] {
            if ui.radio(cur == m, label).clicked() {
                b.set_background_mode(m);
            }
        }
        // Per-site exception for the current host.
        if let Some(host) = b.tabs.strip.selected.and_then(|t| b.tabs.strip.tab(t)).and_then(|t| t.host().map(str::to_owned)) {
            ui.separator();
            ui.label(egui::RichText::new(format!("This site: {host}")).strong());
            let entry = b.settings.sites.get(&host).cloned().unwrap_or_default();
            let mut mode = entry.mode;
            let mut refresh = entry.refresh_on_return;
            let mut changed = false;
            for (m, label) in [(None, "Use default"), (Some(SiteMode::AlwaysActive), "Always active"), (Some(SiteMode::SmartPause), "Smart Pause"), (Some(SiteMode::DeepSleep), "Deep Sleep"), (Some(SiteMode::StrictOffline), "Strict Offline")] {
                if ui.radio(mode == m, label).clicked() {
                    mode = m;
                    changed = true;
                }
            }
            if ui.checkbox(&mut refresh, "Refresh content when returning to this tab").changed() {
                changed = true;
            }
            if changed {
                if mode.is_none() && !refresh {
                    b.settings.sites.remove(&host);
                } else {
                    b.settings.sites.insert(host, SiteSettings { mode, refresh_on_return: refresh });
                }
                let cmds = b.tabs.settings_changed(&b.settings);
                b.execute(cmds);
            }
        }
        ui.separator();
        ui.label(egui::RichText::new("Appearance").strong());
        let th = b.settings.appearance.theme;
        ui.horizontal(|ui| {
            for (t, l) in [(Theme::System, "System"), (Theme::Light, "Light"), (Theme::Dark, "Dark")] {
                if ui.selectable_label(th == t, l).clicked() {
                    b.settings.appearance.theme = t;
                    apply_theme(ui.ctx(), b);
                }
            }
        });
        ui.horizontal(|ui| {
            for (d, l) in [(Density::Compact, "Compact"), (Density::Comfortable, "Comfortable"), (Density::Spacious, "Spacious")] {
                if ui.selectable_label(b.settings.appearance.density == d, l).clicked() {
                    b.settings.appearance.density = d;
                }
            }
        });
        let mut rgb = b.settings.appearance.accent;
        ui.horizontal(|ui| {
            ui.label("Accent");
            if ui.color_edit_button_srgb(&mut rgb).changed() {
                b.settings.appearance.accent = rgb;
                apply_theme(ui.ctx(), b);
            }
        });
        ui.separator();
        ui.checkbox(&mut st.show_memory, "Memory and network panel");
        ui.checkbox(&mut st.show_history, "History");
        ui.checkbox(&mut st.show_bookmarks, "Bookmarks");
        if ui.button("Reopen closed tab").clicked() {
            b.reopen_closed_tab();
            ui.close();
        }
        if ui.button("Clear history").clicked() {
            let _ = store.clear_history();
            ui.close();
        }
    });
}

fn windows(ctx: &egui::Context, st: &mut UiState, b: &mut Browser<ServoEngine>, store: &Store, now: i64) {
    if st.show_memory {
        let mut open = true;
        egui::Window::new("Memory and network").open(&mut open).default_width(560.0).show(ctx, |ui| {
            match b.memory_report() {
                Some(r) => {
                    ui.label(format!("Browser memory (unique): {}   |   budget: {}   |   pressure: {:?}", r.browser_total, r.budget.global_limit, r.pressure));
                    if let Some(s) = r.system {
                        ui.label(format!("System: {} total, {} available", s.total, s.available));
                    }
                    ui.label(format!("Per-tab soft limit {} / hard limit {}", r.budget.tab_soft_limit, r.budget.tab_hard_limit));
                }
                None => {
                    ui.label("Sampling...");
                }
            }
            if let Some((a, r)) = servo_integration::allocator_stats() {
                ui.label(format!("Allocator (jemalloc): {:.0} MiB allocated, {:.0} MiB resident", a as f64 / 1048576.0, r as f64 / 1048576.0));
            }
            ui.separator();
            egui::Grid::new("tabs").striped(true).show(ui, |ui| {
                for h in ["Tab", "State", "Engine mem", "Held", "Sent/min"] {
                    ui.strong(h);
                }
                ui.end_row();
                let rows: Vec<_> = b.tabs.strip.tabs().iter().map(|t| (t.id, t.display_title().chars().take(34).collect::<String>(), t.state)).collect();
                for (id, title, state) in rows {
                    ui.label(title);
                    ui.colored_label(state_color(state, accent(b)), state.label());
                    let mem = b.memory_report().and_then(|r| r.tab_private.iter().find(|(i, _)| *i == id).map(|(_, m)| m.to_string())).unwrap_or_else(|| "-".into());
                    ui.label(mem);
                    ui.label(b.engine.held_requests(id).to_string());
                    ui.label(b.engine.requests_sent(id, Duration::from_secs(60)).to_string());
                    ui.end_row();
                }
            });
            ui.separator();
            let caps = browser_engine::Engine::caps(&b.engine);
            ui.label(egui::RichText::new(format!(
                "Engine capabilities: throttle-on-hide {} | request interception {} | stream gate {} | image release {} | script suspend {}",
                caps.hide_throttles_script, caps.request_interception, caps.pause_stream_delivery, caps.release_tab_images, caps.suspend_script
            )).small());
            if let Some(t) = b.tabs.strip.selected {
                ui.collapsing("Recent requests (selected tab)", |ui| {
                    for r in b.engine.network_log(t).iter().rev().take(25) {
                        ui.label(egui::RichText::new(format!("{:?} | {} | {}", r.disposition, r.class.label(), r.url)).small().monospace());
                    }
                });
            }
        });
        st.show_memory = open;
    }
    if st.show_history {
        let mut open = true;
        egui::Window::new("History").open(&mut open).default_width(480.0).show(ctx, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                for e in store.recent(200).unwrap_or_default() {
                    ui.horizontal(|ui| {
                        if ui.link(if e.title.is_empty() { e.url.clone() } else { e.title.clone() }).clicked() {
                            let _ = b.navigate_input(&e.url);
                        }
                        ui.label(egui::RichText::new(format!("{} ({}x)", e.url, e.visits)).small().weak());
                    });
                }
            });
        });
        st.show_history = open;
    }
    if st.show_bookmarks {
        let mut open = true;
        egui::Window::new("Bookmarks").open(&mut open).default_width(480.0).show(ctx, |ui| {
            for bm in store.bookmarks().unwrap_or_default() {
                ui.horizontal(|ui| {
                    if ui.link(if bm.title.is_empty() { bm.url.clone() } else { bm.title.clone() }).clicked() {
                        let _ = b.navigate_input(&bm.url);
                    }
                    if ui.small_button("remove").clicked() {
                        let _ = store.remove_bookmark(&bm.url);
                    }
                });
            }
        });
        st.show_bookmarks = open;
    }
    let _ = now;
}
