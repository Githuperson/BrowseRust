use url::Url;

/// Engine-neutral resource destination (the Fetch spec's `destination`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Destination {
    Document,
    Script,
    Style,
    Image,
    Font,
    Audio,
    Video,
    Track,
    Worker,
    Manifest,
    /// `fetch()` / `XMLHttpRequest` / `EventSource` / beacons: destination `""`.
    Empty,
    Other,
}

/// Everything the classifier needs to know about one outgoing request. The
/// engine adapter fills this in; header values are only the few we inspect.
#[derive(Clone, Debug)]
pub struct RequestInfo {
    pub method: String,
    pub url: Url,
    pub destination: Destination,
    pub is_main_frame: bool,
    pub is_redirect: bool,
    /// `Accept` request header, if present.
    pub accept: Option<String>,
    /// `Purpose` / `Sec-Purpose` / `X-Moz` header value, if present.
    pub purpose: Option<String>,
    /// `Upgrade` request header (WebSocket handshake), if present.
    pub upgrade: Option<String>,
    /// Set by the browser (never by page content) when the user started this
    /// load: typing in the address bar, clicking a link, or starting a download.
    pub user_initiated: bool,
    /// Set by the browser when the user explicitly started a download.
    pub is_download: bool,
}

impl RequestInfo {
    pub fn new(method: &str, url: Url, destination: Destination) -> Self {
        Self {
            method: method.to_ascii_uppercase(),
            url,
            destination,
            is_main_frame: false,
            is_redirect: false,
            accept: None,
            purpose: None,
            upgrade: None,
            user_initiated: false,
            is_download: false,
        }
    }

    /// GET/HEAD are idempotent; only those may be dropped without side effects.
    pub fn is_idempotent(&self) -> bool {
        matches!(self.method.as_str(), "GET" | "HEAD" | "OPTIONS")
    }
}
