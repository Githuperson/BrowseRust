use crate::classify::{Decision, decide, filename_for};
use std::io::Read;
use std::time::Duration;
use url::Url;

/// Everything needed to repeat the engine's request ourselves.
#[derive(Clone, Debug)]
pub struct RequestContext {
    pub url: Url,
    /// `Cookie` header value for `url` (from the engine's cookie jar).
    pub cookie: Option<String>,
    pub referer: Option<String>,
    pub user_agent: Option<String>,
}

/// Resolves the `Cookie` header for a URL; used when following redirects so a
/// cookie is only ever sent to the origin it belongs to.
pub type CookieFn<'a> = &'a dyn Fn(&Url) -> Option<String>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProbeResult {
    Render,
    Download { url: Url, filename: String, total: Option<u64>, mime: Option<String> },
    /// Network error etc. The caller must let the engine proceed normally.
    Failed,
}

const MAX_REDIRECTS: usize = 10;

fn agent(probe: bool) -> ureq::Agent {
    let mut b = ureq::Agent::config_builder().max_redirects(0).http_status_as_error(false).timeout_connect(Some(Duration::from_secs(10))).timeout_recv_response(Some(Duration::from_secs(15)));
    if probe {
        b = b.timeout_global(Some(Duration::from_secs(12)));
    }
    b.build().into()
}

/// GET `ctx.url`, following redirects manually. Redirects are followed by us (not
/// ureq) so the `Cookie` header is recomputed for every hop and never forwarded to
/// another origin. Returns the first non-redirect response and its final URL.
pub fn follow(ctx: &RequestContext, cookies: CookieFn<'_>, probe: bool) -> Result<(ureq::http::Response<ureq::Body>, Url), String> {
    let agent = agent(probe);
    let mut url = ctx.url.clone();
    for _ in 0..=MAX_REDIRECTS {
        if !matches!(url.scheme(), "http" | "https") {
            return Err("unsupported scheme".into());
        }
        let mut req = agent.get(url.as_str());
        if let Some(c) = if url == ctx.url { ctx.cookie.clone() } else { cookies(&url) } {
            req = req.header("Cookie", &c);
        }
        if let Some(r) = &ctx.referer {
            req = req.header("Referer", r);
        }
        if let Some(ua) = &ctx.user_agent {
            req = req.header("User-Agent", ua);
        }
        let resp = req.call().map_err(|e| e.to_string())?;
        let status = resp.status().as_u16();
        if matches!(status, 301 | 302 | 303 | 307 | 308) {
            let loc = resp.headers().get("location").and_then(|v| v.to_str().ok()).ok_or("redirect without Location")?;
            url = url.join(loc).map_err(|e| e.to_string())?;
            continue;
        }
        return Ok((resp, url));
    }
    Err("too many redirects".into())
}

fn header<'a>(r: &'a ureq::http::Response<ureq::Body>, name: &str) -> Option<&'a str> {
    r.headers().get(name).and_then(|v| v.to_str().ok())
}

/// Fetch only the headers and classify the response. The connection is dropped
/// without reading the body.
pub fn probe(ctx: &RequestContext, cookies: CookieFn<'_>) -> ProbeResult {
    let Ok((resp, final_url)) = follow(ctx, cookies, true) else { return ProbeResult::Failed };
    let status = resp.status().as_u16();
    match decide(status, header(&resp, "content-type"), header(&resp, "content-disposition")) {
        Decision::Render => ProbeResult::Render,
        Decision::Download { filename } => ProbeResult::Download {
            filename: filename_for(&final_url, filename.as_deref()),
            total: header(&resp, "content-length").and_then(|v| v.parse().ok()),
            mime: header(&resp, "content-type").map(|s| s.to_string()),
            url: final_url,
        },
    }
}

/// Stream a response body into `out`, calling `progress(bytes_so_far)` per chunk.
/// Stops early (returning `Ok(false)`) when `should_cancel()` is true.
pub fn copy_body(resp: ureq::http::Response<ureq::Body>, out: &mut impl std::io::Write, mut progress: impl FnMut(u64), should_cancel: impl Fn() -> bool) -> std::io::Result<bool> {
    let (_, body) = resp.into_parts();
    let mut reader = body.into_reader();
    let mut buf = vec![0u8; 64 * 1024];
    let mut total = 0u64;
    loop {
        if should_cancel() {
            return Ok(false);
        }
        let n = reader.read(&mut buf)?;
        if n == 0 {
            return Ok(true);
        }
        out.write_all(&buf[..n])?;
        total += n as u64;
        progress(total);
    }
}
