use url::Url;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResourceKind {
    /// Scripts, styles, iframes, fetch/XHR, workers, fonts: can alter the page.
    Active,
    /// Images, audio, video: cannot read or alter the page.
    Passive,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MixedContent {
    NotMixed,
    /// Upgrade the request to https (passive content).
    Upgrade,
    /// Block outright (active content).
    Block,
}

/// A secure page loading an insecure subresource.
pub fn check_mixed_content(page: &Url, resource: &Url, kind: ResourceKind) -> MixedContent {
    let secure_page = matches!(page.scheme(), "https" | "wss");
    let insecure_resource = matches!(resource.scheme(), "http" | "ws");
    if !(secure_page && insecure_resource) {
        return MixedContent::NotMixed;
    }
    match kind {
        ResourceKind::Active => MixedContent::Block,
        ResourceKind::Passive => MixedContent::Upgrade,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn u(s: &str) -> Url {
        Url::parse(s).unwrap()
    }
    #[test]
    fn rules() {
        let page = u("https://a.test/");
        assert_eq!(check_mixed_content(&page, &u("http://b.test/x.js"), ResourceKind::Active), MixedContent::Block);
        assert_eq!(check_mixed_content(&page, &u("http://b.test/x.png"), ResourceKind::Passive), MixedContent::Upgrade);
        assert_eq!(check_mixed_content(&page, &u("https://b.test/x.js"), ResourceKind::Active), MixedContent::NotMixed);
        assert_eq!(check_mixed_content(&u("http://a.test/"), &u("http://b.test/x.js"), ResourceKind::Active), MixedContent::NotMixed);
        assert_eq!(check_mixed_content(&page, &u("ws://b.test/s"), ResourceKind::Active), MixedContent::Block);
    }
}
