//! The in-page tier: the request sent by the flow's tab with its own `fetch` (`page.evaluate`),
//! so the browser adds its cookies, client hints and the headers the site's scripts would not.

use std::time::{Duration, Instant};

use serde_json::{Map, Value, json};
use vexdom_core::result::ActionError;

use super::outcome::{Failure, TierResult, tier_result, with_retry_after};
use super::send::Browser;
use super::{Flow, HeaderClass, HostPolicy, PreparedRequest, Tier, classify_header};

const PAGE_FETCH_WRITE: &str =
    r#"const r = await fetch(u, { ...i, credentials: 'include', signal: controller.signal });"#;
const PAGE_FETCH_READ: &str = r#"let r;
try {
    r = await fetch(u, { ...i, credentials: 'include', signal: controller.signal });
} catch (e) {
    if (controller.signal.aborted) throw e;
    r = await fetch(u, { ...i, signal: controller.signal });
}"#;

/// Codes that say `page.evaluate` never ran the script: the request was not sent.
const NOT_RUN: &[&str] = &[
    "USER_SCRIPTS_DISABLED",
    "SITE_BLOCKED",
    "TAB_UNREACHABLE",
    "NO_ACTIVE_TAB",
    "SECRET_NOT_RELEASABLE",
];

/// The script the tab runs: one `fetch`, no redirect followed.
pub(crate) fn page_fetch_script(req: &PreparedRequest, timeout: Duration) -> String {
    // Headers the browser sets itself. Sending them as app headers turns the
    // request into a CORS preflight the site never allowed.
    let headers: Map<String, Value> = req
        .headers
        .iter()
        .filter(|(k, _)| classify_header(k) != HeaderClass::BrowserSet)
        .map(|(k, v)| (k.clone(), json!(v)))
        .collect();
    let mut init = json!({
        "method": req.method,
        "headers": headers,
        "body": req.body,
    });
    // No request follows a redirect: the browser would send the next hop
    // before the site boundary could be checked. A write's 3xx counts as the
    // site's answer; a read moves on to the next tier.
    init["redirect"] = json!("manual");
    // Cross-origin APIs differ on credentials: some need cookies, some reject
    // a credentialed request outright. Try with, then without.
    // A write is never sent a second time: a thrown fetch may still have
    // reached the server, so it is reported as an unknown outcome instead.
    let send = if req.write {
        PAGE_FETCH_WRITE
    } else {
        PAGE_FETCH_READ
    };
    format!(
        r#"(async () => {{
    const u = {url}, i = {init};
    const controller = new AbortController();
    const timer = setTimeout(() => controller.abort(new Error("the run's time is up")), {timeout_ms});
    try {{
        {send}
        return {{ status: r.type === 'opaqueredirect' ? 302 : r.status, redirected: r.type === 'opaqueredirect', url: r.url, retryAfter: r.headers.get('retry-after'), text: await r.text() }};
    }} finally {{
        clearTimeout(timer);
    }}
}})()"#,
        url = json!(req.url),
        timeout_ms = timeout.as_millis().min(i32::MAX as u128),
    )
}

pub(crate) fn page_result(
    policy: &HostPolicy,
    req: &PreparedRequest,
    started: Instant,
    value: Result<Value, ActionError>,
) -> TierResult {
    let retry_after = value
        .as_ref()
        .ok()
        .and_then(|v| v["retryAfter"].as_str())
        .map(String::from);
    // The page it ended on must be on the site. An opaque redirect has no URL.
    let landed = value
        .as_ref()
        .ok()
        .and_then(|v| v["url"].as_str())
        .filter(|u| !u.is_empty())
        .and_then(|u| policy.check(u).err().map(|_| u.to_string()));
    let outcome = match value {
        // The page tier does not follow redirects, so the hop was never
        // sent; the next tier can still answer the read.
        Ok(v) if !req.write && v["redirected"] == json!(true) => Err(Failure::skipped(
            "the in-page read was answered with a redirect, which the page tier does not follow"
                .to_string(),
        )),
        Ok(v) if landed.is_some() => {
            let to = landed.unwrap_or_default();
            if req.write {
                Err(Failure::unknown_outcome(format!(
                    "the in-page write was sent and ended at {to}, outside the flow's site"
                )))
            } else {
                Err(Failure::new(
                    false,
                    v["status"].as_u64().map(|s| s as u16),
                    format!("the redirect to {to} was not followed"),
                ))
            }
        }
        Ok(v) => match (v["status"].as_u64(), v["text"].as_str()) {
            (Some(status), Some(text)) => Ok((status as u16, text.to_string())),
            _ if req.write => Err(Failure::unknown_outcome(
                "the in-page write returned no response and may have been sent".to_string(),
            )),
            _ => Err(Failure::transport(
                "in-page fetch returned no response".to_string(),
            )),
        },
        Err(e) if NOT_RUN.contains(&e.code.as_str()) => Err(Failure::skipped(format!(
            "the in-page request did not run: {e}"
        ))),
        Err(e) if req.write => Err(Failure::unknown_outcome(format!(
            "the in-page write may have been sent: {e}"
        ))),
        Err(e) => Err(Failure::transport(format!("in-page fetch failed: {e}"))),
    };
    with_retry_after(tier_result(req, Tier::Page, started, outcome), retry_after)
}

/// The in-page tier needs a tab that is already on the flow's site.
pub(crate) fn tab_on_site(flow: &Flow, current: Option<&str>) -> Result<(), String> {
    let policy = HostPolicy::of(flow);
    let site = policy.hosts().join(", ");
    match current {
        Some(url) if policy.check(url).is_ok() => Ok(()),
        Some(url) => Err(format!(
            "the active tab ({url}) is not on the flow's site ({site}); open the site to use the in-page request"
        )),
        None => Err(format!(
            "the tab shows no web page; open the flow's site ({site}) to use the in-page request"
        )),
    }
}

/// The in-page tier over the flow's tab.
pub(crate) async fn send<B: Browser>(
    flow: &Flow,
    browser: &mut B,
    req: &PreparedRequest,
    deadline: Instant,
) -> TierResult {
    let started = Instant::now();
    let left = deadline.saturating_duration_since(Instant::now());
    let Ok(tab) = tokio::time::timeout(left, browser.tab_url()).await else {
        return TierResult::fail(
            req,
            Tier::Page,
            started,
            Failure::unavailable("the run's time is up".to_string()),
        );
    };
    if let Err(reason) = tab_on_site(flow, tab.as_deref()) {
        return TierResult::fail(req, Tier::Page, started, Failure::skipped(reason));
    }
    let left = deadline.saturating_duration_since(Instant::now());
    if left.is_zero() {
        return TierResult::fail(
            req,
            Tier::Page,
            started,
            Failure::unavailable("the run's time is up".to_string()),
        );
    }
    let value =
        match tokio::time::timeout(left, browser.eval(&page_fetch_script(req, left), left)).await {
            Ok(value) => value,
            Err(_) => Err(ActionError {
                code: "TIMEOUT".into(),
                message: "the run's time is up".into(),
            }),
        };
    page_result(&HostPolicy::of(flow), req, started, value)
}
