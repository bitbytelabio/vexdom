//! Which hosts a flow may reach: the one place a URL is checked.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use super::Flow;

/// Registrable part of a host, close enough for first-party checks:
/// `api.example.com.ar` -> `example.com.ar`, `www.example.org` -> `example.org`.
pub(crate) fn base_domain(host: &str) -> String {
    let host = host.trim_end_matches('.');
    if host.parse::<IpAddr>().is_ok() || host.starts_with('[') {
        return host.to_string();
    }
    // The public suffix list keeps `a.vercel.app` and `b.vercel.app` apart.
    psl::domain_str(&host.to_ascii_lowercase())
        .map(String::from)
        .unwrap_or_else(|| host.to_ascii_lowercase())
}

pub(crate) fn host_of(url: &str) -> Option<String> {
    url::Url::parse(url).ok()?.host_str().map(String::from)
}

/// A flow origin becomes a directory name, so it must be a plain hostname.
pub(crate) fn validate_origin(origin: &str) -> Result<(), String> {
    let ok = !origin.is_empty()
        && origin.len() <= 253
        && !origin.starts_with('.')
        && !origin.ends_with('.')
        && !origin.contains("..")
        && origin
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_');
    if ok {
        Ok(())
    } else {
        Err(format!(
            "invalid flow origin '{origin}': expected a hostname"
        ))
    }
}

/// Link-local addresses and the cloud metadata endpoints a run never sends to.
fn is_link_local_or_metadata(ip: IpAddr) -> bool {
    match ip.to_canonical() {
        IpAddr::V4(v4) => v4.is_link_local() || v4 == Ipv4Addr::new(100, 100, 100, 200),
        IpAddr::V6(v6) => {
            (v6.segments()[0] & 0xffc0) == 0xfe80
                || v6 == Ipv6Addr::new(0xfd00, 0xec2, 0, 0, 0, 0, 0, 0x254)
        }
    }
}

fn normalize_host(host: &str) -> String {
    host.trim_end_matches('.').to_ascii_lowercase()
}

fn host_matches(host: &str, allowed: &str) -> bool {
    if allowed.parse::<IpAddr>().is_ok() {
        return host == allowed;
    }
    host == allowed || host.ends_with(&format!(".{allowed}"))
}

/// The hosts a flow may reach: the base domain of its start URL, which a
/// flow file can only narrow, never widen.
#[derive(Clone, Debug)]
pub(crate) struct HostPolicy {
    base: Vec<String>,
    narrowed: Vec<String>,
}

impl HostPolicy {
    /// The policy a flow recorded from `start_url` gets.
    pub(crate) fn from_start(start_url: &str) -> HostPolicy {
        HostPolicy {
            base: host_of(start_url)
                .map(|h| vec![base_domain(&h)])
                .unwrap_or_default(),
            narrowed: Vec::new(),
        }
    }

    /// The policy of a saved flow.
    pub(crate) fn of(flow: &Flow) -> HostPolicy {
        HostPolicy {
            narrowed: flow
                .allowed_hosts
                .iter()
                .map(|h| normalize_host(h))
                .collect(),
            ..HostPolicy::from_start(&flow.start_url)
        }
    }

    /// The base domains, as `allowedHosts` records them.
    pub(crate) fn hosts(&self) -> &[String] {
        &self.base
    }

    pub(crate) fn allows_host(&self, host: &str) -> bool {
        let host = normalize_host(host);
        self.base.iter().any(|a| host_matches(&host, a))
            && (self.narrowed.is_empty() || self.narrowed.iter().any(|a| host_matches(&host, a)))
    }

    pub(crate) fn allows_url(&self, url: &str) -> bool {
        host_of(url).is_some_and(|h| self.allows_host(&h))
    }

    /// Reject a URL that is not http(s) or whose host is outside the policy.
    pub(crate) fn check(&self, url: &str) -> Result<(), String> {
        let parsed = url::Url::parse(url).map_err(|_| format!("'{url}' is not a valid URL"))?;
        if !matches!(parsed.scheme(), "http" | "https") {
            return Err(format!("'{url}' is not an http(s) URL"));
        }
        let host = parsed
            .host_str()
            .ok_or_else(|| format!("'{url}' has no host"))?;
        // An IP literal never reaches a resolver, so the address is checked here.
        let literal = match parsed.host() {
            Some(url::Host::Ipv4(ip)) => Some(IpAddr::V4(ip)),
            Some(url::Host::Ipv6(ip)) => Some(IpAddr::V6(ip)),
            _ => None,
        };
        if let Some(ip) = literal.filter(|ip| is_link_local_or_metadata(*ip)) {
            return Err(format!(
                "{} is a link-local or cloud metadata address; flow run never sends requests there",
                ip.to_canonical()
            ));
        }
        if self.allows_host(host) {
            return Ok(());
        }
        Err(format!(
            "{} is outside the hosts this flow may reach ({})",
            host.to_ascii_lowercase(),
            if self.base.is_empty() {
                "none".to_string()
            } else {
                self.base.join(", ")
            }
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn host_policy_accepts_equivalent_dns_hostnames() {
        let policy = HostPolicy::from_start("https://Example.COM./start");
        for url in [
            "https://example.com./next",
            "https://EXAMPLE.com/next",
            "https://api.example.com./next",
        ] {
            assert!(policy.check(url).is_ok(), "{url}");
        }
    }

    #[test]
    fn saved_allowed_hosts_are_case_and_root_dot_insensitive() {
        let flow: Flow = serde_json::from_value(json!({
            "version": 1,
            "name": "api",
            "origin": "example.com",
            "startUrl": "https://example.com/",
            "allowedHosts": ["API.Example.COM."],
            "steps": [],
        }))
        .unwrap();
        let policy = HostPolicy::of(&flow);
        assert!(policy.check("https://api.example.com/data").is_ok());
        assert!(policy.check("https://api.example.com./data").is_ok());
        assert!(policy.check("https://v1.api.example.com/data").is_ok());
        for url in [
            "https://example.com/data",
            "https://www.example.com/data",
            "https://notapi.example.com/data",
            "https://api.example.com.evil.com/data",
        ] {
            assert!(policy.check(url).is_err(), "{url}");
        }
    }

    #[test]
    fn normalized_allowed_hosts_cannot_widen_start_domain() {
        let flow: Flow = serde_json::from_value(json!({
            "version": 1,
            "name": "api",
            "origin": "a.vercel.app",
            "startUrl": "https://a.vercel.app/",
            "allowedHosts": ["A.Vercel.APP.", "B.Vercel.APP."],
            "steps": [],
        }))
        .unwrap();
        let policy = HostPolicy::of(&flow);
        assert!(policy.check("https://a.vercel.app./data").is_ok());
        assert!(policy.check("https://b.vercel.app./data").is_err());
        assert!(policy.check("https://not-a.vercel.app/data").is_err());
        assert!(policy.check("file://a.vercel.app/data").is_err());
    }

    #[test]
    fn link_local_and_metadata_addresses_are_refused() {
        let policy = HostPolicy::from_start("http://169.254.169.254/");
        let err = policy.check("http://169.254.169.254/latest").unwrap_err();
        assert!(err.contains("link-local or cloud metadata"), "{err}");
        let policy = HostPolicy::from_start("http://127.0.0.1:8765/");
        assert!(policy.check("http://127.0.0.1:8765/api/items.json").is_ok());
    }
}
