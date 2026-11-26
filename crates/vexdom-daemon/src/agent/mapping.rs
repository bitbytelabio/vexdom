//! What a site-mapping run may do, and what it has done (`mapping.ts`): it looks at the pages of
//! the one site it maps and changes none of them, within a budget of pages and screenshots, and
//! its screenshots go into the staged map.

use serde_json::Value;
use url::Url;
use vexdom_core::config::SiteMapSettings;
use vexdom_core::log;
use vexdom_core::proto::RESERVED_PREFIX;
use vexdom_core::result::{ActionError, ActionResult};
use vexdom_core::tools::SAVE_SITE_MAP_ACTION;

use super::SaveTo;
use super::site_map_store::{MapTarget, Staging};
use super::sitemap::SiteIndex;
use vexdom_pure::text::js_trim;

const READ_ONLY_ACTIONS: [&str; 19] = [
    "page.getPageInfo",
    "page.query",
    "page.waitFor",
    "page.extractText",
    "page.navigate",
    "page.screenshot",
    "page.scrollTo",
    "page.waitForElement",
    "page.hoverElement",
    "page.highlightElement",
    "page.findProgress",
    "page.findSearch",
    "page.findCaptcha",
    "page.listSiteTools",
    "page.switchFrame",
    "page.monitorStatus",
    "page.timerStatus",
    "page.readTheme",
    "page.auditContrast",
];

const CLICK_ACTION: &str = "page.clickElement";
const NAVIGATE_ACTION: &str = "page.navigate";
const SCREENSHOT_ACTION: &str = "page.screenshot";

/// `MapRun`.
#[derive(Debug, Clone)]
pub struct MapRun {
    pub target: MapTarget,
    pub staging: Staging,
    pub index: SiteIndex,
    pub settings: SiteMapSettings,
    pub tab_id: Option<i64>,
    pub shots: u64,
    /// Paths on the site, each once, in the order the run first reached them.
    pub pages_visited: Vec<String>,
    /// The host (and port) the tab drifted to, while it is off the site. Empty for an address with
    /// no host, which, as in TS, blocks nothing.
    pub off_site: Option<String>,
    pub submitted: bool,
}

impl MapRun {
    /// The host the tab is on while that blocks reading (`if (run.offSite)`).
    fn drifted_to(&self) -> Option<&str> {
        self.off_site.as_deref().filter(|host| !host.is_empty())
    }
}

/// `Gate`: a mapping run's call let through (a screenshot saved where the map keeps them) or
/// refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Gate {
    Allow { save_to: Option<SaveTo> },
    Deny(ActionError),
}

const ALLOW: Gate = Gate::Allow { save_to: None };

fn deny(code: &str, message: String) -> Gate {
    Gate::Deny(ActionError {
        code: code.to_owned(),
        message,
    })
}

/// `gateMappingInvoke(run, action, input)`.
pub fn gate_mapping_invoke(run: &MapRun, action: &str, input: &Value) -> Gate {
    if action == SAVE_SITE_MAP_ACTION {
        return ALLOW;
    }
    if action.starts_with(RESERVED_PREFIX) {
        return deny("UNKNOWN_ACTION", format!("Unknown action \"{action}\"."));
    }

    let allowed = READ_ONLY_ACTIONS.contains(&action)
        || (run.settings.allow_clicks && action == CLICK_ACTION);
    if !allowed {
        return deny(
            "MAPPING_READ_ONLY",
            format!(
                "A site-mapping run may only look at pages, not change them — \"{action}\" is not available. Record what you see and move on."
            ),
        );
    }

    if let Some(host) = run.drifted_to()
        && action != NAVIGATE_ACTION
    {
        return deny(
            "MAPPING_OFF_SITE",
            format!(
                "The tab is on {host}, not {}. Navigate back to an absolute {} URL before reading anything.",
                run.target.host, run.target.origin
            ),
        );
    }

    match action {
        NAVIGATE_ACTION => gate_navigate(run, input),
        SCREENSHOT_ACTION => gate_screenshot(run),
        _ => ALLOW,
    }
}

fn gate_navigate(run: &MapRun, input: &Value) -> Gate {
    let origin = &run.target.origin;
    if let Some(history) = input.get("action").and_then(Value::as_str) {
        if history == "reload" {
            return ALLOW;
        }
        return deny(
            "MAPPING_OFF_SITE",
            format!(
                "\"{history}\" would leave the site being mapped — it walks the tab's own history. Navigate to an absolute {origin} URL instead."
            ),
        );
    }

    let Some(address) = input
        .get("url")
        .and_then(Value::as_str)
        .filter(|address| !js_trim(address).is_empty())
    else {
        return deny(
            "INVALID_INPUT",
            "Give navigate an absolute URL while mapping.".into(),
        );
    };
    let Ok(url) = Url::parse(address) else {
        return deny(
            "MAPPING_OFF_SITE",
            format!(
                "While mapping, navigate needs an absolute URL like {origin}/pricing — not \"{address}\"."
            ),
        );
    };
    let landed = url.origin().ascii_serialization();
    if landed != *origin {
        return deny(
            "MAPPING_OFF_SITE",
            format!("This run may only visit {origin}, not {landed}."),
        );
    }
    let max_pages = run.settings.max_pages;
    if run.pages_visited.len() as u64 >= max_pages
        && !run.pages_visited.iter().any(|path| path == url.path())
    {
        return deny(
            "MAPPING_BUDGET",
            format!(
                "This run has already visited {max_pages} pages, which is its limit. Write up what you have with act {{\"do\":\"saveSiteMap\"}}."
            ),
        );
    }
    ALLOW
}

fn gate_screenshot(run: &MapRun) -> Gate {
    let max_screenshots = run.settings.max_screenshots;
    if run.shots >= max_screenshots {
        return deny(
            "MAPPING_BUDGET",
            format!(
                "This run has taken its {max_screenshots} screenshots. Carry on mapping without them."
            ),
        );
    }
    Gate::Allow {
        save_to: Some(SaveTo {
            dir: run.staging.screenshots.to_string_lossy().into_owned(),
            filename: format!("{:02}-{}.png", run.shots + 1, slug_for(run)),
        }),
    }
}

/// The last page seen, as a file name: lowercase letters and digits, every other run of
/// characters one dash, at most 40 long; `home` when nothing is left.
fn slug_for(run: &MapRun) -> String {
    let last = run.pages_visited.last().map_or("/", String::as_str);
    let mut slug = String::new();
    for c in last.to_lowercase().chars() {
        if c.is_ascii_lowercase() || c.is_ascii_digit() {
            slug.push(c);
        } else if !slug.ends_with('-') {
            slug.push('-');
        }
    }
    let slug: String = slug.trim_matches('-').chars().take(40).collect();
    if slug.is_empty() { "home".into() } else { slug }
}

/// `noteMappingResult(run, action, result)`: a screenshot taken, and where the tab landed.
pub fn note_mapping_result(run: &mut MapRun, action: &str, result: &ActionResult) {
    let Ok(data) = result else {
        return;
    };
    if action == SCREENSHOT_ACTION {
        run.shots += 1;
    }

    let Some(url) = landed_url(action, data).and_then(|landed| Url::parse(landed).ok()) else {
        return;
    };
    if url.origin().ascii_serialization() == run.target.origin {
        if run.drifted_to().is_some() {
            log::log(&format!("mapping run back on {}", run.target.host));
        }
        run.off_site = None;
        if !run.pages_visited.iter().any(|path| path == url.path()) {
            run.pages_visited.push(url.path().to_owned());
        }
        return;
    }
    let host = match (url.host_str(), url.port()) {
        (Some(host), Some(port)) => format!("{host}:{port}"),
        (Some(host), None) => host.to_owned(),
        (None, _) => String::new(),
    };
    if run.off_site.as_ref() != Some(&host) {
        log::log(&format!(
            "mapping run drifted to {host}; reads are blocked until it returns"
        ));
        run.off_site = Some(host);
    }
}

/// The address an action reports the tab is on, if it reports one.
fn landed_url<'a>(action: &str, data: &'a Value) -> Option<&'a str> {
    match action {
        "page.getPageInfo" => data.get("document")?.get("url")?.as_str(),
        NAVIGATE_ACTION => data.get("navigatedTo")?.as_str(),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use serde_json::json;
    use vexdom_core::result::failure;

    use super::*;

    fn map_run(settings: impl FnOnce(&mut SiteMapSettings), visited: &[&str]) -> MapRun {
        let mut defaults = SiteMapSettings {
            research: true,
            allow_clicks: false,
            max_pages: 3,
            max_screenshots: 2,
            timeout_ms: 60_000,
        };
        settings(&mut defaults);
        MapRun {
            target: MapTarget {
                host: "www.example.com".into(),
                origin: "https://www.example.com".into(),
                domain: "example.com".into(),
                name: "example-com".into(),
            },
            staging: Staging {
                id: "staging-1".into(),
                dir: PathBuf::from("/staging"),
                screenshots: PathBuf::from("/staging/screenshots"),
                evidence: PathBuf::from("/staging/evidence"),
                pages: PathBuf::from("/staging/pages"),
            },
            index: SiteIndex::none(),
            settings: defaults,
            tab_id: None,
            shots: 0,
            pages_visited: visited.iter().map(|path| (*path).to_owned()).collect(),
            off_site: None,
            submitted: false,
        }
    }

    fn fresh() -> MapRun {
        map_run(|_| {}, &[])
    }

    fn verdict(run: &MapRun, action: &str, input: Value) -> String {
        match gate_mapping_invoke(run, action, &input) {
            Gate::Allow { .. } => "allowed".into(),
            Gate::Deny(error) => error.code,
        }
    }

    fn verdicts(run: &MapRun, calls: &[(&str, Value)]) -> Vec<String> {
        calls
            .iter()
            .map(|(action, input)| verdict(run, action, input.clone()))
            .collect()
    }

    #[test]
    fn looking_at_a_page_is_allowed() {
        for action in [
            "page.getPageInfo",
            "page.extractText",
            "page.scrollTo",
            "page.hoverElement",
            "page.findSearch",
        ] {
            assert_eq!(verdict(&fresh(), action, json!({})), "allowed", "{action}");
        }
    }

    #[test]
    fn changing_a_page_is_not() {
        for action in [
            "page.fillInput",
            "page.submitForm",
            "page.typeText",
            "page.runScript",
        ] {
            assert_eq!(
                verdict(&fresh(), action, json!({})),
                "MAPPING_READ_ONLY",
                "{action}"
            );
        }
    }

    #[test]
    fn clicking_is_refused_unless_the_settings_allow_it() {
        let clicks = map_run(|settings| settings.allow_clicks = true, &[]);
        assert_eq!(
            [
                verdict(&fresh(), CLICK_ACTION, json!({})),
                verdict(&clicks, CLICK_ACTION, json!({}))
            ],
            ["MAPPING_READ_ONLY", "allowed"]
        );
    }

    #[test]
    fn handing_in_the_report_is_always_allowed_and_no_other_reserved_action_is() {
        assert_eq!(
            [
                verdict(&fresh(), SAVE_SITE_MAP_ACTION, json!({})),
                verdict(&fresh(), "vexdom.focusShot", json!({}))
            ],
            ["allowed", "UNKNOWN_ACTION"]
        );
    }

    #[test]
    fn any_page_on_the_site_being_mapped_is_allowed() {
        assert_eq!(
            verdict(
                &fresh(),
                NAVIGATE_ACTION,
                json!({"url": "https://www.example.com/pricing"})
            ),
            "allowed"
        );
    }

    #[test]
    fn another_origin_is_off_the_site_subdomain_or_not() {
        for url in [
            "https://evil.example/",
            "https://docs.example.com/",
            "http://www.example.com/",
        ] {
            assert_eq!(
                verdict(&fresh(), NAVIGATE_ACTION, json!({ "url": url })),
                "MAPPING_OFF_SITE",
                "{url}"
            );
        }
    }

    #[test]
    fn a_relative_address_is_refused_with_an_example_of_an_absolute_one() {
        assert_eq!(
            gate_mapping_invoke(&fresh(), NAVIGATE_ACTION, &json!({"url": "/pricing"})),
            Gate::Deny(
                failure::<()>(
                    "MAPPING_OFF_SITE",
                    "While mapping, navigate needs an absolute URL like https://www.example.com/pricing — not \"/pricing\"."
                )
                .unwrap_err()
            )
        );
    }

    #[test]
    fn going_back_or_forward_is_refused_but_reloading_is_fine() {
        assert_eq!(
            verdicts(
                &fresh(),
                &[
                    (NAVIGATE_ACTION, json!({"action": "back"})),
                    (NAVIGATE_ACTION, json!({"action": "forward"})),
                    (NAVIGATE_ACTION, json!({"action": "reload"})),
                ]
            ),
            ["MAPPING_OFF_SITE", "MAPPING_OFF_SITE", "allowed"]
        );
    }

    #[test]
    fn navigating_nowhere_is_invalid() {
        assert_eq!(
            verdicts(
                &fresh(),
                &[
                    (NAVIGATE_ACTION, json!({})),
                    (NAVIGATE_ACTION, json!({"url": "  "}))
                ]
            ),
            ["INVALID_INPUT", "INVALID_INPUT"]
        );
    }

    #[test]
    fn once_the_page_budget_is_spent_only_pages_already_seen_may_be_revisited() {
        let run = map_run(|settings| settings.max_pages = 2, &["/", "/pricing"]);
        assert_eq!(
            verdicts(
                &run,
                &[
                    (
                        NAVIGATE_ACTION,
                        json!({"url": "https://www.example.com/docs"})
                    ),
                    (
                        NAVIGATE_ACTION,
                        json!({"url": "https://www.example.com/pricing?plan=pro"})
                    ),
                ]
            ),
            ["MAPPING_BUDGET", "allowed"]
        );
    }

    #[test]
    fn a_tab_that_lands_elsewhere_may_only_navigate_back_until_it_returns() {
        let mut run = fresh();
        note_mapping_result(
            &mut run,
            NAVIGATE_ACTION,
            &Ok(json!({"navigatedTo": "https://accounts.example.net/login"})),
        );
        let while_away = verdicts(
            &run,
            &[
                ("page.getPageInfo", json!({})),
                (NAVIGATE_ACTION, json!({"url": "https://www.example.com/"})),
            ],
        );
        note_mapping_result(
            &mut run,
            "page.getPageInfo",
            &Ok(json!({"document": {"url": "https://www.example.com/"}})),
        );
        assert_eq!(while_away, ["MAPPING_OFF_SITE", "allowed"]);
        assert_eq!(verdict(&run, "page.getPageInfo", json!({})), "allowed");
        assert_eq!(run.off_site, None);
    }

    #[test]
    fn a_drift_names_the_host_with_its_port() {
        let mut run = fresh();
        note_mapping_result(
            &mut run,
            NAVIGATE_ACTION,
            &Ok(json!({"navigatedTo": "https://www.example.com:8443/"})),
        );
        assert_eq!(run.off_site.as_deref(), Some("www.example.com:8443"));
        assert_eq!(
            gate_mapping_invoke(&run, "page.extractText", &json!({})),
            Gate::Deny(
                failure::<()>(
                    "MAPPING_OFF_SITE",
                    "The tab is on www.example.com:8443, not www.example.com. Navigate back to an absolute https://www.example.com URL before reading anything."
                )
                .unwrap_err()
            )
        );
    }

    #[test]
    fn each_page_landed_on_is_counted_once_by_path() {
        let mut run = fresh();
        for url in [
            "https://www.example.com/",
            "https://www.example.com/pricing",
            "https://www.example.com/pricing#faq",
        ] {
            note_mapping_result(
                &mut run,
                NAVIGATE_ACTION,
                &Ok(json!({ "navigatedTo": url })),
            );
        }
        assert_eq!(run.pages_visited, ["/", "/pricing"]);
    }

    #[test]
    fn a_failed_action_or_one_that_says_nothing_about_where_the_tab_is_changes_nothing() {
        let mut run = fresh();
        note_mapping_result(&mut run, NAVIGATE_ACTION, &failure("TIMEOUT", "slow"));
        note_mapping_result(
            &mut run,
            "page.extractText",
            &Ok(json!({"content": "https://evil.example"})),
        );
        note_mapping_result(
            &mut run,
            NAVIGATE_ACTION,
            &Ok(json!({"navigatedTo": "not a url"})),
        );
        assert_eq!((run.pages_visited.len(), run.off_site), (0, None));
    }

    #[test]
    fn each_screenshot_is_saved_into_the_staged_map_named_after_the_last_page_seen() {
        let run = map_run(|_| {}, &["/", "/Plans & Pricing"]);
        assert_eq!(
            gate_mapping_invoke(&run, SCREENSHOT_ACTION, &json!({})),
            Gate::Allow {
                save_to: Some(SaveTo {
                    dir: "/staging/screenshots".into(),
                    filename: "01-plans-pricing.png".into()
                })
            }
        );
    }

    #[test]
    fn the_home_page_is_named_home() {
        let Gate::Allow {
            save_to: Some(save_to),
        } = gate_mapping_invoke(&fresh(), SCREENSHOT_ACTION, &json!({}))
        else {
            panic!("a first screenshot is allowed and saved");
        };
        assert_eq!(save_to.filename, "01-home.png");
    }

    #[test]
    fn a_taken_screenshot_counts_against_the_budget_and_a_spent_budget_refuses_the_next() {
        let mut run = map_run(|settings| settings.max_screenshots = 1, &[]);
        note_mapping_result(
            &mut run,
            SCREENSHOT_ACTION,
            &Ok(json!({"dataUrl": "data:image/png;base64,"})),
        );
        assert_eq!(run.shots, 1);
        assert_eq!(
            verdict(&run, SCREENSHOT_ACTION, json!({})),
            "MAPPING_BUDGET"
        );
    }
}
