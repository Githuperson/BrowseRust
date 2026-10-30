use crate::request::{Destination, RequestInfo};
use browser_core::{Clock, TabId};
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RequestClass {
    /// A top-level navigation the *user* started.
    EssentialNavigation,
    /// A document load nobody asked for: meta-refresh, scripted `location=`, iframes…
    Document,
    /// Page sub-resources: scripts, styles, images, fonts.
    Subresource,
    /// `fetch()` / XHR calls.
    Api,
    /// An `Api` request repeating at a regular interval.
    BackgroundPolling,
    Analytics,
    Prefetch,
    /// WebSocket handshakes and `text/event-stream` requests.
    Streaming,
    Media,
    /// A download the user started.
    Download,
}

impl RequestClass {
    pub const ALL: [RequestClass; 10] = [
        Self::EssentialNavigation,
        Self::Document,
        Self::Subresource,
        Self::Api,
        Self::BackgroundPolling,
        Self::Analytics,
        Self::Prefetch,
        Self::Streaming,
        Self::Media,
        Self::Download,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::EssentialNavigation => "navigation",
            Self::Document => "document",
            Self::Subresource => "subresource",
            Self::Api => "api",
            Self::BackgroundPolling => "polling",
            Self::Analytics => "analytics",
            Self::Prefetch => "prefetch",
            Self::Streaming => "streaming",
            Self::Media => "media",
            Self::Download => "download",
        }
    }
}

/// Matches well-known telemetry endpoints by host suffix. The list is data, not
/// logic: users can extend it.
#[derive(Clone, Debug)]
pub struct AnalyticsMatcher {
    host_suffixes: Vec<String>,
}

impl Default for AnalyticsMatcher {
    fn default() -> Self {
        let defaults = [
            "google-analytics.com",
            "analytics.google.com",
            "googletagmanager.com",
            "doubleclick.net",
            "googlesyndication.com",
            "segment.io",
            "segment.com",
            "mixpanel.com",
            "amplitude.com",
            "hotjar.com",
            "fullstory.com",
            "clarity.ms",
            "plausible.io",
            "matomo.cloud",
            "sentry.io",
            "datadoghq.com",
            "newrelic.com",
            "nr-data.net",
            "scorecardresearch.com",
            "facebook.net",
            "connect.facebook.com",
        ];
        Self { host_suffixes: defaults.iter().map(|s| s.to_string()).collect() }
    }
}

impl AnalyticsMatcher {
    pub fn add_suffix(&mut self, s: &str) {
        self.host_suffixes.push(s.to_ascii_lowercase());
    }

    pub fn matches(&self, info: &RequestInfo) -> bool {
        let Some(host) = info.url.host_str() else { return false };
        let host = host.to_ascii_lowercase();
        if self.host_suffixes.iter().any(|s| host == *s || host.ends_with(&format!(".{s}"))) {
            return true;
        }
        // First-party beacons: tiny POSTs to conventional paths.
        let path = info.url.path();
        info.method == "POST"
            && ["/collect", "/beacon", "/analytics", "/telemetry", "/track", "/log"].iter().any(|p| path.ends_with(p))
    }
}

/// Detects `Api` requests that repeat on a steady cadence.
///
/// Keyed by `(tab, method, origin+path)`; query strings are ignored because
/// pollers commonly append a cache-busting parameter.
pub struct PollingDetector {
    clock: Arc<dyn Clock>,
    history: HashMap<(TabId, String), VecDeque<Instant>>,
}

impl PollingDetector {
    const WINDOW: usize = 4;
    const MIN_INTERVAL: Duration = Duration::from_millis(500);
    const MAX_INTERVAL: Duration = Duration::from_secs(15 * 60);

    pub fn new(clock: Arc<dyn Clock>) -> Self {
        Self { clock, history: HashMap::new() }
    }

    pub fn key(info: &RequestInfo) -> String {
        format!(
            "{} {}://{}{}",
            info.method,
            info.url.scheme(),
            info.url.host_str().unwrap_or(""),
            info.url.path()
        )
    }

    /// Record a request and report whether the key now looks like polling:
    /// at least three occurrences whose gaps all lie within ±35 % of their mean.
    pub fn observe(&mut self, tab: TabId, info: &RequestInfo) -> bool {
        let now = self.clock.now();
        let h = self.history.entry((tab, Self::key(info))).or_default();
        h.push_back(now);
        while h.len() > Self::WINDOW {
            h.pop_front();
        }
        if h.len() < 3 {
            return false;
        }
        let gaps: Vec<f64> = h.iter().zip(h.iter().skip(1)).map(|(a, b)| b.duration_since(*a).as_secs_f64()).collect();
        let mean = gaps.iter().sum::<f64>() / gaps.len() as f64;
        mean >= Self::MIN_INTERVAL.as_secs_f64()
            && mean <= Self::MAX_INTERVAL.as_secs_f64()
            && gaps.iter().all(|g| (g - mean).abs() <= 0.35 * mean)
    }

    pub fn forget_tab(&mut self, tab: TabId) {
        self.history.retain(|(t, _), _| *t != tab);
    }

    /// Drop histories that have not been touched for a while (bounded memory).
    pub fn gc(&mut self) {
        let now = self.clock.now();
        self.history.retain(|_, h| h.back().is_some_and(|t| now.duration_since(*t) < Self::MAX_INTERVAL));
    }
}

/// Classify a request. `is_polling` comes from [`PollingDetector::observe`].
pub fn classify(info: &RequestInfo, analytics: &AnalyticsMatcher, is_polling: bool) -> RequestClass {
    if info.is_download {
        return RequestClass::Download;
    }
    if info.is_main_frame && info.destination == Destination::Document {
        return if info.user_initiated { RequestClass::EssentialNavigation } else { RequestClass::Document };
    }
    if info.upgrade.as_deref().is_some_and(|u| u.eq_ignore_ascii_case("websocket"))
        || matches!(info.url.scheme(), "ws" | "wss")
        || info.accept.as_deref().is_some_and(|a| a.contains("text/event-stream"))
    {
        return RequestClass::Streaming;
    }
    if info.purpose.as_deref().is_some_and(|p| p.to_ascii_lowercase().contains("prefetch")) {
        return RequestClass::Prefetch;
    }
    if analytics.matches(info) {
        return RequestClass::Analytics;
    }
    match info.destination {
        Destination::Document => RequestClass::Document,
        Destination::Audio | Destination::Video | Destination::Track => RequestClass::Media,
        Destination::Script
        | Destination::Style
        | Destination::Image
        | Destination::Font
        | Destination::Worker
        | Destination::Manifest => RequestClass::Subresource,
        Destination::Empty | Destination::Other => {
            if is_polling { RequestClass::BackgroundPolling } else { RequestClass::Api }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use browser_core::ManualClock;
    use url::Url;

    fn req(m: &str, u: &str, d: Destination) -> RequestInfo {
        RequestInfo::new(m, Url::parse(u).unwrap(), d)
    }

    #[test]
    fn user_navigation_vs_automatic_document() {
        let m = AnalyticsMatcher::default();
        let mut r = req("GET", "https://a.test/", Destination::Document);
        r.is_main_frame = true;
        assert_eq!(classify(&r, &m, false), RequestClass::Document);
        r.user_initiated = true;
        assert_eq!(classify(&r, &m, false), RequestClass::EssentialNavigation);
    }

    #[test]
    fn streaming_prefetch_analytics_media_download() {
        let m = AnalyticsMatcher::default();
        let mut sse = req("GET", "https://a.test/events", Destination::Empty);
        sse.accept = Some("text/event-stream".into());
        assert_eq!(classify(&sse, &m, false), RequestClass::Streaming);
        let ws = req("GET", "wss://a.test/socket", Destination::Empty);
        assert_eq!(classify(&ws, &m, false), RequestClass::Streaming);
        let mut pf = req("GET", "https://a.test/next", Destination::Document);
        pf.purpose = Some("prefetch".into());
        assert_eq!(classify(&pf, &m, false), RequestClass::Prefetch);
        let an = req("POST", "https://www.google-analytics.com/g/collect", Destination::Empty);
        assert_eq!(classify(&an, &m, false), RequestClass::Analytics);
        let sub = req("GET", "https://stats.google-analytics.com/x.js", Destination::Script);
        assert_eq!(classify(&sub, &m, false), RequestClass::Analytics);
        let first_party = req("POST", "https://a.test/api/collect", Destination::Empty);
        assert_eq!(classify(&first_party, &m, false), RequestClass::Analytics);
        assert_eq!(classify(&req("GET", "https://a.test/v.mp4", Destination::Video), &m, false), RequestClass::Media);
        let mut dl = req("GET", "https://a.test/f.zip", Destination::Document);
        dl.is_download = true;
        assert_eq!(classify(&dl, &m, false), RequestClass::Download);
        assert_eq!(classify(&req("GET", "https://a.test/api/x", Destination::Empty), &m, false), RequestClass::Api);
        assert_eq!(classify(&req("GET", "https://a.test/a.js", Destination::Script), &m, false), RequestClass::Subresource);
    }

    #[test]
    fn host_suffix_matching_is_not_a_substring_match() {
        let m = AnalyticsMatcher::default();
        let r = req("GET", "https://notgoogle-analytics.com/x", Destination::Empty);
        assert_eq!(classify(&r, &m, false), RequestClass::Api);
    }

    #[test]
    fn detects_regular_polling_but_not_bursts() {
        let clock = ManualClock::default();
        let mut d = PollingDetector::new(Arc::new(clock.clone()));
        let tab = TabId(1);
        let r = req("GET", "https://a.test/api/status?t=1", Destination::Empty);
        assert!(!d.observe(tab, &r));
        clock.advance(Duration::from_secs(5));
        assert!(!d.observe(tab, &r));
        clock.advance(Duration::from_secs(5));
        assert!(d.observe(tab, &r), "3 regular hits = polling");
        // Another tab polling the same URL is independent.
        assert!(!d.observe(TabId(2), &r));
        // Irregular traffic is not polling.
        let r2 = req("GET", "https://a.test/api/search", Destination::Empty);
        for gap in [1u64, 30, 2] {
            clock.advance(Duration::from_secs(gap));
            assert!(!d.observe(tab, &r2));
        }
    }
}
