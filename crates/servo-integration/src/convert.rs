//! Servo request → engine-neutral [`RequestInfo`].

use browser_network::{Destination, RequestInfo};
use servo::WebResourceRequest;

pub fn map_destination(d: content_security_policy::Destination) -> Destination {
    use content_security_policy::Destination as D;
    match d {
        D::Document | D::Frame | D::IFrame | D::Embed | D::Object => Destination::Document,
        D::Script => Destination::Script,
        D::Style | D::Xslt => Destination::Style,
        D::Image => Destination::Image,
        D::Font => Destination::Font,
        D::Audio => Destination::Audio,
        D::Video => Destination::Video,
        D::Track => Destination::Track,
        D::Worker | D::SharedWorker | D::ServiceWorker | D::AudioWorklet | D::PaintWorklet => Destination::Worker,
        D::Manifest => Destination::Manifest,
        D::None => Destination::Empty,
        D::Json | D::Text | D::Report | D::WebIdentity => Destination::Other,
    }
}

fn header(r: &WebResourceRequest, name: &str) -> Option<String> {
    r.headers.get(name).and_then(|v| v.to_str().ok()).map(str::to_owned)
}

pub fn request_info(r: &WebResourceRequest) -> RequestInfo {
    let mut i = RequestInfo::new(r.method.as_str(), r.url.clone(), map_destination(r.destination));
    i.is_main_frame = r.is_for_main_frame;
    i.is_redirect = r.is_redirect;
    i.accept = header(r, "accept");
    i.purpose = header(r, "sec-purpose").or_else(|| header(r, "purpose"));
    i.upgrade = header(r, "upgrade");
    // `user_initiated` and `is_download` are deliberately NOT derived from request
    // headers: they come from the browser's own records (see NetworkHub).
    i
}
