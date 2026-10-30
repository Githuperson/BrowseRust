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
use browser_security::{NavVerdict, check_navigation};
use servo::{
    CreateNewWebViewRequest, LoadStatus, NavigationRequest, PermissionRequest, RenderingContext, WebResourceLoad, WebResourceResponse,
    WebView, WebViewDelegate,
};
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, VecDeque};
use std::rc::Rc;
use url::Url;

pub struct Shared {
    pub hub: RefCell<NetworkHub<WebResourceLoad>>,
    pub events: RefCell<VecDeque<EngineEvent>>,
    /// WebViews created by `window.open`, waiting to be adopted by the engine.
    pub adopted: RefCell<Vec<(TabId, WebView)>>,
    /// The one URL the browser itself asked each tab to load (needed so a typed
    /// `file:` URL is allowed while a page-initiated one is not).
    pub expected: RefCell<HashMap<TabId, Url>>,
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

    pub fn push(&self, e: EngineEvent) {
        self.events.borrow_mut().push_back(e);
    }
}

pub fn cancel(h: WebResourceLoad) {
    let url = h.request.url.clone();
    h.intercept(WebResourceResponse::new(url)).cancel();
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

    fn notify_load_status_changed(&self, _wv: WebView, status: LoadStatus) {
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

    fn request_permission(&self, _wv: WebView, req: PermissionRequest) {
        // Secure default: nothing is granted until a permission prompt exists.
        // Decision::Ask is treated as Deny (see docs/SECURITY.md).
        tracing::info!(tab = %self.tab, feature = ?req.feature(), "permission denied by default");
        req.deny();
    }

    fn load_web_resource(&self, _wv: WebView, load: WebResourceLoad) {
        let info = convert::request_info(load.request());
        let outcomes = self.shared.hub.borrow_mut().on_request(self.tab, info, load);
        self.shared.settle(outcomes);
    }
}
