use crate::delegate::{Shared, TabDelegate, cancel};
use crate::memory::{self, Line, ServoMemory};
use browser_core::{Clock, TabId, TabState};
use browser_engine::{Engine, EngineCaps, EngineEvent, TabMemorySample};
use browser_memory::TrimLevel;
use browser_network::{ControllerConfig, NetworkHub, NetworkPolicy};
use browser_settings::BackgroundMode;
use servo::profile_traits::mem::MemoryReportResult;
use servo::{RenderingContext, Servo, UserContentManager, UserScript, WebView, WebViewBuilder};
use servo_base::generic_channel::GenericCallback;
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet, VecDeque};
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use url::Url;

pub struct ServoEngineConfig {
    pub hidpi: f32,
    /// Consulted when a page calls `window.open`. Must only return `true` for
    /// popups caused by a real user gesture.
    pub allow_popup: Box<dyn Fn(TabId) -> bool>,
    /// Called when Servo has a new frame (schedule a redraw).
    pub on_frame: Box<dyn Fn()>,
    pub policy: NetworkPolicy,
    pub controller: ControllerConfig,
    pub memory_report_interval: Duration,
}

impl ServoEngineConfig {
    pub fn new(hidpi: f32, allow_popup: Box<dyn Fn(TabId) -> bool>, on_frame: Box<dyn Fn()>) -> Self {
        Self {
            hidpi,
            allow_popup,
            on_frame,
            policy: NetworkPolicy::default(),
            controller: ControllerConfig::default(),
            memory_report_interval: Duration::from_secs(5),
        }
    }
}

pub struct ServoEngine {
    servo: Servo,
    shared: Rc<Shared>,
    views: HashMap<TabId, WebView>,
    /// Holds the stream-gate user script; kept alive for the engine's lifetime.
    user_content: Rc<UserContentManager>,
    clock: Arc<dyn Clock>,
    memory_inbox: Arc<Mutex<Option<MemoryReportResult>>>,
    memory: ServoMemory,
    last_report_request: Cell<Option<Instant>>,
    report_interval: Duration,
}

impl ServoEngine {
    pub fn new(servo: Servo, ctx: Rc<dyn RenderingContext>, clock: Arc<dyn Clock>, cfg: ServoEngineConfig) -> Self {
        let shared = Rc::new(Shared {
            hub: RefCell::new(NetworkHub::new(clock.clone(), cfg.controller, cfg.policy)),
            events: RefCell::new(VecDeque::new()),
            adopted: RefCell::new(Vec::new()),
            expected: RefCell::new(HashMap::new()),
            paused: RefCell::new(HashSet::new()),
            ctx,
            hidpi: Cell::new(cfg.hidpi),
            allow_popup: cfg.allow_popup,
            on_frame: cfg.on_frame,
        });
        let user_content = Rc::new(UserContentManager::new(&servo));
        user_content.add_script(Rc::new(UserScript::new(STREAM_GATE_JS.to_string(), None)));
        Self {
            servo,
            shared,
            user_content,
            views: HashMap::new(),
            clock,
            memory_inbox: Arc::default(),
            memory: ServoMemory::default(),
            last_report_request: Cell::new(None),
            report_interval: cfg.memory_report_interval,
        }
    }

    pub fn servo(&self) -> &Servo {
        &self.servo
    }

    pub fn webview(&self, tab: TabId) -> Option<&WebView> {
        self.views.get(&tab)
    }

    /// Run Servo's event loop once. Call from the host event loop.
    pub fn spin(&self) {
        self.servo.spin_event_loop();
    }

    pub fn set_hidpi(&self, scale: f32) {
        self.shared.hidpi.set(scale);
        for v in self.views.values() {
            v.set_hidpi_scale_factor(euclid::Scale::new(scale));
        }
    }

    pub fn held_requests(&self, tab: TabId) -> usize {
        self.shared.hub.borrow().held_count(tab)
    }

    /// Requests that actually left the browser for `tab` in the last `window`.
    pub fn requests_sent(&self, tab: TabId, window: Duration) -> usize {
        self.shared.hub.borrow().monitor(tab).map_or(0, |m| m.sent_within(window))
    }

    pub fn network_log(&self, tab: TabId) -> Vec<browser_network::RequestRecord> {
        self.shared.hub.borrow().monitor(tab).map(|m| m.recent().cloned().collect()).unwrap_or_default()
    }

    pub fn servo_memory(&self) -> &ServoMemory {
        &self.memory
    }

    fn settle_resumed(&self, r: browser_network::Resumed<servo::WebResourceLoad>) {
        for h in r.release {
            drop(h);
        }
        for (h, _) in r.cancel {
            cancel(h);
        }
    }

    fn adopt_popups(&mut self) {
        let adopted: Vec<_> = self.shared.adopted.borrow_mut().drain(..).collect();
        for (tab, wv) in adopted {
            self.views.insert(tab, wv);
        }
    }

    fn request_memory_report(&self) {
        let now = self.clock.now();
        if self.last_report_request.get().is_some_and(|t| now.saturating_duration_since(t) < self.report_interval) {
            return;
        }
        self.last_report_request.set(Some(now));
        let inbox = self.memory_inbox.clone();
        match GenericCallback::new(move |res| {
            if let Ok(r) = res {
                *inbox.lock().expect("memory inbox poisoned") = Some(r);
            }
        }) {
            Ok(cb) => self.servo.create_memory_report(cb),
            Err(e) => tracing::warn!(?e, "could not create memory report callback"),
        }
    }

    fn ingest_memory_report(&mut self) {
        let Some(r) = self.memory_inbox.lock().expect("memory inbox poisoned").take() else { return };
        use servo::profile_traits::mem::ReportKind;
        let lines = r.results.iter().flat_map(|p| {
            p.reports.iter().map(|rep| Line {
                path: &rep.path,
                size: rep.size as u64,
                explicit: !matches!(rep.kind, ReportKind::NonExplicitSize),
            })
        });
        self.memory = memory::attribute(r.results.len(), lines);
    }

    fn eval(&self, tab: TabId, script: &str, cb: impl FnOnce(Result<servo::JSValue, servo::JavaScriptEvaluationError>) + 'static) {
        if let Some(v) = self.views.get(&tab) {
            v.evaluate_javascript(script, cb);
        }
    }
}

/// Document-start script that gates WebSocket/EventSource delivery (see stream_gate.js).
const STREAM_GATE_JS: &str = include_str!("stream_gate.js");

const PAGE_STATE_JS: &str = r#"(function(){var d=false;try{var els=document.querySelectorAll('input,textarea,select');
for(var i=0;i<els.length&&!d;i++){var e=els[i];var t=(e.type||'').toLowerCase();if(t==='password'||t==='hidden'||t==='file')continue;
if(e.tagName==='SELECT'){for(var j=0;j<e.options.length;j++){if(e.options[j].selected!==e.options[j].defaultSelected){d=true;break;}}}
else if(t==='checkbox'||t==='radio'){if(e.checked!==e.defaultChecked)d=true;}
else if(e.value!==e.defaultValue){d=true;}}}catch(_){}
return JSON.stringify({x:window.scrollX||0,y:window.scrollY||0,d:d});})()"#;

impl Engine for ServoEngine {
    fn caps(&self) -> EngineCaps {
        EngineCaps {
            // Verified in servo-paint/servo-constellation/servo-script 0.7.0: hide() →
            // SetWebViewHidden → pipelines SetThrottled(true) → timers slowed, rAF skipped.
            hide_throttles_script: true,
            request_interception: true,
            // Servo exposes no per-WebView image-cache release API.
            release_tab_images: false,
            // Memory reports are per process, attributed to tabs by document URL.
            per_tab_memory: true,
            // No embedder hook exists in Servo 0.7.0, so delivery is gated by an injected
            // user script (stream_gate.js) with a bounded queue. Main-thread sockets only.
            pause_stream_delivery: true,
            // Throttling only; no embedder API suspends script outright.
            suspend_script: false,
        }
    }

    fn create_tab(&mut self, tab: TabId, url: Option<&Url>) {
        {
            let mut hub = self.shared.hub.borrow_mut();
            hub.add_tab(tab);
            if url.is_some() {
                hub.begin_user_navigation(tab);
            }
        }
        if let Some(u) = url {
            self.shared.expected.borrow_mut().insert(tab, u.clone());
        }
        let mut b = WebViewBuilder::new(&self.servo, self.shared.ctx.clone())
            .hidpi_scale_factor(euclid::Scale::new(self.shared.hidpi.get()))
            .user_content_manager(self.user_content.clone())
            .delegate(Rc::new(TabDelegate { tab, shared: self.shared.clone() }));
        if let Some(u) = url {
            b = b.url(u.clone());
        }
        self.views.insert(tab, b.build());
    }

    fn destroy_tab(&mut self, tab: TabId) {
        // Dropping the last handle sends CloseWebView to the constellation and
        // removes the WebView from the painter: this is what frees renderer memory.
        self.views.remove(&tab);
        self.shared.expected.borrow_mut().remove(&tab);
        let outcomes = self.shared.hub.borrow_mut().remove_tab(tab);
        self.shared.settle(outcomes);
    }

    fn load(&mut self, tab: TabId, url: &Url) {
        self.shared.expected.borrow_mut().insert(tab, url.clone());
        if let Some(v) = self.views.get(&tab) {
            v.load(url.clone());
        }
    }

    fn reload(&mut self, tab: TabId) {
        if let Some(v) = self.views.get(&tab) {
            v.reload();
        }
    }

    fn stop(&mut self, tab: TabId) {
        // Servo 0.7.0 has no embedder-level stop; `window.stop()` is the DOM equivalent.
        self.eval(tab, "window.stop()", |_| {});
    }

    fn go_back(&mut self, tab: TabId) {
        if let Some(v) = self.views.get(&tab) {
            if v.can_go_back() {
                v.go_back(1);
            }
        }
    }

    fn go_forward(&mut self, tab: TabId) {
        if let Some(v) = self.views.get(&tab) {
            if v.can_go_forward() {
                v.go_forward(1);
            }
        }
    }

    fn show(&mut self, tab: TabId) {
        if let Some(v) = self.views.get(&tab) {
            v.show();
            v.set_focused(true);
        }
    }

    fn hide(&mut self, tab: TabId) {
        if let Some(v) = self.views.get(&tab) {
            v.set_focused(false);
            v.hide();
        }
    }

    fn apply_network(&mut self, tab: TabId, state: TabState, mode: BackgroundMode) {
        let resumed = self.shared.hub.borrow_mut().set_state(tab, state, mode);
        self.settle_resumed(resumed);
        // Pause stream delivery exactly when the network policy restricts the tab.
        let pause = browser_network::effective_mode(state, mode) != BackgroundMode::Normal;
        if pause {
            self.shared.paused.borrow_mut().insert(tab);
        } else {
            self.shared.paused.borrow_mut().remove(&tab);
        }
        self.eval(tab, if pause { "window.__bpPause && window.__bpPause(true)" } else { "window.__bpPause && window.__bpPause(false)" }, |_| {});
    }

    fn begin_user_navigation(&mut self, tab: TabId) {
        self.shared.hub.borrow_mut().begin_user_navigation(tab);
    }

    fn release_images(&mut self, _tab: TabId) -> bool {
        false
    }

    fn trim_caches(&mut self, level: TrimLevel) {
        if level >= TrimLevel::Moderate {
            self.servo.network_manager().clear_cache();
        }
    }

    fn tab_memory(&mut self, tab: TabId) -> Option<TabMemorySample> {
        let url = self.views.get(&tab)?.url()?;
        let bytes = self.memory.by_url.get(url.as_str()).copied()?;
        Some(TabMemorySample { private: browser_core::ByteSize(bytes), shared: browser_core::ByteSize(0) })
    }

    fn request_page_state(&mut self, tab: TabId) {
        let shared = self.shared.clone();
        self.eval(tab, PAGE_STATE_JS, move |res| {
            if let Ok(servo::JSValue::String(s)) = res {
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(&s) {
                    let f = |k: &str| v.get(k).and_then(|x| x.as_f64()).unwrap_or(0.0) as f32;
                    shared.push(EngineEvent::PageState { tab, scroll: (f("x"), f("y")), dirty_form: v.get("d").and_then(|d| d.as_bool()).unwrap_or(false) });
                }
            }
        });
    }

    fn restore_scroll(&mut self, tab: TabId, x: f32, y: f32) {
        self.eval(tab, &format!("window.scrollTo({x},{y})"), |_| {});
    }

    fn housekeeping(&mut self) {
        let expired = self.shared.hub.borrow_mut().tick();
        self.shared.settle(expired);
        self.request_memory_report();
        self.ingest_memory_report();
    }

    fn poll_events(&mut self) -> Vec<EngineEvent> {
        self.adopt_popups();
        self.shared.events.borrow_mut().drain(..).collect()
    }
}
