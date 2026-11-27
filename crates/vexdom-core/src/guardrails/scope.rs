//! A run's blast radius: the hosts it may reach and the tab it may drive (`guardrails/scope.ts`).

use std::sync::LazyLock;

use regex::Regex;
use serde_json::Value;
use url::Url;

use vexdom_pure::secrets::WS;

/// Actions whose `url` names where they reach: a page to load, a file to fetch, a cookie jar, a
/// page read over HTTP (`page.checkSites`, asked before each fetch).
pub const NAVIGATIONS: [&str; 6] = [
    "page.checkSites",
    "page.navigate",
    "page.openTab",
    "page.openWindow",
    "page.captureDownload",
    "page.cookies",
];
const TAB_MOVES: [&str; 2] = ["page.switchTab", "page.closeTab"];
/// Endings that look like a host but are almost always a filename in prose.
const NOT_A_HOST: [&str; 13] = [
    "txt", "md", "json", "csv", "pdf", "png", "jpg", "jpeg", "zip", "js", "ts", "sh", "py",
];

/// The look-ahead of the TS pattern becomes a consumed terminator; equivalent because a match can
/// only start with an alphanumeric, which no terminator is.
static HOST_IN_TEXT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(
        r#"(?i-u:https?://)?((?:[a-zA-Z0-9](?:[a-zA-Z0-9-]*[a-zA-Z0-9])?\.)+[a-zA-Z]{{2,24}})(?:[/{WS},;:!?)"'\]]|$)"#
    ))
    .expect("host pattern compiles")
});

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Scope {
    /// Hosts this run may navigate to. `["*"]` means anywhere.
    pub hosts: Vec<String>,
    /// Tab the run is pinned to.
    pub tab_id: Option<i64>,
    /// Tabs the run opened itself.
    pub owned_tab_ids: Vec<i64>,
    /// The conversation's other tabs.
    pub shared_tab_ids: Vec<i64>,
    /// Where the run's actions land since a tab action moved it: `Some(None)` when the browser
    /// picked the tab, `None` when no tab action happened.
    pub current_tab_id: Option<Option<i64>>,
}

/// No confinement: what an external MCP client gets.
pub fn anywhere() -> Scope {
    Scope {
        hosts: vec!["*".into()],
        ..Scope::default()
    }
}

#[derive(Debug, Clone, Default)]
pub struct SharedTab {
    pub tab_id: i64,
    pub host: Option<String>,
    pub chosen: bool,
}

#[derive(Debug, Clone, Default)]
pub struct ScopeSeed {
    pub url: Option<String>,
    pub tab_id: Option<i64>,
    pub instruction: Option<String>,
    pub extra_hosts: Vec<String>,
    pub pin_tab: bool,
    pub tabs: Vec<SharedTab>,
}

pub fn scope_for(seed: &ScopeSeed) -> Scope {
    let tab_id = if seed.pin_tab { seed.tab_id } else { None };
    let shared_tab_ids: Vec<i64> = seed.tabs.iter().map(|tab| tab.tab_id).collect();
    let mut hosts: Vec<String> = Vec::new();
    let mut add = |host: String| {
        if !hosts.contains(&host) {
            hosts.push(host);
        }
    };
    for host in &seed.extra_hosts {
        if host == "*" {
            return Scope {
                hosts: vec!["*".into()],
                tab_id,
                shared_tab_ids,
                ..Scope::default()
            };
        }
        if let Some(normalized) = normalize_host(host) {
            add(normalized);
        }
    }
    if let Some(start) = seed.url.as_deref().and_then(host_of) {
        add(start);
    }
    for tab in &seed.tabs {
        if tab.chosen
            && let Some(host) = tab.host.as_deref().filter(|host| !host.is_empty())
            && let Some(chosen) = host_of(&format!("https://{host}/"))
        {
            add(chosen);
        }
    }
    for caps in HOST_IN_TEXT.captures_iter(seed.instruction.as_deref().unwrap_or("")) {
        if let Some(normalized) = normalize_host(&caps[1])
            && !NOT_A_HOST.contains(&normalized.rsplit('.').next().unwrap_or(""))
        {
            add(normalized);
        }
    }
    if hosts.is_empty() {
        hosts.push("*".into());
    }
    Scope {
        hosts,
        tab_id,
        shared_tab_ids,
        ..Scope::default()
    }
}

/// Lowercase, drop a trailing root dot and a leading `*.` or `www.`; `None` for anything that is
/// not a bare host.
pub fn normalize_host(host: &str) -> Option<String> {
    let lower = host.trim().to_lowercase();
    let mut trimmed = lower.strip_suffix('.').unwrap_or(&lower);
    trimmed = trimmed.strip_prefix("*.").unwrap_or(trimmed);
    trimmed = trimmed.strip_prefix("www.").unwrap_or(trimmed);
    let valid = !trimmed.is_empty()
        && trimmed.chars().all(|c| {
            c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '-' | '[' | ']' | ':')
        });
    valid.then(|| trimmed.to_owned())
}

pub fn host_allowed(host: &str, hosts: &[String]) -> bool {
    if hosts.iter().any(|entry| entry == "*") {
        return true;
    }
    let Some(target) = normalize_host(host) else {
        return false;
    };
    hosts
        .iter()
        .any(|entry| target == *entry || target.ends_with(&format!(".{entry}")))
}

fn host_of(url: &str) -> Option<String> {
    Url::parse(url)
        .ok()
        .and_then(|parsed| normalize_host(&hostname(&parsed)))
}

/// `URL.hostname`: empty for a URL with no host.
pub fn hostname(url: &Url) -> String {
    url.host_str().unwrap_or("").to_owned()
}

const PROBES: [&str; 2] = ["https://one.probe.invalid/", "https://two.probe.invalid/"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetKind {
    Absolute,
    Authority,
    Path,
    Opaque,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NavigationTarget {
    pub kind: TargetKind,
    pub url: Option<Url>,
}

/// The navigation an action asks for, classified by how much of the destination the caller's own
/// string settles (`targetUrl`).
pub fn target_url(action: &str, input: &Value) -> Option<NavigationTarget> {
    if !NAVIGATIONS.contains(&action) {
        return None;
    }
    let url = input.get("url")?.as_str()?;
    if let Ok(alone) = Url::parse(url) {
        return Some(NavigationTarget {
            kind: TargetKind::Absolute,
            url: Some(alone),
        });
    }
    let parse = |base: &str| Url::parse(base).ok().and_then(|base| base.join(url).ok());
    let (Some(one), Some(two)) = (parse(PROBES[0]), parse(PROBES[1])) else {
        return Some(NavigationTarget {
            kind: TargetKind::Opaque,
            url: None,
        });
    };
    let (one_host, two_host) = (hostname(&one), hostname(&two));
    if one_host == two_host {
        return Some(NavigationTarget {
            kind: TargetKind::Authority,
            url: Some(one),
        });
    }
    if one_host == "one.probe.invalid" && two_host == "two.probe.invalid" {
        return Some(NavigationTarget {
            kind: TargetKind::Path,
            url: Some(one),
        });
    }
    Some(NavigationTarget {
        kind: TargetKind::Opaque,
        url: None,
    })
}

/// The URL when the caller's own string settled the host.
pub fn host_bearing_url(action: &str, input: &Value) -> Option<Url> {
    target_url(action, input)
        .filter(|target| matches!(target.kind, TargetKind::Absolute | TargetKind::Authority))
        .and_then(|target| target.url)
}

/// Bytes a navigation would carry out in the query string or fragment (`search` and `hash`
/// include their `?`/`#` when non-empty).
pub fn url_payload_bytes(url: &Url) -> usize {
    let search = url
        .query()
        .filter(|q| !q.is_empty())
        .map_or(0, |q| q.len() + 1);
    let hash = url
        .fragment()
        .filter(|f| !f.is_empty())
        .map_or(0, |f| f.len() + 1);
    search + hash
}

/// Whether an action would move the run off the tab it is pinned to.
pub fn targets_another_tab(action: &str, input: &Value, scope: &Scope) -> bool {
    let Some(pinned) = scope.tab_id else {
        return false;
    };
    if !TAB_MOVES.contains(&action) {
        return false;
    }
    let own = |tab: Option<i64>| {
        tab.is_some_and(|tab| tab == pinned || scope.owned_tab_ids.contains(&tab))
    };
    if let Some(tab) = input.get("tabId").and_then(Value::as_f64) {
        let tab = tab as i64;
        if own(Some(tab)) {
            return false;
        }
        return !(action == "page.switchTab" && scope.shared_tab_ids.contains(&tab));
    }
    if input
        .get("match")
        .and_then(Value::as_str)
        .is_some_and(|text| !text.is_empty())
    {
        return true;
    }
    let current = scope.current_tab_id.unwrap_or(Some(pinned));
    action == "page.closeTab" && !own(current)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn scope_from_instruction_and_url() {
        let scope = scope_for(&ScopeSeed {
            url: Some("https://www.Example.com/a".into()),
            instruction: Some("compare with docs.rs, and notes.txt".into()),
            ..ScopeSeed::default()
        });
        assert_eq!(scope.hosts, vec!["example.com", "docs.rs"]);
        assert_eq!(scope_for(&ScopeSeed::default()).hosts, vec!["*"]);
        assert!(host_allowed("app.example.com", &scope.hosts));
        assert!(!host_allowed("evil.com", &scope.hosts));
    }

    #[test]
    fn classifies_targets() {
        let kind = |url: &str| target_url("page.navigate", &json!({"url": url})).map(|t| t.kind);
        assert_eq!(kind("https://a.com/x"), Some(TargetKind::Absolute));
        assert_eq!(kind("//evil.com/x"), Some(TargetKind::Authority));
        assert_eq!(kind("\\\\evil.com/x"), Some(TargetKind::Authority));
        assert_eq!(kind("/path?q=1"), Some(TargetKind::Path));
        assert_eq!(kind("http://[::1"), Some(TargetKind::Opaque));
        assert_eq!(target_url("page.click", &json!({"url": "x"})), None);
        let url = Url::parse("https://a.com/?ab#c").unwrap();
        assert_eq!(url_payload_bytes(&url), 5);
    }

    #[test]
    fn tab_moves() {
        let scope = Scope {
            hosts: vec!["*".into()],
            tab_id: Some(1),
            shared_tab_ids: vec![2],
            ..Scope::default()
        };
        assert!(!targets_another_tab(
            "page.switchTab",
            &json!({"tabId": 2}),
            &scope
        ));
        assert!(targets_another_tab(
            "page.closeTab",
            &json!({"tabId": 2}),
            &scope
        ));
        assert!(targets_another_tab(
            "page.switchTab",
            &json!({"match": "x"}),
            &scope
        ));
        assert!(!targets_another_tab("page.closeTab", &json!({}), &scope));
        assert!(!targets_another_tab("page.switchTab", &json!({}), &scope));
    }
}
