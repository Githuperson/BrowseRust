//! BrowseRust shell: winit window + egui interface + Servo content.
//!
//! Composition (same technique as servoshell): Servo paints into an
//! `OffscreenRenderingContext`; egui draws the interface into the window's GL
//! context and a paint callback blits the web content into the reserved rectangle.

mod input;
mod ui;

use browser_app::Browser;
use browser_core::{Clock, SystemClock, TabId};
use browser_memory::probe::platform_probe;
use browser_security::PopupBlocker;
use browser_session::SessionStore;
use browser_settings::Settings;
use browser_storage::Store;
use egui::Rect;
use servo::{EventLoopWaker, OffscreenRenderingContext, RenderingContext, ServoBuilder, WindowRenderingContext};
use servo_integration::{ServoEngine, ServoEngineConfig};
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};
use url::Url;
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalSize};
use winit::event::{ElementState, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::keyboard::{Key, ModifiersState, NamedKey};
use winit::raw_window_handle::{HasDisplayHandle, HasWindowHandle};
use winit::window::{Window, WindowId};

#[derive(Default)]
struct Args {
    url: Option<String>,
    profile: Option<PathBuf>,
    private: bool,
    screenshot: Option<PathBuf>,
    exit_after: Option<f32>,
}

fn parse_args() -> Args {
    let mut a = Args::default();
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--profile" => a.profile = it.next().map(PathBuf::from),
            "--private" => a.private = true,
            "--screenshot" => a.screenshot = it.next().map(PathBuf::from),
            "--exit-after" => a.exit_after = it.next().and_then(|s| s.parse().ok()),
            other if !other.starts_with("--") => a.url = Some(other.to_string()),
            _ => {}
        }
    }
    a
}

fn data_dir(args: &Args) -> PathBuf {
    args.profile.clone().unwrap_or_else(|| {
        let base = std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
            .unwrap_or_else(std::env::temp_dir);
        base.join("browserust")
    })
}

#[derive(Clone)]
struct Waker(EventLoopProxy<()>);
impl EventLoopWaker for Waker {
    fn clone_box(&self) -> Box<dyn EventLoopWaker> {
        Box::new(self.clone())
    }
    fn wake(&self) {
        let _ = self.0.send_event(());
    }
}

struct Running {
    window: Window,
    egui_ctx: egui::Context,
    egui_winit: egui_winit::State,
    painter: egui_glow::Painter,
    window_ctx: Rc<WindowRenderingContext>,
    offscreen: Rc<OffscreenRenderingContext>,
    browser: Browser<ServoEngine>,
    ui: ui::UiState,
    store: Store,
    session: Option<SessionStore>,
    settings_path: Option<PathBuf>,
    mods: ModifiersState,
    cursor: (f32, f32),
    content_px: Option<(f32, f32, f32, f32)>, // x, y, w, h in physical pixels
    content_size: PhysicalSize<u32>,
    recorded: HashMap<TabId, String>,
    started: Instant,
    last_session_save: Instant,
    next_redraw: Instant,
    popups: Rc<RefCell<PopupBlocker>>,
    screenshot: Option<PathBuf>,
    exit_after: Option<Duration>,
}

struct App {
    args: Args,
    proxy: EventLoopProxy<()>,
    state: Option<Running>,
}

fn unix_now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

impl App {
    fn init(&mut self, el: &ActiveEventLoop) -> Running {
        let args = &self.args;
        let dir = data_dir(args);
        let window = el
            .create_window(Window::default_attributes().with_title("BrowseRust").with_inner_size(LogicalSize::new(1280.0, 820.0)))
            .expect("window");
        let size = window.inner_size();
        let window_ctx = Rc::new(
            WindowRenderingContext::new(el.display_handle().expect("display"), window.window_handle().expect("window handle"), size)
                .expect("rendering context"),
        );
        window_ctx.make_current().ok();
        let offscreen = Rc::new(window_ctx.offscreen_context(size));
        let servo = ServoBuilder::default().event_loop_waker(Box::new(Waker(self.proxy.clone()))).build();

        let clock: Arc<dyn Clock> = Arc::new(SystemClock);
        let popups = Rc::new(RefCell::new(PopupBlocker::new(clock.clone())));
        let gate = popups.clone();
        let proxy = self.proxy.clone();
        let cfg = ServoEngineConfig::new(
            window.scale_factor() as f32,
            Box::new(move |tab| gate.borrow_mut().allow_popup(tab)),
            Box::new(move || {
                let _ = proxy.send_event(());
            }),
        );
        let engine = ServoEngine::new(servo, offscreen.clone() as Rc<dyn RenderingContext>, clock.clone(), cfg);

        let settings_path = (!args.private).then(|| dir.join("settings.json"));
        let settings = settings_path.as_deref().and_then(|p| Settings::load(p).ok()).unwrap_or_default();
        let mut browser = Browser::new(engine, settings, clock, platform_probe(), popups.clone());

        let store = if args.private { Store::in_memory() } else { Store::open(&dir.join("data.db")) }.expect("storage");
        let session = (!args.private).then(|| SessionStore::new(&dir));
        let crashed = session.as_ref().is_some_and(|s| s.begin_run().unwrap_or(false));

        match (&args.url, &session) {
            (Some(u), _) => {
                let url = browser_app::resolve_input(u, &browser.settings.search_template).map(|r| r.url().clone());
                browser.open_tab(url, true);
            }
            (None, Some(s)) if browser.restore_session(s) > 0 => {}
            _ => {
                browser.open_tab(Url::parse(&browser.settings.home_page.clone()).ok(), true);
            }
        }

        let egui_ctx = egui::Context::default();
        ui::apply_theme(&egui_ctx, &browser);
        let egui_winit = egui_winit::State::new(egui_ctx.clone(), egui::ViewportId::ROOT, el, Some(window.scale_factor() as f32), window.theme(), None);
        let painter = egui_glow::Painter::new(window_ctx.glow_gl_api(), "", None, false).expect("egui painter");
        let mut ui_state = ui::UiState::default();
        if crashed {
            ui_state.status = "The browser did not close cleanly last time; your tabs were restored.".into();
        }
        ui_state.focus_address = args.url.is_none();

        Running {
            window,
            egui_ctx,
            egui_winit,
            painter,
            window_ctx,
            offscreen,
            browser,
            ui: ui_state,
            store,
            session,
            settings_path,
            mods: ModifiersState::empty(),
            cursor: (0.0, 0.0),
            content_px: None,
            content_size: size,
            recorded: HashMap::new(),
            started: Instant::now(),
            last_session_save: Instant::now(),
            next_redraw: Instant::now(),
            popups,
            screenshot: args.screenshot.clone(),
            exit_after: args.exit_after.map(Duration::from_secs_f32),
        }
    }
}

impl Running {
    fn shutdown(&mut self) {
        if let Some(s) = &self.session {
            let _ = self.browser.save_session(s);
            let _ = s.end_run();
        }
        if let Some(p) = &self.settings_path {
            let _ = self.browser.settings.save(p);
        }
    }

    fn in_content(&self) -> Option<(f32, f32)> {
        let (x, y, w, h) = self.content_px?;
        let (cx, cy) = self.cursor;
        (cx >= x && cy >= y && cx < x + w && cy < y + h).then_some((cx - x, cy - y))
    }

    fn shortcut(&mut self, key: &Key) -> bool {
        let ctrl = self.mods.control_key() || self.mods.super_key();
        let alt = self.mods.alt_key();
        let shift = self.mods.shift_key();
        let sel = self.browser.tabs.strip.selected;
        let ch = if let Key::Character(c) = key { c.to_lowercase() } else { String::new() };
        match () {
            _ if ctrl && !shift && ch == "t" => {
                let home = self.browser.settings.home_page.clone();
                self.browser.open_tab(Url::parse(&home).ok(), true);
                self.ui.focus_address = true;
            }
            _ if ctrl && shift && ch == "t" => self.browser.reopen_closed_tab(),
            _ if ctrl && ch == "w" => {
                if let Some(t) = sel {
                    self.browser.close_tab(t)
                }
            }
            _ if ctrl && ch == "l" => self.ui.focus_address = true,
            _ if ctrl && ch == "r" => self.browser.reload(),
            _ if ctrl && matches!(key, Key::Named(NamedKey::Tab)) => {
                let ids: Vec<TabId> = self.browser.tabs.strip.visible().map(|t| t.id).collect();
                if let Some(i) = ids.iter().position(|t| Some(*t) == sel) {
                    let n = if shift { (i + ids.len() - 1) % ids.len() } else { (i + 1) % ids.len() };
                    self.browser.select_tab(ids[n]);
                }
            }
            _ if alt && matches!(key, Key::Named(NamedKey::ArrowLeft)) => self.browser.go_back(),
            _ if alt && matches!(key, Key::Named(NamedKey::ArrowRight)) => self.browser.go_forward(),
            _ => return false,
        }
        true
    }

    fn redraw(&mut self, el: &ActiveEventLoop) {
        let now = Instant::now();
        self.browser.engine.spin();
        self.browser.tick();

        // Record visits once a page has finished loading.
        let unix = unix_now();
        let finished: Vec<(TabId, String, String)> = self
            .browser
            .tabs
            .strip
            .tabs()
            .iter()
            .filter(|t| !t.loading)
            .filter_map(|t| t.url.as_ref().map(|u| (t.id, u.to_string(), t.title.clone())))
            .filter(|(id, u, _)| self.recorded.get(id) != Some(u))
            .collect();
        for (id, u, title) in finished {
            let _ = self.store.record_visit(&u, &title, unix);
            self.recorded.insert(id, u);
        }

        if self.session.is_some() && now.duration_since(self.last_session_save) > Duration::from_secs(20) {
            self.last_session_save = now;
            if let Some(s) = &self.session {
                let _ = self.browser.save_session(s);
            }
        }

        // 1. Servo paints the selected tab into its offscreen framebuffer.
        let _ = self.window_ctx.make_current();
        if let Some(wv) = self.browser.tabs.strip.selected.and_then(|t| self.browser.engine.webview(t)) {
            wv.paint();
        }
        let blit = self.offscreen.render_to_parent_callback().map(Arc::new);

        // 2. egui lays out the interface and reserves the content rectangle.
        let raw = self.egui_winit.take_egui_input(&self.window);
        let ppp = self.egui_ctx.pixels_per_point();
        let mut content = Rect::NOTHING;
        let out = {
            let (ctx, browser, st, store) = (&self.egui_ctx, &mut self.browser, &mut self.ui, &self.store);
            ctx.run_ui(raw, |ui| {
                content = ui::show(ui, st, browser, store, unix, |painter, rect| {
                    if let Some(blit) = blit.clone() {
                        let cb = egui_glow::CallbackFn::new(move |info, p| {
                            let vp = info.viewport_in_pixels();
                            let r = euclid::default::Rect::new(
                                euclid::default::Point2D::new(vp.left_px, vp.from_bottom_px),
                                euclid::default::Size2D::new(vp.width_px, vp.height_px),
                            );
                            blit(p.gl(), r);
                        });
                        painter.add(egui::PaintCallback { rect, callback: Arc::new(cb) });
                    }
                });
            })
        };
        self.egui_winit.handle_platform_output(&self.window, out.platform_output);

        // 3. Keep the web content the size of its rectangle.
        if content.width() > 1.0 && content.height() > 1.0 {
            self.content_px = Some((content.min.x * ppp, content.min.y * ppp, content.width() * ppp, content.height() * ppp));
            let want = PhysicalSize::new((content.width() * ppp) as u32, (content.height() * ppp) as u32);
            if want != self.content_size && want.width > 0 && want.height > 0 {
                self.content_size = want;
                if let Some(wv) = self.browser.tabs.strip.selected.and_then(|t| self.browser.engine.webview(t)) {
                    wv.resize(want);
                } else {
                    self.offscreen.resize(want);
                }
            }
        }

        // 4. Paint interface + blit content, then present.
        let size = self.window.inner_size();
        self.window_ctx.prepare_for_rendering();
        let prims = self.egui_ctx.tessellate(out.shapes, out.pixels_per_point);
        let mut textures = out.textures_delta;
        self.painter.paint_and_update_textures([size.width, size.height], out.pixels_per_point, &prims, &mut textures);

        let elapsed = self.started.elapsed();
        if let (Some(path), Some(after)) = (&self.screenshot, self.exit_after) {
            if elapsed >= after {
                let rect = servo::DeviceIntRect::from_size(servo::DeviceIntSize::new(size.width as i32, size.height as i32));
                if let Some(img) = self.window_ctx.read_to_image(rect) {
                    match img.save(path) {
                        Ok(()) => println!("screenshot saved to {}", path.display()),
                        Err(e) => eprintln!("screenshot failed: {e}"),
                    }
                }
                self.shutdown();
                el.exit();
                return;
            }
        } else if let Some(after) = self.exit_after {
            if elapsed >= after {
                self.shutdown();
                el.exit();
                return;
            }
        }
        self.window_ctx.present();

        let animating = self.browser.tabs.strip.selected.and_then(|t| self.browser.engine.webview(t)).is_some_and(|w| w.animating());
        let ui_delay = out.viewport_output.get(&egui::ViewportId::ROOT).map(|v| v.repaint_delay).unwrap_or(Duration::from_secs(1));
        let wait = if animating { Duration::from_millis(16) } else { ui_delay.min(Duration::from_millis(250)) };
        self.next_redraw = Instant::now() + wait;
    }
}

impl ApplicationHandler<()> for App {
    fn resumed(&mut self, el: &ActiveEventLoop) {
        if self.state.is_none() {
            let st = self.init(el);
            self.state = Some(st);
        }
    }

    fn user_event(&mut self, _el: &ActiveEventLoop, _e: ()) {
        if let Some(s) = &mut self.state {
            s.next_redraw = Instant::now();
            s.window.request_redraw();
        }
    }

    fn about_to_wait(&mut self, el: &ActiveEventLoop) {
        if let Some(s) = &self.state {
            if Instant::now() >= s.next_redraw {
                s.window.request_redraw();
            }
            el.set_control_flow(ControlFlow::WaitUntil(s.next_redraw.max(Instant::now() + Duration::from_millis(5))));
        }
    }

    fn window_event(&mut self, el: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        let Some(s) = &mut self.state else { return };
        let resp = s.egui_winit.on_window_event(&s.window, &event);
        if resp.repaint {
            s.next_redraw = Instant::now();
            s.window.request_redraw();
        }
        match &event {
            WindowEvent::CloseRequested => {
                s.shutdown();
                el.exit();
            }
            WindowEvent::Resized(sz) if sz.width > 0 && sz.height > 0 => {
                s.window_ctx.resize(*sz);
            }
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => s.browser.engine.set_hidpi(*scale_factor as f32),
            WindowEvent::ModifiersChanged(m) => s.mods = m.state(),
            WindowEvent::RedrawRequested => s.redraw(el),
            WindowEvent::CursorMoved { position, .. } => {
                s.cursor = (position.x as f32, position.y as f32);
                if let (Some((x, y)), Some(wv)) = (s.in_content(), s.browser.tabs.strip.selected.and_then(|t| s.browser.engine.webview(t))) {
                    wv.notify_input_event(input::mouse_move(x, y));
                }
            }
            WindowEvent::MouseInput { state, button, .. } if !resp.consumed => {
                if let (Some((x, y)), Some(tab)) = (s.in_content(), s.browser.tabs.strip.selected) {
                    if *state == ElementState::Pressed {
                        s.popups.borrow_mut().note_user_input(tab);
                    }
                    if let Some(wv) = s.browser.engine.webview(tab) {
                        wv.notify_input_event(input::mouse_button(*button, *state, x, y));
                    }
                }
            }
            WindowEvent::MouseWheel { delta, .. } if !resp.consumed => {
                if let (Some((x, y)), Some(wv)) = (s.in_content(), s.browser.tabs.strip.selected.and_then(|t| s.browser.engine.webview(t))) {
                    let d = match delta {
                        MouseScrollDelta::LineDelta(a, b) => MouseScrollDelta::LineDelta(*a, *b),
                        MouseScrollDelta::PixelDelta(p) => MouseScrollDelta::PixelDelta(*p),
                    };
                    wv.notify_input_event(input::wheel(d, x, y));
                }
            }
            WindowEvent::KeyboardInput { event: ke, .. } => {
                if ke.state == ElementState::Pressed && s.shortcut(&ke.logical_key) {
                    s.window.request_redraw();
                    return;
                }
                let typing_in_ui = s.egui_ctx.egui_wants_keyboard_input();
                if !typing_in_ui {
                    if let (Some(tab), Some(ev)) = (s.browser.tabs.strip.selected, input::keyboard(ke, s.mods)) {
                        if ke.state == ElementState::Pressed {
                            s.popups.borrow_mut().note_user_input(tab);
                        }
                        if let Some(wv) = s.browser.engine.webview(tab) {
                            wv.notify_input_event(ev);
                        }
                    }
                }
            }
            _ => {}
        }
    }
}

fn main() {
    rustls::crypto::aws_lc_rs::default_provider().install_default().ok();
    let event_loop = EventLoop::<()>::with_user_event().build().expect("event loop");
    let mut app = App { args: parse_args(), proxy: event_loop.create_proxy(), state: None };
    event_loop.run_app(&mut app).expect("run");
}
