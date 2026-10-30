use browser_security::sanitize_filename;
use url::Url;

/// `(is_attachment, filename)` from a Content-Disposition header value.
pub fn parse_content_disposition(v: &str) -> (bool, Option<String>) {
    let mut parts = v.split(';').map(str::trim);
    let kind = parts.next().unwrap_or("").to_ascii_lowercase();
    let mut plain = None;
    let mut ext = None;
    for p in parts {
        let Some((k, val)) = p.split_once('=') else { continue };
        let k = k.trim().to_ascii_lowercase();
        let val = val.trim();
        if k == "filename*" {
            // RFC 5987: charset'lang'percent-encoded
            if let Some(enc) = val.splitn(3, '\'').nth(2) {
                ext = Some(percent_decode(enc));
            }
        } else if k == "filename" {
            plain = Some(val.trim_matches('"').to_string());
        }
    }
    (kind == "attachment", ext.or(plain))
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 3 <= b.len() {
            if let Some(v) = std::str::from_utf8(&b[i + 1..i + 3]).ok().and_then(|h| u8::from_str_radix(h, 16).ok()) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Can the engine display this MIME type itself?
pub fn renderable_mime(mime: &str) -> bool {
    let m = mime.split(';').next().unwrap_or("").trim().to_ascii_lowercase();
    m.starts_with("text/")
        || m.starts_with("image/")
        || m.starts_with("audio/")
        || m.starts_with("video/")
        || matches!(
            m.as_str(),
            "application/xhtml+xml" | "application/xml" | "application/json" | "application/javascript" | "application/x-javascript" | "application/ld+json" | "application/rss+xml" | "application/atom+xml" | "application/wasm" | "application/manifest+json"
        )
        || m.ends_with("+xml")
        || m.ends_with("+json")
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Decision {
    Render,
    Download { filename: Option<String> },
}

/// Decide from response headers. Error responses and redirects always render
/// (so error pages show), as does a response with no Content-Type.
pub fn decide(status: u16, content_type: Option<&str>, content_disposition: Option<&str>) -> Decision {
    if !(200..300).contains(&status) || status == 204 || status == 205 {
        return Decision::Render;
    }
    if let Some(cd) = content_disposition {
        let (attachment, name) = parse_content_disposition(cd);
        if attachment {
            return Decision::Download { filename: name };
        }
    }
    match content_type {
        Some(ct) if !renderable_mime(ct) => Decision::Download { filename: content_disposition.and_then(|c| parse_content_disposition(c).1) },
        _ => Decision::Render,
    }
}

/// A safe file name: Content-Disposition name, else last URL path segment, else "download".
pub fn filename_for(url: &Url, from_header: Option<&str>) -> String {
    let raw = from_header
        .map(str::to_string)
        .or_else(|| url.path_segments().and_then(|mut s| s.next_back()).filter(|s| !s.is_empty()).map(percent_decode))
        .unwrap_or_default();
    sanitize_filename(&raw)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_disposition_variants() {
        assert_eq!(parse_content_disposition("attachment; filename=\"a b.zip\""), (true, Some("a b.zip".into())));
        assert_eq!(parse_content_disposition("inline; filename=x.pdf"), (false, Some("x.pdf".into())));
        assert_eq!(parse_content_disposition("attachment; filename=\"fallback.txt\"; filename*=UTF-8''na%C3%AFve%20file.txt"), (true, Some("naïve file.txt".into())));
        assert_eq!(parse_content_disposition("attachment"), (true, None));
    }

    #[test]
    fn percent_decoding_never_panics_on_hostile_input() {
        for s in ["%", "%2", "%zz", "%é%é", "a%C3", "%%%%", "é%4"] {
            let _ = percent_decode(s);
        }
        assert_eq!(percent_decode("a%20b%e"), "a b%e");
    }

    #[test]
    fn decisions() {
        assert_eq!(decide(200, Some("text/html; charset=utf-8"), None), Decision::Render);
        assert_eq!(decide(200, Some("image/png"), None), Decision::Render);
        assert_eq!(decide(200, Some("application/octet-stream"), None), Decision::Download { filename: None });
        assert_eq!(decide(200, Some("application/zip"), None), Decision::Download { filename: None });
        assert_eq!(decide(200, Some("application/pdf"), None), Decision::Download { filename: None }); // Servo cannot show PDFs
        assert_eq!(decide(200, Some("text/html"), Some("attachment; filename=r.html")), Decision::Download { filename: Some("r.html".into()) });
        assert_eq!(decide(404, Some("application/zip"), None), Decision::Render, "error pages render");
        assert_eq!(decide(302, None, None), Decision::Render);
        assert_eq!(decide(204, None, None), Decision::Render);
        assert_eq!(decide(200, None, None), Decision::Render);
        assert_eq!(decide(200, Some("application/vnd.api+json"), None), Decision::Render);
    }

    #[test]
    fn filenames_are_sanitised() {
        let u = Url::parse("https://a.test/files/report%20final.zip?x=1").unwrap();
        assert_eq!(filename_for(&u, None), "report final.zip");
        assert_eq!(filename_for(&u, Some("../../evil.sh")), "evil.sh");
        assert_eq!(filename_for(&Url::parse("https://a.test/").unwrap(), None), "download");
    }
}
