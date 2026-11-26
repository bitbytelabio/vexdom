//! The blocked-sites list (`src/lib/settings/blocked-sites.ts`, ported with P9): parsing what the
//! user types into a stored pattern, and matching a page's URL against the list.
//!
//! A pattern compiles to a regex `source` that is also handed to user scripts, which compile it in
//! the page as a JavaScript `RegExp(source, 'i')`; so the emitted text stays exactly what the
//! TypeScript emitted. Rust matches it with the regex crate, case-insensitively.

use std::sync::LazyLock;

use regex::{Regex, RegexBuilder};
use serde_json::Value;
use url::Url;

use crate::secrets::WS;
use crate::text::utf16_len;

/// The most characters an entry may have (`BLOCKED_SITES_LIMITS.length`).
pub const MAX_LENGTH: usize = 200;

const BOUNDARY: &str = "(?:[/?&;].*)?$";
const DEFAULT_PORTS: &[&str] = &["80", "443"];

const NOT_A_SITE: &str = "That is not a site name, like example.com or localhost:3000.";

fn re(pattern: &str) -> Regex {
    Regex::new(pattern).expect("blocked-sites pattern compiles")
}

static WEB_SCHEME: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)^(?:https?|\*)://"));
static ANY_SCHEME: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)^[a-z][a-z0-9+.-]*://"));
static WILDCARD_HOST: LazyLock<Regex> = LazyLock::new(|| re(r"^[a-z0-9*.-]+$"));
static HAS_SPACE: LazyLock<Regex> = LazyLock::new(|| re(&format!("[{WS}]")));
static HOST_PORT: LazyLock<Regex> =
    LazyLock::new(|| re(r"^(\[[0-9a-f:.]+\]|[^:]+)(?::(\d{1,5}))?$"));
static ALL_STARS: LazyLock<Regex> = LazyLock::new(|| re(r"^[*.]+$"));
static NUMERIC_OR_V6: LazyLock<Regex> = LazyLock::new(|| re(r"^[\d.]+$|^\["));

/// A pattern split into its parts: host (maybe with `*`), a non-default port, and a path.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Parts {
    host: String,
    port: Option<String>,
    path: Option<String>,
}

/// One compiled entry: the stored pattern, its regex source, and the source of its site part.
#[derive(Debug, Clone)]
pub struct Compiled {
    pub pattern: String,
    pub source: String,
    pub site_source: String,
    regex: Regex,
    site: Regex,
}

fn ci(source: &str) -> Option<Regex> {
    RegexBuilder::new(source)
        .case_insensitive(true)
        .build()
        .ok()
}

/// `{ok:true, pattern, source}` as `Ok((pattern, source))`, or the reason the entry is refused.
pub fn parse_pattern(entry: &str) -> Result<(String, String), String> {
    let parts = parts_of(entry)?;
    Ok((pattern_of(&parts), source_of(&parts)))
}

/// `None` when the stored value is not a list at all: the caller must refuse, not guess. Entries
/// that are not strings or do not parse are skipped.
pub fn compile(raw: &Value) -> Option<Vec<Compiled>> {
    let Value::Array(entries) = raw else {
        return None;
    };
    Some(
        entries
            .iter()
            .filter_map(Value::as_str)
            .filter_map(|entry| {
                let parts = parts_of(entry).ok()?;
                let source = source_of(&parts);
                let site_source = site_source(&parts);
                Some(Compiled {
                    pattern: pattern_of(&parts),
                    regex: ci(&source)?,
                    site: ci(&format!("{site_source}$"))?,
                    source,
                    site_source,
                })
            })
            .collect(),
    )
}

/// The patterns that could match somewhere on this origin: all a page on it ever needs.
pub fn patterns_for<'a>(origin: &str, list: &'a [Compiled]) -> Vec<&'a Compiled> {
    let Some(subject) = subject_of(&format!("{origin}/")) else {
        return Vec::new();
    };
    let site = subject.strip_suffix('/').unwrap_or(&subject);
    list.iter()
        .filter(|entry| entry.site.is_match(site))
        .collect()
}

/// The first pattern that blocks `url`.
pub fn blocked_by<'a>(url: &str, list: &'a [Compiled]) -> Option<&'a str> {
    let subject = subject_of(url)?;
    list.iter()
        .find(|entry| entry.regex.is_match(&subject))
        .map(|entry| entry.pattern.as_str())
}

/// What a pattern is matched against: host, port and path with the query, never the hash. `None`
/// for anything but an http(s) URL.
pub fn subject_of(url: &str) -> Option<String> {
    if url.is_empty() {
        return None;
    }
    let parsed = Url::parse(url).ok()?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return None;
    }
    let host = parsed.host_str().unwrap_or("").trim_end_matches('.');
    let port = parsed
        .port()
        .map(|port| format!(":{port}"))
        .unwrap_or_default();
    let search = parsed
        .query()
        .filter(|query| !query.is_empty())
        .map(|query| format!("?{query}"))
        .unwrap_or_default();
    Some(format!(
        "{host}{port}{}{search}",
        normalize_path(parsed.path())
    ))
}

/// A sentence for the settings page: what the pattern blocks.
pub fn describe_pattern(pattern: &str) -> String {
    let Ok(Parts { host, port, path }) = parts_of(pattern) else {
        return pattern.to_owned();
    };
    let name = if host.contains('*') {
        format!("sites matching {host}")
    } else if has_subdomains(&host) {
        format!("{host} and its subdomains")
    } else {
        host.clone()
    };
    let site = match &port {
        Some(port) => format!("{name}, port {port}"),
        None => name,
    };
    match path {
        None => format!("Every page on {site}"),
        Some(path) => format!(
            "{} {path} on {site}",
            if path.contains('*') {
                "Pages matching"
            } else {
                "Pages under"
            }
        ),
    }
}

fn has_subdomains(host: &str) -> bool {
    host.contains('.') && !NUMERIC_OR_V6.is_match(host)
}

fn parts_of(entry: &str) -> Result<Parts, String> {
    let trimmed = crate::text::js_trim(entry);
    if trimmed.is_empty() {
        return Err("Type a site, like example.com.".into());
    }
    if utf16_len(trimmed) > MAX_LENGTH {
        return Err(format!("Keep it under {MAX_LENGTH} characters."));
    }
    if HAS_SPACE.is_match(trimmed) {
        return Err("A pattern has no spaces.".into());
    }
    if !WEB_SCHEME.is_match(trimmed) && ANY_SCHEME.is_match(trimmed) {
        return Err("Only web pages (http and https) can be blocked.".into());
    }

    let bare = WEB_SCHEME.replace(trimmed, "");
    let bare = bare.split('#').next().unwrap_or("");
    let split = bare.find(['/', '?']);
    let authority = split.map_or(bare, |at| &bare[..at]).to_lowercase();
    let rest = split.map_or("", |at| &bare[at..]);

    if authority.contains('@') {
        return Err("Leave out any user name — block the site itself.".into());
    }
    let caps = HOST_PORT.captures(&authority).ok_or(NOT_A_SITE)?;
    let raw_port = caps.get(2).map(|m| m.as_str());
    if raw_port.is_some_and(|port| port.parse::<u32>().unwrap_or(0) > 65_535) {
        return Err(NOT_A_SITE.into());
    }

    let host = normalize_host(&caps[1]).ok_or(NOT_A_SITE)?;
    if ALL_STARS.is_match(&host) {
        return Err("That would block every site.".into());
    }

    let port = raw_port
        .filter(|port| !DEFAULT_PORTS.contains(port))
        .map(|port| port.parse::<u32>().unwrap_or(0).to_string());
    Ok(Parts {
        host,
        port,
        path: normalize_pattern_path(rest),
    })
}

fn normalize_host(raw: &str) -> Option<String> {
    let host = raw.trim_end_matches('.');
    let host = host.strip_prefix("*.").unwrap_or(host);
    let dropped = match host.strip_prefix("www.") {
        Some(rest) if rest.contains('.') => rest,
        _ => host,
    };
    if dropped.is_empty() {
        return None;
    }
    if !dropped.contains('*') {
        return ascii_host(dropped);
    }
    let labels: Option<Vec<String>> = dropped
        .split('.')
        .map(|label| {
            let label = if label.contains('*') {
                label.to_owned()
            } else {
                ascii_host(label)?
            };
            (!label.is_empty() && WILDCARD_HOST.is_match(&label)).then_some(label)
        })
        .collect();
    labels.map(|labels| labels.join("."))
}

/// `new URL('http://' + host + '/').hostname`, trailing dots trimmed: IDNA to punycode, and
/// anything that is not a host is `None`.
fn ascii_host(host: &str) -> Option<String> {
    let parsed = Url::parse(&format!("http://{host}/")).ok()?;
    let name = parsed.host_str()?.trim_end_matches('.');
    (!name.is_empty()).then(|| name.to_owned())
}

fn normalize_pattern_path(rest: &str) -> Option<String> {
    if rest.is_empty() {
        return None;
    }
    let with_slash = if rest.starts_with('?') {
        format!("/{rest}")
    } else {
        rest.to_owned()
    };
    let query = with_slash.find('?');
    let pathname = normalize_path(query.map_or(with_slash.as_str(), |at| &with_slash[..at]));
    // `.replace(/(.)\/$/, '$1')`: a trailing slash goes unless it is the whole path.
    let pathname = match pathname.strip_suffix('/') {
        Some(stripped) if !stripped.is_empty() => stripped.to_owned(),
        _ => pathname,
    };
    let path = format!("{pathname}{}", query.map_or("", |at| &with_slash[at..]));
    (path != "/").then_some(path)
}

/// Collapses `//` and decodes escapes that spell plain characters, so `/%61dmin` is `/admin`.
fn normalize_path(path: &str) -> String {
    let bytes = path.as_bytes();
    let mut decoded = String::with_capacity(path.len());
    let mut at = 0;
    while at < path.len() {
        if bytes[at] == b'%'
            && let Some(hex) = path.get(at + 1..at + 3)
            && hex.bytes().all(|b| b.is_ascii_hexdigit())
        {
            let byte = u8::from_str_radix(hex, 16).expect("two hex digits");
            if byte.is_ascii_alphanumeric() || b"-._~".contains(&byte) {
                decoded.push(byte as char);
            } else {
                decoded.push_str(&path[at..at + 3]);
            }
            at += 3;
            continue;
        }
        let c = path[at..].chars().next().expect("at is a char boundary");
        decoded.push(c);
        at += c.len_utf8();
    }
    let mut out = String::with_capacity(decoded.len());
    for c in decoded.chars() {
        if c == '/' && out.ends_with('/') {
            continue;
        }
        out.push(c);
    }
    out
}

fn pattern_of(parts: &Parts) -> String {
    format!(
        "{}{}{}",
        parts.host,
        parts
            .port
            .as_ref()
            .map(|port| format!(":{port}"))
            .unwrap_or_default(),
        parts.path.as_deref().unwrap_or("")
    )
}

fn source_of(parts: &Parts) -> String {
    format!(
        "{}{}",
        site_source(parts),
        path_source(parts.path.as_deref())
    )
}

fn site_source(parts: &Parts) -> String {
    format!(
        "^(?:[^/:]*\\.)?{}{}",
        glob(&parts.host, "[^/:]*"),
        parts
            .port
            .as_ref()
            .map_or_else(|| "(?::\\d+)?".to_owned(), |port| format!(":{port}"))
    )
}

fn path_source(path: Option<&str>) -> String {
    match path {
        None => BOUNDARY.to_owned(),
        Some(path) => format!(
            "{}{}",
            glob(path, ".*"),
            if path.ends_with('*') { "$" } else { BOUNDARY }
        ),
    }
}

/// `*` becomes `star`; everything a JavaScript regex treats as special is escaped, `/` included.
fn glob(text: &str, star: &str) -> String {
    text.split('*')
        .map(|piece| {
            let mut out = String::with_capacity(piece.len());
            for c in piece.chars() {
                if ".*+?^${}()|[]\\/".contains(c) {
                    out.push('\\');
                }
                out.push(c);
            }
            out
        })
        .collect::<Vec<_>>()
        .join(star)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn normalized(entry: &str) -> String {
        parse_pattern(entry)
            .unwrap_or_else(|reason| panic!("{entry}: {reason}"))
            .0
    }

    fn blocks(pattern: &str, url: &str) -> bool {
        blocked_by(url, &compile(&json!([pattern])).expect("a list")).is_some()
    }

    #[test]
    fn stored_forms() {
        for (entry, pattern) in [
            ("example.com", "example.com"),
            ("  Example.COM  ", "example.com"),
            ("https://www.example.com/", "example.com"),
            ("*://example.com/admin", "example.com/admin"),
            ("*.example.com", "example.com"),
            ("www.com", "www.com"),
            ("example.com./admin/", "example.com/admin"),
            ("example.com/%61dmin", "example.com/admin"),
            ("example.com//admin", "example.com/admin"),
            ("example.com?q=1", "example.com/?q=1"),
            ("example.com/admin#section", "example.com/admin"),
            ("localhost:3000", "localhost:3000"),
            ("example.com:443", "example.com"),
            ("bücher.de", "xn--bcher-kva.de"),
            ("bank.*", "bank.*"),
            ("[::1]:8080", "[::1]:8080"),
            ("GitHub.com/*/Settings", "github.com/*/Settings"),
        ] {
            assert_eq!(normalized(entry), pattern, "{entry}");
        }
        for entry in [
            "",
            "   ",
            "*",
            "*.*",
            "ftp://example.com",
            "javascript:alert(1)",
            "exa mple.com",
            "user@example.com",
            "example.com:99999",
            "%%%",
            "bü*cher.de",
        ] {
            assert!(parse_pattern(entry).is_err(), "{entry:?}");
        }
        assert!(parse_pattern(&"x".repeat(MAX_LENGTH + 1)).is_err());
    }

    #[test]
    fn matching() {
        for (pattern, url, want) in [
            ("example.com", "https://mail.example.com/inbox?x=1", true),
            ("example.com", "https://EXAMPLE.com/", true),
            ("example.com", "https://example.com.evil.net/", false),
            (
                "example.com",
                "https://evil.net/?next=https://example.com",
                false,
            ),
            ("example.com/admin", "https://example.com/%61dmin", true),
            (
                "example.com/admin",
                "https://example.com/admin;jsessionid=1",
                true,
            ),
            (
                "example.com/admin",
                "https://example.com/administrator",
                false,
            ),
            (
                "github.com/*/settings",
                "https://github.com/acme/settings/keys",
                true,
            ),
            ("bank.*", "https://bank.co.uk/", true),
            ("bank.*", "https://bank/", false),
            ("localhost:3000", "http://localhost:3001/app", false),
            ("example.com/a+b(c)", "https://example.com/a+b(c)/d", true),
            ("example.com/a+b(c)", "https://example.com/aab(c)", false),
        ] {
            assert_eq!(blocks(pattern, url), want, "{pattern} on {url}");
        }
        assert!(compile(&json!("example.com")).is_none());
        assert!(compile(&Value::Null).is_none());
        let list = compile(&json!(["example.com", 42, "*", "ftp://x.org"])).expect("a list");
        assert_eq!(list.len(), 1);
        assert_eq!(blocked_by("chrome://settings", &list), None);
    }

    #[test]
    fn subjects_sources_and_descriptions() {
        assert_eq!(
            subject_of("https://Example.com:8443/a//b?c=1#frag").as_deref(),
            Some("example.com:8443/a/b?c=1")
        );
        assert_eq!(
            subject_of("http://example.com:80/").as_deref(),
            Some("example.com/")
        );
        assert_eq!(
            parse_pattern("example.com/a+b").expect("parses").1,
            r"^(?:[^/:]*\.)?example\.com(?::\d+)?\/a\+b(?:[/?&;].*)?$"
        );
        let list = compile(&json!(["example.com/admin", "other.org"])).expect("a list");
        let sources: Vec<_> = patterns_for("https://www.example.com", &list)
            .into_iter()
            .map(|entry| entry.source.as_str())
            .collect();
        assert_eq!(sources.len(), 1);
        assert_eq!(
            describe_pattern("127.0.0.1:8080/admin"),
            "Pages under /admin on 127.0.0.1, port 8080"
        );
        assert_eq!(
            describe_pattern("bank.*"),
            "Every page on sites matching bank.*"
        );
    }
}
