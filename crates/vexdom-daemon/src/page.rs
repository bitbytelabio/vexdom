//! The token features (plan step 8): `Control.snapshot` renders the content script's tree with
//! refs and answers with a text delta; refs in an action's target are rewritten to the element
//! they name (or refused); actions on a snapshotted tab come back with the delta they caused;
//! `Control.read` turns the page into markdown; screenshots can be skipped when unchanged or
//! annotated with refs.

use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;

use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use vexdom_core::config::read_agent_config;
use vexdom_core::guardrails::fence::{fence, fence_tag, should_fence};
use vexdom_core::guardrails::policy_from;
use vexdom_core::guardrails::secrets::seal_secrets;
use vexdom_core::result::{ActionError, failure};
use vexdom_schema::{read_opts, snapshot_opts};
use vexdom_snapshot::diff::diff_screenshot;
use vexdom_snapshot::{
    Baselines, RefMap, SnapshotOptions, WireSnapshot, parse_ref, render, truncate,
};

use crate::Daemon;
use crate::link::{Link, Reply};
use crate::wire::{optional, parse_json, text};

/// Baselines kept per browser (agent-browser's cap, T8).
const BASELINES: usize = 32;
/// The page gets this long to react before the after-action snapshot.
const SETTLE: Duration = Duration::from_millis(150);
/// Actions whose reply carries the delta they caused.
const DELTA_ACTIONS: &[&str] = &[
    "page.clickElement",
    "page.fillInput",
    "page.typeText",
    "page.insertText",
    "page.selectOption",
    "page.setChecked",
    "page.pushState",
    "page.pressKey",
    "page.submitForm",
    "page.navigate",
    "page.mouse",
    "page.dialog",
];
pub const UNCHANGED_SCREENSHOT: &str = "Screenshot unchanged since the last one.";

/// One browser's tabs as the agent last saw them.
#[derive(Default)]
pub(crate) struct PageState {
    refs: HashMap<i64, RefMap>,
    baselines: Option<Baselines>,
    options: HashMap<i64, SnapshotOptions>,
    last_tab: Option<i64>,
    shots: HashMap<i64, [u8; 32]>,
    /// The last image per tab, kept only for `ifChanged` with a `threshold`.
    images: HashMap<i64, Vec<u8>>,
    /// The WebMCP tools the agent was last told about.
    pub(crate) advertised: crate::webmcp::Advertised,
}

impl PageState {
    fn baselines(&mut self) -> &mut Baselines {
        self.baselines
            .get_or_insert_with(|| Baselines::new(BASELINES))
    }
}

/// `SnapshotOpts` as the daemon uses it.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct SnapshotRequest {
    pub options: SnapshotOptions,
    pub full: bool,
    pub tab_id: Option<i64>,
    pub run_id: Option<String>,
    /// Answer `{"text", "refs"}` as JSON text, the whole tree (`vexdom snapshot --json`).
    pub json: bool,
}

impl SnapshotRequest {
    pub(crate) fn read(opts: snapshot_opts::Reader<'_>) -> capnp::Result<Self> {
        Ok(Self {
            options: SnapshotOptions {
                interactive: opts.get_interactive(),
                compact: opts.get_compact(),
                depth: usize::try_from(opts.get_depth()).ok(),
                selector: optional(text(opts.get_selector())),
                urls: opts.get_urls(),
            },
            full: opts.get_full() || opts.get_json(),
            tab_id: Some(opts.get_tab_id()).filter(|tab| *tab >= 0),
            run_id: optional(text(opts.get_run_id())),
            json: opts.get_json(),
        })
    }
}

/// `ReadOpts` as the daemon uses it.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct ReadRequest {
    pub outline: bool,
    pub filter: Option<String>,
    pub max_chars: Option<usize>,
    pub target: Option<Value>,
    pub run_id: Option<String>,
}

impl ReadRequest {
    pub(crate) fn read(opts: read_opts::Reader<'_>) -> capnp::Result<Self> {
        Ok(Self {
            outline: opts.get_outline(),
            filter: optional(text(opts.get_filter())),
            max_chars: Some(opts.get_max_chars() as usize).filter(|max| *max > 0),
            target: Some(parse_json(&text(opts.get_target()))).filter(|target| !target.is_null()),
            run_id: optional(text(opts.get_run_id())),
        })
    }
}

fn offline() -> ActionError {
    ActionError {
        code: "EXTENSION_OFFLINE".into(),
        message: "The Vexdom extension is not connected — open your browser with the extension loaded, then retry".into(),
    }
}

/// The tab an input names, else the tab of this browser's latest snapshot.
fn tab_of(input: &Value, state: &PageState) -> Option<i64> {
    input
        .get("tabId")
        .and_then(Value::as_f64)
        .map(|tab| tab as i64)
        .or(state.last_tab)
}

/// A `target`-like value that names a ref, as its `eN` id.
fn ref_in(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => parse_ref(text.trim())
            .filter(|id| id.len() > 1 && id[1..].bytes().all(|b| b.is_ascii_digit())),
        Value::Object(object) => match object.get("ref") {
            Some(Value::String(text)) => parse_ref(text.trim()),
            _ => None,
        },
        _ => None,
    }
}

impl Daemon {
    pub(crate) fn with_page<T>(&self, link: &Link, change: impl FnOnce(&mut PageState) -> T) -> T {
        let mut pages = self.0.pages.borrow_mut();
        change(pages.entry(link.serial).or_default())
    }

    pub(crate) fn forget_page(&self, serial: u64) {
        self.0.pages.borrow_mut().remove(&serial);
    }

    /// Replaces refs in `target`, `from` and `to` with the element they name in this tab.
    pub(crate) fn rewrite_refs(&self, link: &Link, input: &mut Value) -> Result<(), ActionError> {
        let Some(object) = input.as_object_mut() else {
            return Ok(());
        };
        for key in ["target", "from", "to", "frame", "container"] {
            let Some(value) = object.get(key) else {
                continue;
            };
            let Some(id) = ref_in(value) else {
                // `switchFrame {"to":"top"|"parent"}` names no element.
                if let Value::String(selector) = value
                    && !(key == "to" && matches!(selector.as_str(), "top" | "parent"))
                {
                    let selector = selector.clone();
                    object.insert(key.into(), json!({ "selector": selector }));
                }
                continue;
            };
            let rewritten = self.with_page(link, |state| {
                let tab = tab_of(&Value::Object(object.clone()), state);
                let refs = tab.and_then(|tab| state.refs.get(&tab));
                let Some(refs) = refs else {
                    return failure::<Value>("NO_SNAPSHOT", "Take a snapshot first.");
                };
                let Some(entry) = refs.get(&id) else {
                    return failure("UNKNOWN_REF", format!("Unknown ref: {id}. Take a new snapshot."));
                };
                let mut target = json!({"frameId": entry.key.frame_id, "doc": entry.key.doc_key, "id": entry.key.id,
                    "role": entry.role, "name": entry.name});
                if let Some(nth) = entry.nth {
                    target["nth"] = json!(nth);
                }
                Ok(json!({ "ref": target }))
            })?;
            object.insert(key.into(), rewritten);
        }
        Ok(())
    }

    /// `Control.snapshot`: render, remember refs and the baseline, answer with the delta text.
    pub(crate) async fn snapshot(
        &self,
        target: Option<Rc<Link>>,
        request: SnapshotRequest,
    ) -> Result<String, ActionError> {
        let link = target.filter(|link| link.is_open()).ok_or_else(offline)?;
        let mut input = json!({"interactive": request.options.interactive, "includeFrames": true});
        if let Some(selector) = &request.options.selector {
            input["selector"] = json!(selector);
        }
        let reply = link
            .invoke(
                "page.snapshot",
                &input,
                request.tab_id,
                request.run_id.as_deref(),
            )
            .await;
        let raw = reply.result?;
        let tab = raw
            .get("tabId")
            .and_then(Value::as_f64)
            .map(|tab| tab as i64)
            .or(request.tab_id)
            .unwrap_or(-1);
        let snapshot: WireSnapshot = serde_json::from_value(raw).map_err(|error| ActionError {
            code: "BAD_SNAPSHOT".into(),
            message: format!("The page's snapshot could not be read: {error}"),
        })?;
        let text = self.with_page(&link, |state| {
            let refs = state.refs.entry(tab).or_default();
            let tree = render(&snapshot, 0, &request.options, refs);
            state.options.insert(tab, request.options.clone());
            state.last_tab = Some(tab);
            let url = snapshot.url.clone();
            let text = state
                .baselines()
                .reply(tab, &url, &request.options, &tree, request.full);
            if !request.json {
                return text;
            }
            let refs: Vec<Value> = state
                .refs
                .get(&tab)
                .map(|refs| {
                    refs.entries_sorted()
                        .into_iter()
                        .map(|(id, entry)| json!({"ref": id, "role": entry.role, "name": entry.name}))
                        .collect()
                })
                .unwrap_or_default();
            json!({"text": text, "refs": refs}).to_string()
        });
        Ok(seal_secrets(&text))
    }

    /// `Control.read`: the page (or one element) as markdown; HTML never leaves the daemon.
    pub(crate) async fn read(
        &self,
        target: Option<Rc<Link>>,
        request: ReadRequest,
    ) -> Result<String, ActionError> {
        let link = target.filter(|link| link.is_open()).ok_or_else(offline)?;
        let mut input = json!({});
        if let Some(target) = &request.target {
            input["target"] = target.clone();
            self.rewrite_refs(&link, &mut input)?;
        }
        let reply = link
            .invoke("page.readHtml", &input, None, request.run_id.as_deref())
            .await;
        let data = reply.result?;
        let html = data.get("html").and_then(Value::as_str).unwrap_or("");
        let markdown = crate::markdown::html_to_markdown(html);
        let shaped = if request.outline {
            outline(&markdown)
        } else if let Some(filter) = &request.filter {
            filter_sections(&markdown, filter)
        } else {
            markdown
        };
        let shaped = truncate(&shaped, request.max_chars);
        let sealed = seal_secrets(&shaped);
        let config = read_agent_config();
        let policy = policy_from(&config.guardrails, &config.require_approval);
        Ok(if should_fence("page.readHtml", &policy) {
            fence(&sealed, &fence_tag())
        } else {
            sealed
        })
    }

    /// A page action from a control client: refs rewritten, guardrails, screenshot flags, and
    /// the delta the action caused when the tab has a baseline.
    pub(crate) async fn page_invoke(
        &self,
        target: Option<Rc<Link>>,
        action: &str,
        mut input: Value,
        client: &str,
    ) -> Reply {
        let Some(link) = target.clone().filter(|link| link.is_open()) else {
            return self.invoke_external(target, action, input, client).await;
        };
        if let Err(error) = self.rewrite_refs(&link, &mut input) {
            return Err(error).into();
        }
        if action == "page.screenshot" {
            return self.screenshot(link, input, client).await;
        }
        let tab = self.with_page(&link, |state| tab_of(&input, state));
        let reply = self
            .invoke_external(Some(Rc::clone(&link)), action, input, client)
            .await;
        let mut reply = self.with_delta(&link, action, tab, reply).await;
        if !crate::webmcp::ANNOUNCE_ACTIONS.contains(&action) {
            return reply;
        }
        let Ok(data) = &mut reply.result else {
            return reply;
        };
        // A new or switched-to tab is the one the reply names.
        let announced = data
            .get("tabId")
            .or_else(|| data.pointer("/result/tabId"))
            .and_then(Value::as_i64)
            .or(tab);
        if let Some(context) = self.webmcp_context(&link, announced).await
            && let Some(object) = data.as_object_mut()
        {
            object.insert("webmcp".into(), context);
        }
        reply
    }

    /// The reply with the delta the action caused, when its tab has a baseline.
    async fn with_delta(
        &self,
        link: &Rc<Link>,
        action: &str,
        tab: Option<i64>,
        reply: Reply,
    ) -> Reply {
        if !DELTA_ACTIONS.contains(&action) {
            return reply;
        }
        let Ok(data) = &reply.result else {
            return reply;
        };
        let Some(tab) = tab else { return reply };
        let options = self.with_page(link, |state| state.options.get(&tab).cloned());
        let Some(options) = options else { return reply };
        tokio::time::sleep(SETTLE).await;
        let request = SnapshotRequest {
            options,
            full: false,
            tab_id: Some(tab),
            run_id: None,
            json: false,
        };
        match self.snapshot(Some(Rc::clone(link)), request).await {
            Ok(delta) => Reply {
                result: Ok(json!({"result": data, "snapshot": delta})),
                image: None,
            },
            Err(_) => reply,
        }
    }

    /// `Control.snapshot`: the snapshot, and the WebMCP context when the tab's tools changed.
    pub(crate) async fn snapshot_announced(
        &self,
        target: Option<Rc<Link>>,
        request: SnapshotRequest,
    ) -> Result<(String, Option<Value>), ActionError> {
        let link = target.filter(|link| link.is_open()).ok_or_else(offline)?;
        let text = self.snapshot(Some(Rc::clone(&link)), request).await?;
        let tab = self
            .with_page(&link, |state| state.last_tab)
            .filter(|tab| *tab >= 0);
        Ok((text, self.webmcp_context(&link, tab).await))
    }

    async fn screenshot(&self, link: Rc<Link>, mut input: Value, client: &str) -> Reply {
        let if_changed = input.get("ifChanged") == Some(&Value::Bool(true));
        let annotate = input.get("annotate") == Some(&Value::Bool(true));
        let threshold = input
            .get("threshold")
            .and_then(Value::as_f64)
            .filter(|threshold| threshold.is_finite() && *threshold >= 0.0);
        if let Some(object) = input.as_object_mut() {
            object.shift_remove("ifChanged");
            object.shift_remove("annotate");
            object.shift_remove("threshold");
        }
        let tab = self.with_page(&link, |state| tab_of(&input, state));
        if annotate {
            let options = self.with_page(&link, |state| {
                tab.and_then(|tab| state.options.get(&tab).cloned())
            });
            let request = SnapshotRequest {
                options: options.unwrap_or(SnapshotOptions {
                    interactive: true,
                    ..SnapshotOptions::default()
                }),
                full: false,
                tab_id: tab,
                run_id: None,
                json: false,
            };
            if self.snapshot(Some(Rc::clone(&link)), request).await.is_ok() {
                let labels: Vec<Value> = self.with_page(&link, |state| {
                    let tab = tab.or(state.last_tab);
                    tab.and_then(|tab| state.refs.get(&tab))
                        .map(|refs| {
                            refs.entries_sorted()
                                .into_iter()
                                .map(|(id, entry)| {
                                    json!({"frameId": entry.key.frame_id, "doc": entry.key.doc_key, "id": entry.key.id, "label": id})
                                })
                                .collect()
                        })
                        .unwrap_or_default()
                });
                input["labels"] = Value::Array(labels);
            }
        }
        let reply = self
            .invoke_external(Some(Rc::clone(&link)), "page.screenshot", input, client)
            .await;
        // Every capture is remembered, so `ifChanged` compares with the last one whoever asked.
        let (Ok(_), Some(image)) = (&reply.result, &reply.image) else {
            return reply;
        };
        let digest: [u8; 32] = Sha256::digest(image).into();
        let key = tab.unwrap_or(-1);
        let mut unchanged = self.with_page(&link, |state| {
            state.shots.insert(key, digest) == Some(digest)
        });
        // A threshold compares pixels with the previous image: a capture that differs by no more
        // than that fraction of its pixels counts as unchanged.
        if if_changed && let Some(threshold) = threshold {
            let previous = self.with_page(&link, |state| state.images.insert(key, image.clone()));
            if !unchanged
                && let Some(previous) = previous
                && let Ok(diff) = diff_screenshot(&previous, image, 0.1)
            {
                unchanged = diff.mismatch_percentage / 100.0 <= threshold;
            }
        }
        if if_changed && unchanged {
            return Ok(Value::String(UNCHANGED_SCREENSHOT.into())).into();
        }
        reply
    }
}

/// Headings only, each with its line number in the full text.
pub fn outline(markdown: &str) -> String {
    markdown
        .lines()
        .enumerate()
        .filter(|(_, line)| line.starts_with('#'))
        .map(|(at, line)| format!("{}: {line}", at + 1))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Sections (a heading and what follows until the next heading) whose heading or text contains
/// `needle`, case-insensitively.
pub fn filter_sections(markdown: &str, needle: &str) -> String {
    let needle = needle.to_lowercase();
    let mut sections: Vec<Vec<&str>> = vec![Vec::new()];
    for line in markdown.lines() {
        if line.starts_with('#') {
            sections.push(Vec::new());
        }
        if let Some(section) = sections.last_mut() {
            section.push(line);
        }
    }
    let kept: Vec<String> = sections
        .into_iter()
        .map(|lines| lines.join("\n"))
        .filter(|section| !section.trim().is_empty() && section.to_lowercase().contains(&needle))
        .collect();
    if kept.is_empty() {
        format!("No section mentions \"{needle}\".")
    } else {
        kept.join("\n\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outlines_and_filters() {
        let text = "# Title\nintro\n\n## Pricing\nfree tier\n\n## Docs\nread me";
        assert_eq!(outline(text), "1: # Title\n4: ## Pricing\n7: ## Docs");
        assert_eq!(filter_sections(text, "FREE"), "## Pricing\nfree tier\n");
        assert_eq!(filter_sections(text, "zzz"), "No section mentions \"zzz\".");
    }

    #[test]
    fn refs_in_targets() {
        assert_eq!(ref_in(&json!("@e5")).as_deref(), Some("e5"));
        assert_eq!(ref_in(&json!("e12")).as_deref(), Some("e12"));
        assert_eq!(ref_in(&json!({"ref": "ref=e3"})).as_deref(), Some("e3"));
        assert_eq!(ref_in(&json!("#email")), None);
        assert_eq!(ref_in(&json!({"selector": "#a"})), None);
    }
}
