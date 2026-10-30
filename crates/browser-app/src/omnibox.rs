use url::Url;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Resolved {
    Navigate(Url),
    Search(Url),
}

impl Resolved {
    pub fn url(&self) -> &Url {
        match self {
            Resolved::Navigate(u) | Resolved::Search(u) => u,
        }
    }
}

/// Turn what the user typed into a URL. `search_template` contains `{}` where the
/// percent-encoded query goes.
///
/// Rules, in order: explicit scheme → use it; `localhost`/IP/`host.tld[:port][/path]`
/// without spaces → navigate (https, except http for localhost/IPs); else search.
pub fn resolve_input(input: &str, search_template: &str) -> Option<Resolved> {
    let t = input.trim();
    if t.is_empty() {
        return None;
    }
    let has_scheme = t.split_once(':').is_some_and(|(s, rest)| {
        !s.is_empty()
            && s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
            && s.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
            // `host:8080`, `host:8080/path`, `host:8080?q` are a host and port, not a scheme.
            && !looks_like_port(rest)
            && !t.contains(' ')
    });
    if has_scheme {
        if let Ok(u) = Url::parse(t) {
            return Some(Resolved::Navigate(u));
        }
    }
    if !t.contains(char::is_whitespace) {
        if let Some(u) = host_like(t) {
            return Some(Resolved::Navigate(u));
        }
    }
    let q: String = url::form_urlencoded::byte_serialize(t.as_bytes()).collect();
    Url::parse(&search_template.replacen("{}", &q, 1)).ok().map(Resolved::Search)
}

fn looks_like_port(rest: &str) -> bool {
    let digits = rest.chars().take_while(char::is_ascii_digit).count();
    digits > 0 && rest[digits..].chars().next().is_none_or(|c| matches!(c, '/' | '?' | '#'))
}

fn host_like(t: &str) -> Option<Url> {
    let host_part = t.split(['/', '?', '#']).next()?;
    let host_no_port = host_part.rsplit_once(':').filter(|(_, p)| p.chars().all(|c| c.is_ascii_digit()) && !p.is_empty()).map_or(host_part, |(h, _)| h);
    let local = host_no_port == "localhost" || host_no_port.ends_with(".localhost");
    let ip = host_no_port.parse::<std::net::Ipv4Addr>().is_ok() || (host_no_port.starts_with('[') && host_no_port.ends_with(']'));
    let dotted = host_no_port.contains('.')
        && !host_no_port.starts_with('.')
        && !host_no_port.ends_with('.')
        && host_no_port.chars().all(|c| c.is_alphanumeric() || matches!(c, '.' | '-'))
        && host_no_port.rsplit('.').next().is_some_and(|tld| tld.len() >= 2 && tld.chars().any(|c| c.is_alphabetic()) || host_no_port.parse::<std::net::Ipv4Addr>().is_ok());
    if !(local || ip || dotted) {
        return None;
    }
    let scheme = if local || ip { "http" } else { "https" };
    Url::parse(&format!("{scheme}://{t}")).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    const T: &str = "https://duckduckgo.com/?q={}";

    fn nav(s: &str) -> String {
        match resolve_input(s, T).unwrap() {
            Resolved::Navigate(u) => u.to_string(),
            Resolved::Search(u) => format!("SEARCH {u}"),
        }
    }

    #[test]
    fn explicit_scheme_and_bare_hosts() {
        assert_eq!(nav("https://example.com/a?b=1"), "https://example.com/a?b=1");
        assert_eq!(nav("example.com"), "https://example.com/");
        assert_eq!(nav("example.com/path?x=1"), "https://example.com/path?x=1");
        assert_eq!(nav("sub.example.co.uk:8443/x"), "https://sub.example.co.uk:8443/x");
        assert_eq!(nav("localhost:3000"), "http://localhost:3000/");
        assert_eq!(nav("127.0.0.1:8080/a"), "http://127.0.0.1:8080/a");
        assert_eq!(nav("about:blank"), "about:blank");
    }

    #[test]
    fn everything_else_is_a_search_with_encoding() {
        assert_eq!(nav("rust lifetimes"), "SEARCH https://duckduckgo.com/?q=rust+lifetimes");
        assert_eq!(nav("what is 1.5 + 2"), "SEARCH https://duckduckgo.com/?q=what+is+1.5+%2B+2");
        assert_eq!(nav("cats"), "SEARCH https://duckduckgo.com/?q=cats");
        assert_eq!(nav("v1.2"), "SEARCH https://duckduckgo.com/?q=v1.2", "numeric 'tld' is not a host");
    }

    #[test]
    fn javascript_is_passed_through_for_the_policy_layer_to_block() {
        // The resolver does not decide safety; `check_navigation` does.
        assert!(matches!(resolve_input("javascript:alert(1)", T), Some(Resolved::Navigate(u)) if u.scheme() == "javascript"));
    }

    #[test]
    fn blank_input_resolves_to_nothing() {
        assert!(resolve_input("   ", T).is_none());
    }
}
