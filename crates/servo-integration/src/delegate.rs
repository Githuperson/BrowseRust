//! `WebViewDelegate` implementation and the state shared with [`crate::ServoEngine`].
//!
//! Servo hooks used here (all from the stable embedding API):
//! * `WebViewDelegate::load_web_resource` — per-WebView interception of every request;
//!   a held `WebResourceLoad` leaves the fetch pending, dropping it resumes the fetch
//!   (`DoNotIntercept`), `intercept(..).cancel()` fails it.
//! * `notify_*` callbacks — URL/title/load/history/crash events.
//! * `request_navigation` — navigation policy.
//! * `request_create_new` — popup (`window.open`) policy.
//! * `request_permission` — permission policy (deny by default).

use crate::convert;
use browser_core::TabId;
use browser_engine::{EngineEvent, LoadState};
use browser_network::{NetworkHub, Outcome};
use browser_security::{Decision, Feature, NavVerdict, PermissionStore, check_navigation};
use servo::{
    CreateNewWebViewRequest, LoadStatus, NavigationRequest, PermissionRequest, RenderingContext, WebResourceLoad, WebResourceResponse,
    WebView, WebViewDelegate,
};
use browser_downloads::{ProbeResult, RequestContext};
use std::cell::{Cell, RefCell};
use std::sync::mpsc::Sender;
use std::collections::{HashMap, HashSet, VecDeque};
use std::rc::Rc;
use std::sync::Arc;
use url::Url;

pub struct Shared {
    pub hub: RefCell<NetworkHub<WebResourceLoad>>,
    pub events: RefCell<VecDeque<EngineEvent>>,
    /// WebViews created by `window.open`, waiting to be adopted by the engine.
    pub adopted: RefCell<Vec<(TabId, WebView)>>,
    /// The one URL the browser itself asked each tab to load (needed so a typed
    /// `file:` URL is allowed while a page-initiated one is not).
    pub expected: RefCell<HashMap<TabId, Url>>,
    pub permissions: Rc<RefCell<PermissionStore>>,
    /// Main-frame loads held while their response headers are probed for downloads.
    pub probing: RefCell<HashMap<u64, (TabId, WebResourceLoad)>>,
    pub next_probe: Cell<u64>,
    pub downloads_enabled: bool,
    pub probe_tx: Sender<(u64, RequestContext, ProbeResult)>,
    pub cookie_query_tx: Arc<std::sync::Mutex<Sender<CookieQuery>>>,
    /// Cookie header for a URL, read from Servo's jar (main thread only).
    pub cookie_for: Box<dyn Fn(&Url) -> Option<String>>,
    pub waker: Option<Box<dyn servo::EventLoopWaker>>,
    /// Permission requests waiting for the user (`Decision::Ask`).
    pub pending_perms: RefCell<Vec<PendingPerm>>,
    pub next_perm_id: Cell<u64>,
    /// Tabs whose WebSocket/SSE delivery is currently paused.
    pub paused: RefCell<HashSet<TabId>>,
    pub ctx: Rc<dyn RenderingContext>,
    pub hidpi: Cell<f32>,
    pub allow_popup: Box<dyn Fn(TabId) -> bool>,
    pub on_frame: Box<dyn Fn()>,
}

impl Shared {
    /// Carry out the controller's verdicts on engine handles.
    pub fn settle(&self, outcomes: Vec<Outcome<WebResourceLoad>>) {
        for o in outcomes {
            match o {
                // Dropping the handle sends `DoNotIntercept`: the fetch proceeds normally.
                Outcome::Proceed(h) => drop(h),
                Outcome::Cancel(h, _reason) => cancel(h),
            }
        }
    }

    /// Hold `h` and decide on a worker thread whether it is a download.
    pub fn begin_probe(&self, tab: TabId, h: WebResourceLoad) {
        let req = h.request();
        let ctx = RequestContext {
            url: req.url.clone(),
            cookie: (self.cookie_for)(&req.url),
            referer: req.referrer_url.as_ref().map(|u| u.to_string()),
            user_agent: req.headers.get("user-agent").and_then(|v| v.to_str().ok()).map(str::to_owned),
        };
        let id = self.next_probe.get() + 1;
        self.next_probe.set(id);
        self.probing.borrow_mut().insert(id, (tab, h));
        let tx = self.probe_tx.clone();
        let qtx = self.cookie_query_tx.clone();
        let waker = self.waker.as_ref().map(|w| w.clone_box());
        let done_waker = self.waker.as_ref().map(|w| w.clone_box());
        std::thread::spawn(move || {
            let cookies = move |u: &Url| -> Option<String> {
                let (rtx, rrx) = std::sync::mpsc::channel();
                qtx.lock().ok()?.send((u.clone(), rtx)).ok()?;
                if let Some(w) = &waker {
                    w.wake();
                }
                rrx.recv_timeout(std::time::Duration::from_secs(2)).ok().flatten()
            };
            let result = browser_downloads::probe(&ctx, &cookies);
            let _ = tx.send((id, ctx, result));
            if let Some(w) = done_waker {
                w.wake(); // let the host spin Servo now instead of at the next idle redraw
            }
        });
    }

    pub fn push(&self, e: EngineEvent) {
        self.events.borrow_mut().push_back(e);
    }
}

pub fn cancel(h: WebResourceLoad) {
    let url = h.request.url.clone();
    h.intercept(WebResourceResponse::new(url)).cancel();
}

/// A worker thread asking the main thread for the cookies of a URL.
pub type CookieQuery = (Url, Sender<Option<String>>);

pub struct PendingPerm {
    pub id: u64,
    pub tab: TabId,
    pub origin: String,
    pub feature: Feature,
    pub label: String,
    pub request: PermissionRequest,
}

pub fn map_feature(f: servo::PermissionFeature) -> Feature {
    use servo::PermissionFeature as P;
    match f {
        P::Geolocation => Feature::Geolocation,
        P::Notifications | P::Push => Feature::Notifications,
        P::Camera => Feature::Camera,
        P::Microphone => Feature::Microphone,
        P::Midi => Feature::Midi,
        P::Bluetooth => Feature::Bluetooth,
        P::PersistentStorage => Feature::PersistentStorage,
        _ => Feature::Other,
    }
}

pub struct TabDelegate {
    pub tab: TabId,
    pub shared: Rc<Shared>,
}

impl WebViewDelegate for TabDelegate {
    fn notify_url_changed(&self, _wv: WebView, url: Url) {
        self.shared.push(EngineEvent::UrlChanged(self.tab, url));
    }

    fn notify_page_title_changed(&self, _wv: WebView, title: Option<String>) {
        self.shared.push(EngineEvent::TitleChanged(self.tab, title));
    }

    fn notify_load_status_changed(&self, wv: WebView, status: LoadStatus) {
        // A new document in a paused tab starts unpaused: re-arm the stream gate.
        if status == LoadStatus::HeadParsed && self.shared.paused.borrow().contains(&self.tab) {
            wv.evaluate_javascript("window.__bpPause && window.__bpPause(true)", |_| {});
        }
        let st = match status {
            LoadStatus::Started => LoadState::Started,
            LoadStatus::HeadParsed => LoadState::HeadParsed,
            LoadStatus::Complete => LoadState::Complete,
        };
        if st == LoadState::Complete {
            self.shared.hub.borrow_mut().load_complete(self.tab);
            self.shared.expected.borrow_mut().remove(&self.tab);
        }
        self.shared.push(EngineEvent::Load(self.tab, st));
    }

    fn notify_favicon_changed(&self, _wv: WebView) {
        self.shared.push(EngineEvent::FaviconChanged(self.tab));
    }

    fn notify_history_changed(&self, _wv: WebView, entries: Vec<Url>, current: usize) {
        self.shared.push(EngineEvent::HistoryChanged { tab: self.tab, entries, current });
    }

    fn notify_new_frame_ready(&self, _wv: WebView) {
        (self.shared.on_frame)();
    }

    fn notify_crashed(&self, _wv: WebView, reason: String, _backtrace: Option<String>) {
        self.shared.push(EngineEvent::Crashed(self.tab, reason));
    }

    fn notify_fullscreen_state_changed(&self, _wv: WebView, fullscreen: bool) {
        self.shared.push(EngineEvent::FullscreenChanged(self.tab, fullscreen));
    }

    fn request_navigation(&self, _wv: WebView, req: NavigationRequest) {
        // A browser-initiated load of exactly this URL is allowed through; anything
        // else is page-initiated and goes through the strict policy.
        let browser_asked = self.shared.expected.borrow().get(&self.tab).is_some_and(|u| *u == req.url);
        let verdict = if browser_asked { NavVerdict::Allow } else { check_navigation(&req.url, false, true) };
        match verdict {
            NavVerdict::Allow => req.allow(),
            NavVerdict::Block(why) => {
                tracing::warn!(url = %req.url, why, "navigation blocked");
                req.deny();
            }
        }
    }

    fn request_create_new(&self, parent: WebView, req: CreateNewWebViewRequest) {
        let _ = parent;
        if !(self.shared.allow_popup)(self.tab) {
            tracing::info!(tab = %self.tab, "popup blocked (no recent user gesture)");
            return; // dropping the request answers "no new webview"
        }
        let new_tab = TabId::next();
        {
            let mut hub = self.shared.hub.borrow_mut();
            hub.add_tab(new_tab);
            hub.begin_user_navigation(new_tab);
        }
        let wv = req
            .builder(self.shared.ctx.clone())
            .hidpi_scale_factor(euclid::Scale::new(self.shared.hidpi.get()))
            .delegate(Rc::new(TabDelegate { tab: new_tab, shared: self.shared.clone() }))
            .build();
        self.shared.adopted.borrow_mut().push((new_tab, wv));
        self.shared.push(EngineEvent::TabOpenedByPage { tab: new_tab, from: self.tab });
    }

    fn request_permission(&self, wv: WebView, req: PermissionRequest) {
        let feature = map_feature(req.feature());
        let origin = wv.url().map(|u| u.origin().ascii_serialization()).unwrap_or_default();
        // Opaque/unknown origins can never hold a stored grant.
        let decision = if origin.is_empty() || origin == "null" { Decision::Deny } else { self.shared.permissions.borrow().get(&origin, feature) };
        match decision {
            Decision::Allow => req.allow(),
            Decision::Deny => req.deny(),
            // No stored answer: hold the request (the page's promise stays pending)
            // until the user decides in the shell's permission bar.
            Decision::Ask => {
                let id = self.shared.next_perm_id.get() + 1;
                self.shared.next_perm_id.set(id);
                let label = format!("{:?}", req.feature());
                self.shared.pending_perms.borrow_mut().push(PendingPerm { id, tab: self.tab, origin, feature, label, request: req });
            }
        }
    }

    fn load_web_resource(&self, _wv: WebView, load: WebResourceLoad) {
        let info = convert::request_info(load.request());
        let outcomes = self.shared.hub.borrow_mut().on_request(self.tab, info, load);
        if !self.shared.downloads_enabled {
            return self.shared.settle(outcomes);
        }
        // A user-visible main-frame GET that the policy lets through is first probed:
        // if the response is a download we take it over instead of letting Servo
        // (which cannot download) try to render a binary.
        let mut rest = Vec::new();
        for o in outcomes {
            match o {
                Outcome::Proceed(h) if is_probe_candidate(&h) => self.shared.begin_probe(self.tab, h),
                other => rest.push(other),
            }
        }
        self.shared.settle(rest);
    }
}

fn is_probe_candidate(h: &WebResourceLoad) -> bool {
    let r = h.request();
    r.is_for_main_frame && !r.is_redirect && r.method == http::Method::GET && matches!(r.url.scheme(), "http" | "https")
}
