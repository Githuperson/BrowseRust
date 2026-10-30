//! The native interface, drawn with egui (immediate mode, no web UI).

use crate::icons::{self, Icon};
use crate::theme::{self, Palette};
use browser_app::Browser;
use browser_core::{GroupId, TabId, TabState};
use browser_downloads::{DownloadManager, JobState};
use browser_security::{DownloadRisk, SecurityLevel, security_level};
use browser_settings::{BackgroundMode, Density, SiteMode, SiteSettings, Theme};
use browser_storage::Store;
use browser_themes::{ThemeEntry, UiColors};
use browser_tabs::TabIcon;
use egui::{Align, Align2, Color32, CornerRadius, FontId, Layout, Rect, Sense, Stroke, StrokeKind, UiBuilder, Vec2, pos2, vec2};
use servo_integration::ServoEngine;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::time::{Duration, Instant};
use url::Url;

const ZOOM_STEPS: [f32; 17] = [0.25, 0.33, 0.5, 0.67, 0.75, 0.8, 0.9, 1.0, 1.1, 1.25, 1.5, 1.75, 2.0, 2.5, 3.0, 4.0, 5.0];

#[derive(Default)]
pub struct UiState {
    pub address: String,
    pub focus_address: bool,
    pub show_memory: bool,
    pub show_history: bool,
    pub show_bookmarks: bool,
    pub downloads_open: bool,
    pub palette_open: bool,
    pub find_open: bool,
    pub toggle_fullscreen: bool,
    toast: Option<(String, Instant)>,
    palette_text: String,
    palette_sel: usize,
    find_text: String,
    find_result: Rc<RefCell<String>>,
    find_focus: bool,
    last_selected: Option<TabId>,
    last_url: Option<String>,
    tab_rects: Vec<(TabId, Rect)>,
    new_workspace: String,
    ntp_text: String,
    icons: HashMap<usize, egui::TextureHandle>,
    icons_used: HashSet<usize>,
    last_job_count: usize,
    applied_look: Option<(Theme, [u8; 3], Option<String>, u64)>,
    /// Custom themes found in the profile's themes folder.
    pub themes: Vec<ThemeEntry>,
    pub theme_gen: u64,
    pub theme_dir: std::path::PathBuf,
    pub open_themes_folder: bool,
    pub reload_themes: bool,
}

impl UiState {
    pub fn toast(&mut self, msg: impl Into<String>) {
        self.toast = Some((msg.into(), Instant::now()));
    }
}

/// Colours of the selected custom theme, if any (`None` = built-in palette).
fn custom_colors(ctx: &egui::Context, b: &Browser<ServoEngine>, st: &UiState) -> Option<UiColors> {
    let want = b.settings.appearance.custom_theme.as_ref()?;
    let entry = st.themes.iter().find(|e| &e.file == want && e.error.is_none())?;
    let system_dark = ctx.system_theme().map(|t| t == egui::Theme::Dark).unwrap_or(true);
    entry.theme.resolve(system_dark)
}

fn pal(ui: &egui::Ui, b: &Browser<ServoEngine>, st: &UiState) -> Palette {
    if let Some(c) = custom_colors(ui.ctx(), b, st) {
        return theme::Palette::from_theme(&c);
    }
    let a = b.settings.appearance.accent;
    theme::palette(ui.visuals().dark_mode, Color32::from_rgb(a[0], a[1], a[2]))
}

pub fn apply_theme(ctx: &egui::Context, b: &Browser<ServoEngine>, custom: Option<UiColors>) {
    let pref = match (&custom, b.settings.appearance.theme) {
        // A custom theme decides dark/light from its own colours.
        (Some(c), _) => if c.dark { egui::ThemePreference::Dark } else { egui::ThemePreference::Light },
        (None, Theme::Light) => egui::ThemePreference::Light,
        (None, Theme::Dark) => egui::ThemePreference::Dark,
        (None, Theme::System) => egui::ThemePreference::System,
    };
    ctx.set_theme(pref);
    let a = b.settings.appearance.accent;
    theme::apply(ctx, Color32::from_rgb(a[0], a[1], a[2]), custom.as_ref().map(theme::Palette::from_theme));
}

fn state_color(s: TabState, accent: Color32) -> Color32 {
    match s {
        TabState::Foreground => accent,
        TabState::BackgroundWarm => Color32::from_rgb(0xE5, 0xA0, 0x2B),
        TabState::BackgroundFrozen => Color32::from_rgb(0x6C, 0x8E, 0xBF),
        TabState::Discarded => Color32::from_gray(130),
    }
}

fn density_px(d: Density) -> (f32, f32) {
    match d {
        Density::Compact => (30.0, 38.0),
        Density::Comfortable => (36.0, 44.0),
        Density::Spacious => (42.0, 50.0),
    }
}

fn icon_texture(ctx: &egui::Context, st: &mut UiState, icon: &TabIcon) -> egui::TextureId {
    let key = std::sync::Arc::as_ptr(&icon.rgba) as *const u8 as usize;
    st.icons_used.insert(key);
    st.icons
        .entry(key)
        .or_insert_with(|| {
            let img = egui::ColorImage::from_rgba_unmultiplied([icon.width as usize, icon.height as usize], &icon.rgba);
            ctx.load_texture(format!("favicon-{key}"), img, egui::TextureOptions::LINEAR)
        })
        .id()
}

fn hue_color(seed: u64) -> Color32 {
    const C: [[u8; 3]; 7] = [[0x4F, 0x7C, 0xFF], [0xE5, 0x5B, 0x5B], [0x3F, 0xB9, 0x6B], [0xE5, 0xA0, 0x2B], [0xA0, 0x6B, 0xE5], [0x2B, 0xB5, 0xC9], [0xE5, 0x6B, 0xA8]];
    let c = C[(seed as usize) % C.len()];
    Color32::from_rgb(c[0], c[1], c[2])
}

// ------------------------------------------------------------------ commands

#[derive(Clone)]
enum Cmd {
    NewTab,
    CloseTab,
    ReopenTab,
    Reload,
    Back,
    Forward,
    ToggleSidebar,
    ZoomIn,
    ZoomOut,
    ZoomReset,
    Find,
    Downloads,
    History,
    Bookmarks,
    Memory,
    Fullscreen,
    SetTheme(Theme),
    SetCustomTheme(Option<String>),
    OpenThemesFolder,
    SetMode(BackgroundMode),
    DiscardOthers,
    ClearHistory,
    Switch(TabId),
}

fn command_list(b: &Browser<ServoEngine>, st: &UiState) -> Vec<(String, Cmd)> {
    let mut v: Vec<(String, Cmd)> = vec![
        ("New tab".into(), Cmd::NewTab),
        ("Close tab".into(), Cmd::CloseTab),
        ("Reopen closed tab".into(), Cmd::ReopenTab),
        ("Reload page".into(), Cmd::Reload),
        ("Go back".into(), Cmd::Back),
        ("Go forward".into(), Cmd::Forward),
        ("Toggle sidebar".into(), Cmd::ToggleSidebar),
        ("Zoom in".into(), Cmd::ZoomIn),
        ("Zoom out".into(), Cmd::ZoomOut),
        ("Reset zoom".into(), Cmd::ZoomReset),
        ("Find in page".into(), Cmd::Find),
        ("Show downloads".into(), Cmd::Downloads),
        ("Show history".into(), Cmd::History),
        ("Show bookmarks".into(), Cmd::Bookmarks),
        ("Memory and network panel".into(), Cmd::Memory),
        ("Toggle full screen".into(), Cmd::Fullscreen),
        ("Theme: light".into(), Cmd::SetTheme(Theme::Light)),
        ("Theme: dark".into(), Cmd::SetTheme(Theme::Dark)),
        ("Theme: follow system".into(), Cmd::SetTheme(Theme::System)),
        ("Inactive tabs: Normal".into(), Cmd::SetMode(BackgroundMode::Normal)),
        ("Inactive tabs: Smart Pause".into(), Cmd::SetMode(BackgroundMode::SmartPause)),
        ("Inactive tabs: Deep Sleep".into(), Cmd::SetMode(BackgroundMode::DeepSleep)),
        ("Inactive tabs: Strict Offline".into(), Cmd::SetMode(BackgroundMode::StrictOffline)),
        ("Discard all other background tabs (free memory)".into(), Cmd::DiscardOthers),
        ("Clear browsing history".into(), Cmd::ClearHistory),
    ];
    v.push(("Theme: built-in".into(), Cmd::SetCustomTheme(None)));
    v.push(("Open themes folder".into(), Cmd::OpenThemesFolder));
    for e in st.themes.iter().filter(|e| e.error.is_none()) {
        v.push((format!("Theme: {}", e.display_name()), Cmd::SetCustomTheme(Some(e.file.clone()))));
    }
    for t in b.tabs.strip.visible() {
        v.push((format!("Switch to: {}", t.display_title()), Cmd::Switch(t.id)));
    }
    v
}

/// Subsequence match score (higher = better); `None` if `q` is not a subsequence.
fn fuzzy(q: &str, s: &str) -> Option<i32> {
    let (q, s) = (q.to_lowercase(), s.to_lowercase());
    // An exact substring always beats a scattered subsequence; word-start beats mid-word.
    let exact = s.find(&q).map(|i| 1000 + if i == 0 || s.as_bytes().get(i - 1).is_some_and(|c| !c.is_ascii_alphanumeric()) { 200 } else { 0 });
    let mut score = exact.unwrap_or(0);
    let mut it = s.chars().enumerate();
    let mut prev: Option<usize> = None;
    for qc in q.chars() {
        let (i, _) = it.by_ref().find(|(_, c)| *c == qc)?;
        score += 10 - (i as i32).min(9);
        if prev.is_some_and(|p| p + 1 == i) {
            score += 6;
        }
        prev = Some(i);
    }
    Some(score)
}

fn exec(cmd: Cmd, st: &mut UiState, b: &mut Browser<ServoEngine>, store: &Store) {
    match cmd {
        Cmd::NewTab => {
            let home = b.settings.home_page.clone();
            b.open_tab(Url::parse(&home).ok(), true);
            st.focus_address = true;
        }
        Cmd::CloseTab => {
            if let Some(t) = b.tabs.strip.selected {
                b.close_tab(t)
            }
        }
        Cmd::ReopenTab => b.reopen_closed_tab(),
        Cmd::Reload => b.reload(),
        Cmd::Back => b.go_back(),
        Cmd::Forward => b.go_forward(),
        Cmd::ToggleSidebar => b.settings.appearance.sidebar_collapsed = !b.settings.appearance.sidebar_collapsed,
        Cmd::ZoomIn => zoom(b, 1),
        Cmd::ZoomOut => zoom(b, -1),
        Cmd::ZoomReset => zoom(b, 0),
        Cmd::Find => {
            st.find_open = true;
            st.find_focus = true;
        }
        Cmd::Downloads => st.downloads_open = !st.downloads_open,
        Cmd::History => st.show_history = !st.show_history,
        Cmd::Bookmarks => st.show_bookmarks = !st.show_bookmarks,
        Cmd::Memory => st.show_memory = !st.show_memory,
        Cmd::Fullscreen => st.toggle_fullscreen = true,
        Cmd::SetTheme(t) => {
            b.settings.appearance.theme = t;
            b.settings.appearance.custom_theme = None;
        }
        Cmd::SetCustomTheme(f) => b.settings.appearance.custom_theme = f,
        Cmd::OpenThemesFolder => st.open_themes_folder = true,
        Cmd::SetMode(m) => b.set_background_mode(m),
        Cmd::DiscardOthers => {
            let sel = b.tabs.strip.selected;
            let ids: Vec<TabId> = b.tabs.strip.tabs().iter().filter(|t| Some(t.id) != sel && t.state.is_background()).map(|t| t.id).collect();
            let n = ids.len();
            for id in ids {
                b.discard_tab(id);
            }
            st.toast(format!("Discarded {n} background tab(s)"));
        }
        Cmd::ClearHistory => {
            let _ = store.clear_history();
            st.toast("History cleared");
        }
        Cmd::Switch(t) => b.select_tab(t),
    }
}

/// Step the selected tab's zoom: `dir` +1/-1, or 0 to reset.
pub fn zoom(b: &mut Browser<ServoEngine>, dir: i32) {
    let Some(id) = b.tabs.strip.selected else { return };
    let cur = b.tabs.strip.tab(id).map(|t| t.zoom).unwrap_or(1.0);
    let idx = ZOOM_STEPS.iter().position(|z| (*z - cur).abs() < 0.01).unwrap_or(7) as i32;
    let new = if dir == 0 { 1.0 } else { ZOOM_STEPS[(idx + dir).clamp(0, ZOOM_STEPS.len() as i32 - 1) as usize] };
    if let Some(t) = b.tabs.strip.tab_mut(id) {
        t.zoom = new;
    }
    if let Some(wv) = b.engine.webview(id) {
        wv.set_page_zoom(new);
    }
}

const FIND_JS: &str = r#"(function(q,dir){var S=window.__bpFind=window.__bpFind||{q:'',marks:[],i:-1};
function clear(){S.marks.forEach(function(m){var p=m.parentNode;if(!p)return;p.replaceChild(document.createTextNode(m.textContent),m);p.normalize();});S.marks=[];S.i=-1;}
if(q===null){clear();S.q='';return '';}
if(q!==S.q||!S.marks.length){clear();S.q=q;
 if(q&&document.body){var w=document.createTreeWalker(document.body,NodeFilter.SHOW_TEXT,{acceptNode:function(n){var t=n.parentNode&&n.parentNode.nodeName;return(t==='SCRIPT'||t==='STYLE'||t==='NOSCRIPT'||!n.nodeValue.trim())?NodeFilter.FILTER_REJECT:NodeFilter.FILTER_ACCEPT;}});
 var nodes=[];while(w.nextNode())nodes.push(w.currentNode);var ql=q.toLowerCase();
 nodes.forEach(function(n){var txt=n.nodeValue,lo=txt.toLowerCase(),idx=lo.indexOf(ql);if(idx<0||S.marks.length>=2000)return;
  var frag=document.createDocumentFragment(),last=0;
  while(idx>=0&&S.marks.length<2000){frag.appendChild(document.createTextNode(txt.slice(last,idx)));var m=document.createElement('mark');m.style.background='#ffd54a';m.style.color='#000';m.textContent=txt.substr(idx,q.length);frag.appendChild(m);S.marks.push(m);last=idx+q.length;idx=lo.indexOf(ql,last);}
  frag.appendChild(document.createTextNode(txt.slice(last)));n.parentNode.replaceChild(frag,n);});}
 S.i=-1;}
if(!S.marks.length)return q?'0/0':'';
if(S.i>=0)S.marks[S.i].style.background='#ffd54a';
S.i=(S.i+dir+S.marks.length)%S.marks.length;var m=S.marks[S.i];m.style.background='#ff8a3d';m.scrollIntoView({block:'center'});
return (S.i+1)+'/'+S.marks.length;})"#;

fn run_find(st: &UiState, b: &Browser<ServoEngine>, dir: i32, close: bool) {
    let Some(wv) = b.tabs.strip.selected.and_then(|t| b.engine.webview(t)) else { return };
    let arg = if close { "null".to_string() } else { format!("{}", serde_json_string(&st.find_text)) };
    let out = st.find_result.clone();
    wv.evaluate_javascript(format!("{FIND_JS}({arg},{dir})"), move |r| {
        if let Ok(servo::JSValue::String(s)) = r {
            *out.borrow_mut() = s;
        }
    });
}

/// JSON string literal (so arbitrary text is safe to embed in the script).
fn serde_json_string(s: &str) -> String {
    let mut o = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"),
            '\r' => o.push_str("\\r"),
            c if (c as u32) < 0x20 => o.push_str(&format!("\\u{:04x}", c as u32)),
            c => o.push(c),
        }
    }
    o.push('"');
    o
}

fn reveal(path: &std::path::Path) {
    #[cfg(target_os = "macos")]
    let _ = std::process::Command::new("open").arg("-R").arg(path).spawn();
    #[cfg(not(target_os = "macos"))]
    {
        let dir = path.parent().unwrap_or(path);
        let _ = std::process::Command::new("xdg-open").arg(dir).spawn();
    }
}

fn fmt_bytes(n: u64) -> String {
    browser_core::ByteSize(n).to_string()
}

// ---------------------------------------------------------------------- show

/// Draw the whole UI. `paint_content` receives the rectangle reserved for web content.
pub fn show(ui: &mut egui::Ui, st: &mut UiState, b: &mut Browser<ServoEngine>, store: &Store, dl: &DownloadManager, now: i64, paint_content: impl FnOnce(&egui::Painter, Rect)) -> Rect {
    st.icons_used.clear();
    // Re-apply the egui style whenever the theme or accent changed (palette, menu, settings).
    let look = (b.settings.appearance.theme, b.settings.appearance.accent, b.settings.appearance.custom_theme.clone(), st.theme_gen);
    if st.applied_look.as_ref() != Some(&look) {
        let custom = custom_colors(ui.ctx(), b, st);
        apply_theme(ui.ctx(), b, custom);
        st.applied_look = Some(look);
    }
    let p = pal(ui, b, st);
    let (tab_h, bar_h) = density_px(b.settings.appearance.density);
    let selected = b.tabs.strip.selected;

    // Keep the address bar in sync with the page unless the user is typing.
    let cur_url = selected.and_then(|t| b.tabs.strip.tab(t)).and_then(|t| t.url.as_ref()).map(Url::to_string);
    if selected != st.last_selected || (cur_url != st.last_url && !ui.ctx().memory(|m| m.has_focus(egui::Id::new("address")))) {
        st.address = cur_url.clone().filter(|u| u != "about:blank").unwrap_or_default();
        st.last_selected = selected;
    }
    st.last_url = cur_url;

    // Keep each tab's page zoom in sync (also restores zoom after a discard/restore).
    if let Some(t) = selected.and_then(|t| b.tabs.strip.tab(t)).map(|t| (t.id, t.zoom)) {
        if let Some(wv) = b.engine.webview(t.0) {
            if (wv.page_zoom() - t.1).abs() > 0.001 {
                wv.set_page_zoom(t.1);
            }
        }
    }

    // New download started: show the popover.
    let jobs = dl.snapshot();
    if jobs.len() > st.last_job_count {
        st.downloads_open = true;
        // The page did not navigate: show its real URL again instead of the download link.
        st.address = st.last_url.clone().filter(|u| u != "about:blank").unwrap_or_default();
        if let Some(j) = jobs.last() {
            st.toast(format!("Downloading {}", j.filename));
        }
    }
    st.last_job_count = jobs.len();

    // ---------------------------------------------------------------- sidebar
    if !b.settings.appearance.sidebar_collapsed {
        egui::Panel::left("sidebar").exact_size(224.0).frame(egui::Frame::NONE.fill(p.strip)).show(ui, |ui| sidebar(ui, st, b, store, &p));
    }

    // -------------------------------------------------------------------- tabs
    egui::Panel::top("tabs").exact_size(tab_h + 10.0).frame(egui::Frame::NONE.fill(p.strip)).show(ui, |ui| {
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            ui.add_space(6.0);
            if b.settings.appearance.sidebar_collapsed && icons::button(ui, &p, Icon::Sidebar, "Show sidebar (Ctrl+B)", true, false, None).clicked() {
                b.settings.appearance.sidebar_collapsed = false;
            }
            tab_strip(ui, st, b, tab_h, &p);
            if icons::button(ui, &p, Icon::Plus, "New tab (Ctrl+T)", true, false, None).clicked() {
                exec(Cmd::NewTab, st, b, store);
            }
        });
    });

    // ----------------------------------------------------------------- toolbar
    egui::Panel::top("toolbar").exact_size(bar_h).frame(egui::Frame::NONE.fill(p.bar)).show(ui, |ui| {
        ui.horizontal_centered(|ui| {
            ui.add_space(8.0);
            let (can_back, can_fwd) = selected.and_then(|t| b.engine.webview(t)).map(|w| (w.can_go_back(), w.can_go_forward())).unwrap_or_default();
            let loading = selected.and_then(|t| b.tabs.strip.tab(t)).is_some_and(|t| t.loading);
            if icons::button(ui, &p, Icon::Back, "Back (Alt+Left)", can_back, false, None).clicked() {
                b.go_back();
            }
            if icons::button(ui, &p, Icon::Forward, "Forward (Alt+Right)", can_fwd, false, None).clicked() {
                b.go_forward();
            }
            if loading {
                if icons::button(ui, &p, Icon::Stop, "Stop", true, false, None).clicked() {
                    b.stop();
                }
            } else if icons::button(ui, &p, Icon::Reload, "Reload (Ctrl+R)", true, false, None).clicked() {
                b.reload();
            }
            if icons::button(ui, &p, Icon::Home, "Home", true, false, None).clicked() {
                let home = b.settings.home_page.clone();
                let _ = b.navigate_input(&home);
            }
            address_bar(ui, st, b, store, now, &p);
            let active = jobs.iter().filter(|j| j.state == JobState::Running).count() as u32;
            let r = icons::button(ui, &p, Icon::Download, "Downloads (Ctrl+J)", true, st.downloads_open, Some(active));
            if r.clicked() {
                st.downloads_open = !st.downloads_open;
            }
            menu_button(ui, st, b, store, &p);
            ui.add_space(6.0);
        });
    });

    // ------------------------------------------------------------- find in page
    if st.find_open {
        egui::Panel::top("find").exact_size(40.0).frame(egui::Frame::NONE.fill(p.bar)).show(ui, |ui| {
            ui.horizontal_centered(|ui| {
                ui.add_space(12.0);
                let resp = ui.add(egui::TextEdit::singleline(&mut st.find_text).id(egui::Id::new("find_text")).hint_text("Find in page").desired_width(260.0).margin(egui::Margin::symmetric(10, 5)));
                if st.find_focus {
                    resp.request_focus();
                    st.find_focus = false;
                }
                if resp.changed() {
                    run_find(st, b, 1, false);
                }
                let (enter, shift) = ui.input(|i| (i.key_pressed(egui::Key::Enter), i.modifiers.shift));
                if resp.lost_focus() && enter {
                    run_find(st, b, if shift { -1 } else { 1 }, false);
                    resp.request_focus();
                }
                ui.label(egui::RichText::new(st.find_result.borrow().clone()).color(p.subtle));
                if icons::button(ui, &p, Icon::Up, "Previous match", true, false, None).clicked() {
                    run_find(st, b, -1, false);
                }
                if icons::button(ui, &p, Icon::Down, "Next match", true, false, None).clicked() {
                    run_find(st, b, 1, false);
                }
                if icons::button(ui, &p, Icon::Close, "Close (Esc)", true, false, None).clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                    run_find(st, b, 1, true);
                    st.find_open = false;
                    st.find_result.borrow_mut().clear();
                }
            });
        });
    }

    // ---------------------------------------------------- permission prompts
    if let Some(prompt) = selected.and_then(|t| b.engine.pending_permissions().into_iter().find(|p| p.tab == t)) {
        egui::Panel::top("permission").exact_size(40.0).frame(egui::Frame::NONE.fill(p.surface)).show(ui, |ui| {
            ui.horizontal_centered(|ui| {
                ui.add_space(14.0);
                ui.label(egui::RichText::new(format!("{} wants to use: {}", prompt.origin, prompt.feature)).strong());
                let rid = egui::Id::new("perm_remember");
                let mut r = ui.memory(|m| m.data.get_temp::<bool>(rid)).unwrap_or(true);
                if ui.checkbox(&mut r, "Remember").changed() {
                    ui.memory_mut(|m| m.data.insert_temp(rid, r));
                }
                if ui.button("Allow").clicked() {
                    b.engine.answer_permission(prompt.id, true, r);
                }
                if ui.button("Block").clicked() {
                    b.engine.answer_permission(prompt.id, false, r);
                }
            });
        });
    }

    // ------------------------------------------------------------------ status
    egui::Panel::bottom("status").exact_size(22.0).frame(egui::Frame::NONE.fill(p.strip)).show(ui, |ui| {
        ui.horizontal_centered(|ui| {
            ui.add_space(10.0);
            let state = selected.and_then(|t| b.tabs.strip.tab(t)).map(|t| t.state.label()).unwrap_or("");
            ui.label(egui::RichText::new(state).small().color(p.subtle));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.add_space(10.0);
                if let Some(r) = b.memory_report() {
                    ui.label(egui::RichText::new(format!("{}  |  pressure: {:?}  |  budget {}", r.browser_total, r.pressure, r.budget.global_limit)).small().color(p.subtle));
                }
                let blocked = selected.map(|t| b.popups.borrow().blocked_count(t)).unwrap_or(0);
                if blocked > 0 {
                    ui.label(egui::RichText::new(format!("{blocked} popup(s) blocked")).small().color(Color32::from_rgb(0xE5, 0x7A, 0x2B)));
                }
            });
        });
    });

    // ----------------------------------------------------------------- content
    let mut content = Rect::NOTHING;
    let on_ntp = selected.and_then(|t| b.tabs.strip.tab(t)).is_some_and(|t| t.url.as_ref().is_none_or(|u| u.as_str() == "about:blank"));
    egui::CentralPanel::default().frame(egui::Frame::NONE).show(ui, |ui| {
        let (rect, _resp) = ui.allocate_exact_size(ui.available_size(), Sense::hover());
        content = rect;
        if on_ntp {
            ui.painter().rect_filled(rect, 0.0, p.bar);
            new_tab_page(ui, rect, st, b, store, &p);
        } else {
            paint_content(ui.painter(), rect);
        }
    });
    let used = st.icons_used.clone();
    st.icons.retain(|k, _| used.contains(k));

    // ---------------------------------------------------------------- overlays
    downloads_popover(ui.ctx(), st, dl, &p);
    command_palette(ui.ctx(), st, b, store, &p);
    windows(ui.ctx(), st, b, store, now);
    toast(ui.ctx(), st, &p);
    content
}

fn toast(ctx: &egui::Context, st: &mut UiState, p: &Palette) {
    let Some((msg, at)) = st.toast.clone() else { return };
    let age = at.elapsed().as_secs_f32();
    if age > 3.2 {
        st.toast = None;
        return;
    }
    let a = (1.0 - ((age - 2.6) / 0.6).clamp(0.0, 1.0)).clamp(0.0, 1.0);
    let screen = ctx.content_rect();
    egui::Area::new(egui::Id::new("toast")).order(egui::Order::Tooltip).fixed_pos(pos2(screen.center().x, screen.max.y - 56.0)).pivot(Align2::CENTER_BOTTOM).interactable(false).show(ctx, |ui| {
        egui::Frame::NONE.fill(p.text.gamma_multiply(0.92 * a)).corner_radius(18).inner_margin(egui::Margin::symmetric(16, 9)).show(ui, |ui| {
            ui.label(egui::RichText::new(msg).color(p.bar.gamma_multiply(a)));
        });
    });
    ctx.request_repaint_after(Duration::from_millis(30));
}

// ------------------------------------------------------------------- sidebar

fn sidebar(ui: &mut egui::Ui, st: &mut UiState, b: &mut Browser<ServoEngine>, store: &Store, p: &Palette) {
    ui.add_space(10.0);
    ui.horizontal(|ui| {
        ui.add_space(8.0);
        ui.label(egui::RichText::new("Spaces").strong().color(p.subtle).small());
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.add_space(6.0);
            if icons::button(ui, p, Icon::Sidebar, "Hide sidebar (Ctrl+B)", true, false, None).clicked() {
                b.settings.appearance.sidebar_collapsed = true;
            }
        });
    });
    let active = b.tabs.strip.active_workspace;
    let spaces: Vec<_> = b.tabs.strip.workspaces.iter().map(|w| (w.id, w.name.clone())).collect();
    for (id, name) in spaces {
        let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width() - 8.0, 32.0), Sense::click());
        if id == active || resp.hovered() {
            ui.painter().rect_filled(rect, 9.0, if id == active { p.bar } else { p.hover });
        }
        ui.painter().circle_filled(rect.left_center() + vec2(16.0, 0.0), 6.0, hue_color(id.0));
        ui.painter().text(rect.left_center() + vec2(32.0, 0.0), Align2::LEFT_CENTER, name, FontId::proportional(13.5), p.text);
        if resp.clicked() && id != active {
            b.tabs.strip.active_workspace = id;
            let first = b.tabs.strip.visible().next().map(|t| t.id);
            match first {
                Some(t) => b.select_tab(t),
                None => {
                    b.open_tab(None, true);
                }
            }
        }
    }
    ui.horizontal(|ui| {
        ui.add_space(6.0);
        ui.add(egui::TextEdit::singleline(&mut st.new_workspace).hint_text("New space").desired_width(ui.available_width() - 48.0).margin(egui::Margin::symmetric(8, 4)));
        if icons::button(ui, p, Icon::Plus, "Create space", true, false, None).clicked() && !st.new_workspace.trim().is_empty() {
            let id = b.tabs.strip.add_workspace(st.new_workspace.trim());
            b.tabs.strip.active_workspace = id;
            b.open_tab(None, true);
            st.new_workspace.clear();
        }
    });

    // Pinned tabs as tiles.
    let pinned: Vec<(TabId, String, Option<TabIcon>, TabState)> = b.tabs.strip.visible().filter(|t| t.pinned).map(|t| (t.id, t.display_title().to_string(), t.favicon.clone(), t.state)).collect();
    if !pinned.is_empty() {
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            ui.add_space(8.0);
            ui.label(egui::RichText::new("Pinned").strong().color(p.subtle).small());
        });
        let sel = b.tabs.strip.selected;
        ui.horizontal_wrapped(|ui| {
            ui.add_space(6.0);
            for (id, title, icon, state) in pinned {
                let (rect, resp) = ui.allocate_exact_size(vec2(44.0, 44.0), Sense::click());
                let fill = if sel == Some(id) { p.bar } else if resp.hovered() { p.hover } else { p.surface };
                ui.painter().rect_filled(rect, 12.0, fill);
                if sel == Some(id) {
                    ui.painter().rect_stroke(rect, 12.0, Stroke::new(1.5, p.accent), StrokeKind::Inside);
                }
                match &icon {
                    Some(ic) => {
                        let tid = icon_texture(ui.ctx(), st, ic);
                        ui.painter().image(tid, Rect::from_center_size(rect.center(), Vec2::splat(22.0)), Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
                    }
                    None => {
                        ui.painter().text(rect.center(), Align2::CENTER_CENTER, title.chars().next().unwrap_or('?').to_uppercase().to_string(), FontId::proportional(17.0), p.text);
                    }
                }
                ui.painter().circle_filled(rect.right_top() + vec2(-7.0, 7.0), 3.5, state_color(state, p.accent));
                if resp.on_hover_text(&title).clicked() {
                    b.select_tab(id);
                }
            }
        });
    }

    // Bookmarks.
    let marks = store.bookmarks().unwrap_or_default();
    if !marks.is_empty() {
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            ui.add_space(8.0);
            ui.label(egui::RichText::new("Bookmarks").strong().color(p.subtle).small());
        });
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            for m in marks.iter().take(40) {
                let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width() - 8.0, 28.0), Sense::click());
                if resp.hovered() {
                    ui.painter().rect_filled(rect, 8.0, p.hover);
                }
                let label = if m.title.is_empty() { m.url.clone() } else { m.title.clone() };
                let tr = Rect::from_min_max(rect.left_top() + vec2(12.0, 0.0), rect.right_bottom() - vec2(8.0, 0.0));
                ui.painter().with_clip_rect(tr).text(tr.left_center(), Align2::LEFT_CENTER, label, FontId::proportional(13.0), p.text);
                if resp.on_hover_text(&m.url).clicked() {
                    let _ = b.navigate_input(&m.url);
                }
            }
        });
    }
}

// ----------------------------------------------------------------- tab strip

fn tab_strip(ui: &mut egui::Ui, st: &mut UiState, b: &mut Browser<ServoEngine>, h: f32, p: &Palette) {
    let tabs: Vec<(TabId, String, TabState, bool, bool, bool, bool, Option<TabIcon>, Option<GroupId>)> = b
        .tabs
        .strip
        .visible()
        .map(|t| (t.id, t.display_title().to_string(), t.state, t.pinned, t.loading, t.muted, t.audible, t.favicon.clone(), t.group))
        .collect();
    let n = tabs.len().max(1) as f32;
    let pinned_n = tabs.iter().filter(|t| t.3).count() as f32;
    let avail = (ui.available_width() - 52.0 - pinned_n * 44.0).max(60.0);
    let w = (avail / (n - pinned_n).max(1.0)).clamp(72.0, 230.0);
    let selected = b.tabs.strip.selected;
    st.tab_rects.clear();
    let mut action: Option<TabAction> = None;
    let groups: Vec<(GroupId, String, [u8; 3])> = b.tabs.strip.groups.iter().map(|g| (g.id, g.name.clone(), g.color)).collect();
    ui.spacing_mut().item_spacing.x = 2.0;
    for (id, title, state, pinned, loading, muted, audible, icon, group) in tabs {
        let size = vec2(if pinned { 42.0 } else { w }, h + 2.0);
        let (rect, resp) = ui.allocate_exact_size(size, Sense::click_and_drag());
        st.tab_rects.push((id, rect));
        let is_sel = selected == Some(id);
        let hov = ui.ctx().animate_bool_with_time(resp.id.with("hov"), resp.hovered(), 0.12);
        let painter = ui.painter();
        if is_sel {
            // Merges with the toolbar below.
            painter.rect_filled(rect, CornerRadius { nw: 11, ne: 11, sw: 0, se: 0 }, p.bar);
            // Concave "feet" like Chrome.
            for (x, flip) in [(rect.min.x, -1.0f32), (rect.max.x, 1.0)] {
                let r = 8.0;
                let c = pos2(x + flip * r, rect.max.y - r);
                painter.add(egui::Shape::convex_polygon(vec![pos2(x, rect.max.y), pos2(x + flip * r, rect.max.y), pos2(x + flip * r, rect.max.y - r * 0.2)], p.bar, Stroke::NONE));
                let _ = c;
            }
        } else if hov > 0.0 {
            painter.rect_filled(rect.shrink2(vec2(0.0, 4.0)), 9.0, p.hover.gamma_multiply(hov));
        }
        if let Some((_, _, c)) = group.and_then(|g| groups.iter().find(|x| x.0 == g)) {
            painter.rect_filled(Rect::from_min_size(rect.left_top() + vec2(6.0, 0.0), vec2(rect.width() - 12.0, 3.0)), 2.0, Color32::from_rgb(c[0], c[1], c[2]));
        }
        let col = state_color(state, p.accent);
        let dot = rect.left_center() + vec2(11.0, 0.0);
        if state == TabState::Discarded {
            painter.circle_stroke(dot, 3.5, Stroke::new(1.4, col));
        } else {
            painter.circle_filled(dot, 3.5, col);
        }
        let mut text_x = 24.0;
        if let Some(ic) = &icon {
            let tid = icon_texture(ui.ctx(), st, ic);
            ui.painter().image(tid, Rect::from_min_size(rect.left_center() + vec2(20.0, -8.0), Vec2::splat(16.0)), Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
            text_x = 44.0;
        } else if pinned {
            ui.painter().text(rect.center() + vec2(4.0, 0.0), Align2::CENTER_CENTER, title.chars().next().unwrap_or('?').to_uppercase().to_string(), FontId::proportional(14.0), p.text);
        }
        if !pinned {
            let right_pad = 28.0 + if audible || muted { 18.0 } else { 0.0 };
            let text_rect = Rect::from_min_max(rect.left_top() + vec2(text_x, 0.0), rect.right_bottom() - vec2(right_pad, 0.0));
            let tc = if is_sel { p.text } else { p.subtle };
            ui.painter().with_clip_rect(text_rect).text(text_rect.left_center(), Align2::LEFT_CENTER, &title, FontId::proportional(13.0), tc);
            if audible || muted {
                icons::paint(ui.painter(), if muted { Icon::Muted } else { Icon::Sound }, rect.right_center() - vec2(36.0, 0.0), 13.0, p.subtle);
            }
            if loading {
                ui.put(Rect::from_center_size(rect.right_center() - vec2(if audible || muted { 54.0 } else { 36.0 }, 0.0), Vec2::splat(12.0)), egui::Spinner::new().size(12.0));
            }
            if resp.hovered() || is_sel {
                let cr = Rect::from_center_size(rect.right_center() - vec2(14.0, 0.0), Vec2::splat(20.0));
                let cresp = ui.interact(cr, resp.id.with("close"), Sense::click());
                if cresp.hovered() {
                    ui.painter().rect_filled(cr, 6.0, p.hover);
                }
                icons::paint(ui.painter(), Icon::Close, cr.center(), 10.0, p.text);
                if cresp.clicked() {
                    action = Some(TabAction::Close(id));
                }
            }
        }
        if !is_sel && !resp.hovered() {
            // faint separator
            ui.painter().line_segment([rect.right_top() + vec2(0.0, 9.0), rect.right_bottom() - vec2(0.0, 9.0)], Stroke::new(1.0, p.border.gamma_multiply(0.5)));
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
            ui.menu_button("Group", |ui| {
                if ui.button("New group").clicked() {
                    action = Some(TabAction::NewGroup(id));
                    ui.close();
                }
                for (gid, name, _) in &groups {
                    if ui.button(name).clicked() {
                        action = Some(TabAction::SetGroup(id, Some(*gid)));
                        ui.close();
                    }
                }
                if group.is_some() && ui.button("Remove from group").clicked() {
                    action = Some(TabAction::SetGroup(id, None));
                    ui.close();
                }
            });
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
            TabAction::SetGroup(t, g) => b.tabs.strip.set_group(t, g),
            TabAction::NewGroup(t) => {
                let n = b.tabs.strip.groups.len();
                let c = hue_color(n as u64);
                let g = b.tabs.strip.add_group(&format!("Group {}", n + 1), [c.r(), c.g(), c.b()]);
                b.tabs.strip.set_group(t, Some(g));
            }
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
    SetGroup(TabId, Option<GroupId>),
    NewGroup(TabId),
    Move(TabId, usize),
}

// --------------------------------------------------------------- address bar

fn address_bar(ui: &mut egui::Ui, st: &mut UiState, b: &mut Browser<ServoEngine>, store: &Store, now: i64, p: &Palette) {
    let sel = b.tabs.strip.selected;
    let sel_url = sel.and_then(|t| b.tabs.strip.tab(t)).and_then(|t| t.url.clone());
    let zoom = sel.and_then(|t| b.tabs.strip.tab(t)).map(|t| t.zoom).unwrap_or(1.0);
    let id = egui::Id::new("address");
    let focused = ui.memory(|m| m.has_focus(id));
    let width = (ui.available_width() - 76.0).max(160.0);
    let (rect, _) = ui.allocate_exact_size(vec2(width, 32.0), Sense::hover());
    ui.painter().rect_filled(rect, 16.0, p.surface);
    ui.painter().rect_stroke(rect, 16.0, Stroke::new(if focused { 1.6 } else { 1.0 }, if focused { p.accent } else { p.border }), StrokeKind::Inside);

    // Left: security chip.
    let mut left = rect.min.x + 12.0;
    if let Some(u) = &sel_url {
        let (txt, col) = match security_level(u) {
            SecurityLevel::Secure => ("Secure", Color32::from_rgb(0x2E, 0xA0, 0x4F)),
            SecurityLevel::NotSecure => ("Not secure", Color32::from_rgb(0xD9, 0x6A, 0x1E)),
            SecurityLevel::Suspicious => ("Suspicious", Color32::from_rgb(0xD0, 0x2B, 0x2B)),
            SecurityLevel::Local => ("Local", p.subtle),
            SecurityLevel::Internal => ("", p.subtle),
        };
        if !txt.is_empty() {
            let g = ui.painter().layout_no_wrap(txt.to_string(), FontId::proportional(11.5), col);
            let chip = Rect::from_min_size(pos2(left, rect.center().y - 9.0), vec2(g.size().x + 16.0, 18.0));
            ui.painter().rect_filled(chip, 9.0, col.gamma_multiply(0.16));
            ui.painter().galley(pos2(chip.min.x + 8.0, chip.center().y - g.size().y / 2.0), g.clone(), col);
            left = chip.max.x + 8.0;
        }
    }
    // Right: zoom chip + star.
    let mut right = rect.max.x - 8.0;
    if let Some(u) = sel_url.as_ref().filter(|u| matches!(u.scheme(), "http" | "https")) {
        let on = store.is_bookmarked(u.as_str()).unwrap_or(false);
        let sr = Rect::from_center_size(pos2(right - 12.0, rect.center().y), vec2(26.0, 26.0));
        let r = ui.interact(sr, id.with("star"), Sense::click());
        if r.hovered() {
            ui.painter().rect_filled(sr, 13.0, p.hover);
        }
        icons::paint(ui.painter(), if on { Icon::StarFilled } else { Icon::Star }, sr.center(), 15.0, if on { Color32::from_rgb(0xF2, 0xB7, 0x05) } else { p.subtle });
        if r.on_hover_text("Bookmark this page").clicked() {
            if on {
                let _ = store.remove_bookmark(u.as_str());
                st.toast("Bookmark removed");
            } else {
                let title = sel.and_then(|t| b.tabs.strip.tab(t)).map(|t| t.display_title().to_string()).unwrap_or_default();
                let _ = store.add_bookmark(u.as_str(), &title, "");
                st.toast("Bookmarked");
            }
        }
        right = sr.min.x - 4.0;
    }
    if (zoom - 1.0).abs() > 0.01 {
        let txt = format!("{}%", (zoom * 100.0).round() as i32);
        let g = ui.painter().layout_no_wrap(txt, FontId::proportional(11.5), p.text);
        let chip = Rect::from_min_size(pos2(right - g.size().x - 16.0, rect.center().y - 9.0), vec2(g.size().x + 16.0, 18.0));
        let r = ui.interact(chip, id.with("zoom"), Sense::click());
        ui.painter().rect_filled(chip, 9.0, if r.hovered() { p.hover } else { p.border.gamma_multiply(0.5) });
        ui.painter().galley(pos2(chip.min.x + 8.0, chip.center().y - g.size().y / 2.0), g, p.text);
        if r.on_hover_text("Reset zoom").clicked() {
            zoom_reset(b);
        }
        right = chip.min.x - 6.0;
    }

    let edit_rect = Rect::from_min_max(pos2(left, rect.min.y + 2.0), pos2(right, rect.max.y - 2.0));
    let mut child = ui.new_child(UiBuilder::new().max_rect(edit_rect).layout(Layout::left_to_right(Align::Center)));
    let resp = child.add(egui::TextEdit::singleline(&mut st.address).id(id).frame(egui::Frame::NONE).hint_text("Search or enter address").desired_width(edit_rect.width()).margin(egui::Margin::symmetric(4, 6)));
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
    if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
        match b.navigate_input(&st.address) {
            Ok(()) => {}
            Err(e) => st.toast(format!("Blocked: {e}")),
        }
    }
    if resp.has_focus() && !st.address.trim().is_empty() {
        let sugg = store.suggest(&st.address, now, 6).unwrap_or_default();
        if !sugg.is_empty() {
            egui::Area::new(egui::Id::new("suggest")).order(egui::Order::Foreground).fixed_pos(rect.left_bottom() + vec2(0.0, 6.0)).show(ui.ctx(), |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    ui.set_min_width(rect.width());
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
}

fn zoom_reset(b: &mut Browser<ServoEngine>) {
    zoom(b, 0)
}

fn menu_button(ui: &mut egui::Ui, st: &mut UiState, b: &mut Browser<ServoEngine>, store: &Store, p: &Palette) {
    let r = icons::button(ui, p, Icon::Menu, "Menu", true, false, None);
    egui::Popup::menu(&r).show(|ui| {
        ui.set_min_width(250.0);
        ui.label(egui::RichText::new("Inactive tabs").strong().color(p.subtle).small());
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
        if let Some(host) = b.tabs.strip.selected.and_then(|t| b.tabs.strip.tab(t)).and_then(|t| t.host().map(str::to_owned)) {
            ui.separator();
            ui.label(egui::RichText::new(format!("This site: {host}")).strong().color(p.subtle).small());
            let entry = b.settings.sites.get(&host).cloned().unwrap_or_default();
            let (mut mode, mut refresh, mut changed) = (entry.mode, entry.refresh_on_return, false);
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
        ui.label(egui::RichText::new("Custom theme").strong().color(p.subtle).small());
        if ui.radio(b.settings.appearance.custom_theme.is_none(), "Built-in").clicked() {
            b.settings.appearance.custom_theme = None;
        }
        let mut chosen: Option<String> = None;
        for e in &st.themes {
            let sel = b.settings.appearance.custom_theme.as_deref() == Some(e.file.as_str());
            let mut tip = String::new();
            if !e.theme.author.is_empty() {
                tip.push_str(&format!("by {}\n", e.theme.author));
            }
            if let Some(err) = &e.error {
                tip.push_str(&format!("Error: {err}\n"));
            }
            for w in e.theme.warnings.iter().take(6) {
                tip.push_str(&format!("- {w}\n"));
            }
            let label = if e.error.is_some() { format!("{} (unusable)", e.display_name()) } else { e.display_name().to_string() };
            let r = ui.add_enabled(e.error.is_none(), egui::RadioButton::new(sel, label));
            let r = if tip.is_empty() { r } else { r.on_hover_text(tip.trim_end()) };
            if r.clicked() {
                chosen = Some(e.file.clone());
            }
        }
        if let Some(f) = chosen {
            b.settings.appearance.custom_theme = Some(f);
        }
        ui.horizontal(|ui| {
            if ui.small_button("Open themes folder").clicked() {
                st.open_themes_folder = true;
            }
            if ui.small_button("Reload").clicked() {
                st.reload_themes = true;
            }
        });
        ui.separator();
        ui.label(egui::RichText::new("Appearance").strong().color(p.subtle).small());
        ui.horizontal(|ui| {
            for (t, l) in [(Theme::System, "System"), (Theme::Light, "Light"), (Theme::Dark, "Dark")] {
                if ui.selectable_label(b.settings.appearance.theme == t, l).clicked() {
                    b.settings.appearance.theme = t;
                    b.settings.appearance.custom_theme = None;
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
            ui.label("Accent colour");
            if ui.color_edit_button_srgb(&mut rgb).changed() {
                b.settings.appearance.accent = rgb;
            }
        });
        ui.separator();
        ui.horizontal(|ui| {
            if ui.button("Command palette  Ctrl+K").clicked() {
                st.palette_open = true;
                st.palette_text.clear();
                ui.close();
            }
        });
        ui.checkbox(&mut st.show_memory, "Memory and network panel");
        ui.checkbox(&mut st.show_history, "History");
        ui.checkbox(&mut st.show_bookmarks, "Bookmarks");
        if ui.button("Clear history").clicked() {
            let _ = store.clear_history();
            st.toast("History cleared");
            ui.close();
        }
    });
}

// ------------------------------------------------------------------ downloads

fn downloads_popover(ctx: &egui::Context, st: &mut UiState, dl: &DownloadManager, p: &Palette) {
    if !st.downloads_open {
        return;
    }
    let screen = ctx.content_rect();
    let area = egui::Area::new(egui::Id::new("downloads_pop")).order(egui::Order::Foreground).fixed_pos(pos2(screen.max.x - 14.0, 96.0)).pivot(Align2::RIGHT_TOP).show(ctx, |ui| {
        egui::Frame::popup(ui.style()).corner_radius(14).inner_margin(egui::Margin::same(12)).show(ui, |ui| {
            ui.set_width(380.0);
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("Downloads").strong().size(15.0));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui.small_button("Clear finished").clicked() {
                        dl.clear_finished();
                    }
                });
            });
            ui.separator();
            let jobs = dl.snapshot();
            if jobs.is_empty() {
                ui.label(egui::RichText::new(format!("Nothing downloaded yet.\nFiles are saved to {}", dl.dir().display())).color(p.subtle));
            }
            egui::ScrollArea::vertical().max_height(360.0).show(ui, |ui| {
                for j in jobs.iter().rev() {
                    ui.horizontal(|ui| {
                        ui.vertical(|ui| {
                            ui.set_width(300.0);
                            ui.label(egui::RichText::new(&j.filename).strong());
                            let status = match j.state {
                                JobState::Running => match j.total {
                                    Some(t) => format!("{} of {}  -  {}/s", fmt_bytes(j.received), fmt_bytes(t), fmt_bytes(j.bytes_per_sec as u64)),
                                    None => format!("{}  -  {}/s", fmt_bytes(j.received), fmt_bytes(j.bytes_per_sec as u64)),
                                },
                                JobState::Done => format!("Completed  -  {}", fmt_bytes(j.received)),
                                JobState::Failed => format!("Failed: {}", j.error.clone().unwrap_or_default()),
                                JobState::Cancelled => "Cancelled".into(),
                            };
                            let col = if j.state == JobState::Failed { Color32::from_rgb(0xD0, 0x2B, 0x2B) } else { p.subtle };
                            ui.label(egui::RichText::new(status).small().color(col));
                            if j.risk == DownloadRisk::Dangerous && j.state == JobState::Done {
                                ui.label(egui::RichText::new("Executable or installer: only run it if you trust the source.").small().color(Color32::from_rgb(0xE5, 0x7A, 0x2B)));
                            }
                            if j.state == JobState::Running {
                                let frac = j.total.map(|t| (j.received as f32 / t.max(1) as f32).clamp(0.0, 1.0));
                                let (r, _) = ui.allocate_exact_size(vec2(300.0, 6.0), Sense::hover());
                                ui.painter().rect_filled(r, 3.0, p.border);
                                match frac {
                                    Some(f) => {
                                        ui.painter().rect_filled(Rect::from_min_size(r.min, vec2(r.width() * f, r.height())), 3.0, p.accent);
                                    }
                                    None => {
                                        let t = ui.input(|i| i.time) as f32;
                                        let x = r.min.x + (t * 0.9 % 1.0) * (r.width() - 60.0);
                                        ui.painter().rect_filled(Rect::from_min_size(pos2(x, r.min.y), vec2(60.0, r.height())), 3.0, p.accent);
                                        ui.ctx().request_repaint();
                                    }
                                }
                                ui.ctx().request_repaint_after(Duration::from_millis(100));
                            }
                        });
                        ui.with_layout(Layout::top_down(Align::Center), |ui| match j.state {
                            JobState::Running => {
                                if icons::button(ui, p, Icon::Close, "Cancel download", true, false, None).clicked() {
                                    dl.cancel(j.id);
                                }
                            }
                            JobState::Done => {
                                if let Some(path) = &j.dest {
                                    if icons::button(ui, p, Icon::Folder, "Show in folder", true, false, None).clicked() {
                                        reveal(path);
                                    }
                                }
                            }
                            _ => {}
                        });
                    });
                    ui.separator();
                }
            });
        });
    });
    // Click outside closes it (but not the click that opened it).
    let clicked_out = ctx.input(|i| i.pointer.any_pressed()) && ctx.input(|i| i.pointer.interact_pos()).is_some_and(|pos| !area.response.rect.contains(pos) && pos.y > 90.0);
    if clicked_out || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        st.downloads_open = false;
    }
}

// ------------------------------------------------------------ command palette

fn command_palette(ctx: &egui::Context, st: &mut UiState, b: &mut Browser<ServoEngine>, store: &Store, p: &Palette) {
    if !st.palette_open {
        return;
    }
    let screen = ctx.content_rect();
    // Dim the page.
    egui::Area::new(egui::Id::new("palette_dim")).order(egui::Order::Middle).fixed_pos(screen.min).show(ctx, |ui| {
        let r = ui.allocate_rect(screen, Sense::click());
        ui.painter().rect_filled(screen, 0.0, Color32::from_black_alpha(110));
        if r.clicked() {
            st.palette_open = false;
        }
    });
    let all = command_list(b, st);
    let mut scored: Vec<(i32, usize)> = all.iter().enumerate().filter_map(|(i, (l, _))| if st.palette_text.is_empty() { Some((-(i as i32), i)) } else { fuzzy(&st.palette_text, l).map(|s| (s, i)) }).collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0));
    scored.truncate(9);
    let (up, down, enter, esc) = ctx.input(|i| (i.key_pressed(egui::Key::ArrowUp), i.key_pressed(egui::Key::ArrowDown), i.key_pressed(egui::Key::Enter), i.key_pressed(egui::Key::Escape)));
    if down {
        st.palette_sel = (st.palette_sel + 1).min(scored.len().saturating_sub(1));
    }
    if up {
        st.palette_sel = st.palette_sel.saturating_sub(1);
    }
    st.palette_sel = st.palette_sel.min(scored.len().saturating_sub(1));
    let mut chosen: Option<Cmd> = None;
    egui::Area::new(egui::Id::new("palette")).order(egui::Order::Foreground).fixed_pos(pos2(screen.center().x, 90.0)).pivot(Align2::CENTER_TOP).show(ctx, |ui| {
        egui::Frame::popup(ui.style()).corner_radius(16).inner_margin(egui::Margin::same(10)).show(ui, |ui| {
            ui.set_width(580.0);
            let r = ui.add(egui::TextEdit::singleline(&mut st.palette_text).id(egui::Id::new("palette_input")).hint_text("Type a command or tab name...").desired_width(f32::INFINITY).margin(egui::Margin::symmetric(12, 9)));
            r.request_focus();
            if r.changed() {
                st.palette_sel = 0;
            }
            ui.add_space(4.0);
            for (row, (_, i)) in scored.iter().enumerate() {
                let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::click());
                if row == st.palette_sel || resp.hovered() {
                    ui.painter().rect_filled(rect, 8.0, if row == st.palette_sel { p.accent.gamma_multiply(0.25) } else { p.hover });
                }
                ui.painter().text(rect.left_center() + vec2(12.0, 0.0), Align2::LEFT_CENTER, &all[*i].0, FontId::proportional(14.0), p.text);
                if resp.clicked() {
                    chosen = Some(all[*i].1.clone());
                }
            }
            if scored.is_empty() {
                ui.label(egui::RichText::new("No matching command").color(p.subtle));
            }
        });
    });
    if enter {
        if let Some((_, i)) = scored.get(st.palette_sel) {
            chosen = Some(all[*i].1.clone());
        }
    }
    if let Some(c) = chosen {
        st.palette_open = false;
        st.palette_text.clear();
        exec(c, st, b, store);
    } else if esc {
        st.palette_open = false;
    }
}

// ---------------------------------------------------------------- new-tab page

fn new_tab_page(ui: &mut egui::Ui, rect: Rect, st: &mut UiState, b: &mut Browser<ServoEngine>, store: &Store, p: &Palette) {
    let w = 680.0_f32.min(rect.width() - 48.0).max(200.0);
    let area = Rect::from_min_size(pos2(rect.center().x - w / 2.0, rect.min.y + 70.0), vec2(w, (rect.height() - 90.0).max(50.0)));
    let mut child = ui.new_child(UiBuilder::new().max_rect(area));
    let ui = &mut child;
    ui.vertical_centered(|ui| {
        ui.label(egui::RichText::new("BrowseRust").size(38.0).strong());
        ui.label(egui::RichText::new("Fast, light, and quiet in the background").color(p.subtle));
        ui.add_space(18.0);
        let resp = ui.add(egui::TextEdit::singleline(&mut st.ntp_text).hint_text("Search or enter address").desired_width(w * 0.85).margin(egui::Margin::symmetric(18, 12)));
        if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            let q = st.ntp_text.clone();
            if let Err(e) = b.navigate_input(&q) {
                st.toast(format!("Blocked: {e}"));
            }
            st.ntp_text.clear();
        }
    });
    ui.add_space(28.0);
    let mut go: Option<String> = None;
    let mut tiles = |ui: &mut egui::Ui, title: &str, items: Vec<(String, String)>| {
        if items.is_empty() {
            return;
        }
        ui.label(egui::RichText::new(title).strong().color(p.subtle));
        ui.add_space(6.0);
        ui.horizontal_wrapped(|ui| {
            for (label, url) in items {
                let host = Url::parse(&url).ok().and_then(|u| u.host_str().map(str::to_owned)).unwrap_or_default();
                let text = if label.is_empty() { host.clone() } else { label };
                let (r, resp) = ui.allocate_exact_size(vec2(158.0, 60.0), Sense::click());
                ui.painter().rect_filled(r, 12.0, if resp.hovered() { p.hover } else { p.surface });
                let c = r.left_center() + vec2(26.0, 0.0);
                ui.painter().circle_filled(c, 15.0, hue_color(host.len() as u64 + host.bytes().next().unwrap_or(0) as u64));
                ui.painter().text(c, Align2::CENTER_CENTER, text.chars().next().unwrap_or('?').to_uppercase().to_string(), FontId::proportional(14.0), Color32::WHITE);
                let tr = Rect::from_min_max(r.left_top() + vec2(50.0, 0.0), r.right_bottom() - vec2(8.0, 0.0));
                ui.painter().with_clip_rect(tr).text(tr.left_center(), Align2::LEFT_CENTER, text, FontId::proportional(13.0), p.text);
                if resp.clicked() {
                    go = Some(url);
                }
            }
        });
        ui.add_space(18.0);
    };
    let top: Vec<_> = store.top_sites(8).unwrap_or_default().into_iter().map(|e| (e.title, e.url)).collect();
    tiles(ui, "Most visited", top);
    let marks: Vec<_> = store.bookmarks().unwrap_or_default().into_iter().take(16).map(|m| (m.title, m.url)).collect();
    tiles(ui, "Bookmarks", marks);
    if let Some(u) = go {
        let _ = b.navigate_input(&u);
    }
}

// --------------------------------------------------------------------- windows

fn windows(ctx: &egui::Context, st: &mut UiState, b: &mut Browser<ServoEngine>, store: &Store, now: i64) {
    if st.show_memory {
        let mut open = true;
        egui::Window::new("Memory and network").open(&mut open).default_width(580.0).show(ctx, |ui| {
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
            let acc = Color32::from_rgb(b.settings.appearance.accent[0], b.settings.appearance.accent[1], b.settings.appearance.accent[2]);
            egui::Grid::new("tabs").striped(true).show(ui, |ui| {
                for h in ["Tab", "State", "Engine mem", "Held", "Sent/min"] {
                    ui.strong(h);
                }
                ui.end_row();
                let rows: Vec<_> = b.tabs.strip.tabs().iter().map(|t| (t.id, t.display_title().chars().take(34).collect::<String>(), t.state)).collect();
                for (id, title, state) in rows {
                    ui.label(title);
                    ui.colored_label(state_color(state, acc), state.label());
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
        egui::Window::new("History").open(&mut open).default_width(500.0).show(ctx, |ui| {
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
        egui::Window::new("Bookmarks").open(&mut open).default_width(500.0).show(ctx, |ui| {
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

/// Public entry for keyboard shortcuts handled by the shell.
pub fn shortcut(name: &str, st: &mut UiState, b: &mut Browser<ServoEngine>, store: &Store) {
    let c = match name {
        "palette" => {
            st.palette_open = true;
            st.palette_text.clear();
            st.palette_sel = 0;
            return;
        }
        "find" => Cmd::Find,
        "zoom_in" => Cmd::ZoomIn,
        "zoom_out" => Cmd::ZoomOut,
        "zoom_reset" => Cmd::ZoomReset,
        "sidebar" => Cmd::ToggleSidebar,
        "downloads" => Cmd::Downloads,
        "fullscreen" => Cmd::Fullscreen,
        _ => return,
    };
    exec(c, st, b, store);
}

#[cfg(test)]
mod tests {
    use super::fuzzy;

    #[test]
    fn exact_substring_outranks_a_scattered_subsequence() {
        let theme = fuzzy("dark", "Theme: dark").unwrap();
        let discard = fuzzy("dark", "Discard all other background tabs (free memory)").unwrap();
        assert!(theme > discard, "{theme} vs {discard}");
        assert!(fuzzy("zzz", "Theme: dark").is_none());
        assert!(fuzzy("new", "New tab").unwrap() > fuzzy("new", "Reopen closed tab (new)").unwrap_or(0) - 1);
    }
}

/// Open a folder in the system file manager.
pub fn reveal_dir(dir: &std::path::Path) {
    #[cfg(target_os = "macos")]
    let _ = std::process::Command::new("open").arg(dir).spawn();
    #[cfg(not(target_os = "macos"))]
    let _ = std::process::Command::new("xdg-open").arg(dir).spawn();
}
