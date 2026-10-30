use url::Url;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NavVerdict {
    Allow,
    Block(&'static str),
}

/// Decide whether a navigation may proceed.
///
/// * `javascript:` is never allowed as a navigation target (pasting it into the
///   address bar is the classic self-XSS vector).
/// * `file:` and `data:` top-level navigations only when the user typed them;
///   a web page may not send the user there.
/// * Only http(s), about:blank and blob (same-page) are allowed from content.
pub fn check_navigation(url: &Url, user_initiated: bool, from_web_content: bool) -> NavVerdict {
    match url.scheme() {
        "http" | "https" => NavVerdict::Allow,
        "about" if url.as_str() == "about:blank" => NavVerdict::Allow,
        "about" if user_initiated && !from_web_content => NavVerdict::Allow,
        "blob" => NavVerdict::Allow,
        "javascript" => NavVerdict::Block("javascript: navigations are blocked"),
        "file" | "data" if user_initiated && !from_web_content => NavVerdict::Allow,
        "file" | "data" => NavVerdict::Block("web pages may not navigate to local or data URLs"),
        "view-source" if user_initiated => NavVerdict::Allow,
        _ => NavVerdict::Block("unsupported scheme"),
    }
}

/// What the address-bar lock indicator shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SecurityLevel {
    /// HTTPS with a valid certificate.
    Secure,
    /// Plain HTTP.
    NotSecure,
    /// Browser-internal page.
    Internal,
    /// Local file or data URL.
    Local,
    /// URL embeds `user:pass@` credentials — a common spoofing trick.
    Suspicious,
}

pub fn security_level(url: &Url) -> SecurityLevel {
    if !url.username().is_empty() || url.password().is_some() {
        return SecurityLevel::Suspicious;
    }
    match url.scheme() {
        "https" | "wss" => SecurityLevel::Secure,
        "http" | "ws" => {
            // Loopback is treated as a secure context by browsers, but we still
            // do not show a lock for it.
            if is_loopback(url) { SecurityLevel::Local } else { SecurityLevel::NotSecure }
        }
        "about" => SecurityLevel::Internal,
        "file" | "data" | "blob" => SecurityLevel::Local,
        _ => SecurityLevel::NotSecure,
    }
}

fn is_loopback(url: &Url) -> bool {
    match url.host() {
        Some(url::Host::Domain(d)) => d == "localhost" || d.ends_with(".localhost"),
        Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
        Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn u(s: &str) -> Url {
        Url::parse(s).unwrap()
    }

    #[test]
    fn javascript_and_unknown_schemes_are_blocked_even_when_typed() {
        assert!(matches!(check_navigation(&u("javascript:alert(1)"), true, false), NavVerdict::Block(_)));
        assert!(matches!(check_navigation(&u("ftp://x.test/"), true, false), NavVerdict::Block(_)));
        assert!(matches!(check_navigation(&u("chrome://settings"), true, false), NavVerdict::Block(_)));
    }

    #[test]
    fn web_content_cannot_send_the_user_to_file_or_data() {
        assert!(matches!(check_navigation(&u("file:///etc/passwd"), false, true), NavVerdict::Block(_)));
        assert!(matches!(check_navigation(&u("data:text/html,hi"), false, true), NavVerdict::Block(_)));
        assert_eq!(check_navigation(&u("file:///tmp/a.html"), true, false), NavVerdict::Allow);
        // Even if the page claims to be user-initiated, from_web_content wins.
        assert!(matches!(check_navigation(&u("file:///etc/passwd"), true, true), NavVerdict::Block(_)));
    }

    #[test]
    fn http_https_blank_allowed() {
        for s in ["http://a.test/", "https://a.test/", "about:blank"] {
            assert_eq!(check_navigation(&u(s), false, true), NavVerdict::Allow, "{s}");
        }
    }

    #[test]
    fn indicator_levels() {
        assert_eq!(security_level(&u("https://a.test/")), SecurityLevel::Secure);
        assert_eq!(security_level(&u("http://a.test/")), SecurityLevel::NotSecure);
        assert_eq!(security_level(&u("http://localhost:8080/")), SecurityLevel::Local);
        assert_eq!(security_level(&u("http://127.0.0.1/")), SecurityLevel::Local);
        assert_eq!(security_level(&u("https://paypal.com@evil.test/")), SecurityLevel::Suspicious);
        assert_eq!(security_level(&u("about:blank")), SecurityLevel::Internal);
    }
}
