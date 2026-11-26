//! The `bridge.flow*` actions: the Bridge's half of agent-browser's `flow_*` handlers
//! (`native/actions.rs`), over the extension link instead of a CDP session.

use std::rc::Rc;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use vexdom_core::log;
use vexdom_core::result::{ActionError, ActionResult, failure};

use super::outcome::{RunOutcome, TierResult, finish_run, unverify_on_shape_change};
use super::recorder::SaveArgs;
use super::replay::ReplayRun;
use super::send::{Browser, SendCtx, Senders, run_requests};
use super::{
    AllowWrite, Flow, HarEntry, HostPolicy, PreparedRequest, Shape, Tier, Write, action_of,
    flows_dir, har_entries, list_flows, load_flow, page_tier, parse_params, prepare, replay,
    unknown_fields, validate_name, write_flow_in,
};
use crate::Daemon;
use crate::agent::InvokeOpts;
use crate::confirmations::Request as Confirmation;
use crate::link::{Link, Reply};

/// How long a save waits for the page to go quiet and a run may take, unless the call says.
const DEFAULT_TIMEOUT_MS: u64 = 25_000;

const NOT_CAPTURING: &str = "no flow recording on this tab; start one with `vexdom flow record start`, run the task, then `vexdom flow record stop`";

fn error(code: &str, message: impl Into<String>) -> ActionError {
    ActionError {
        code: code.to_owned(),
        message: message.into(),
    }
}

fn invalid(message: impl Into<String>) -> ActionError {
    error("INVALID_INPUT", message)
}

fn offline() -> ActionError {
    error(
        "EXTENSION_OFFLINE",
        "The Vexdom extension is not connected — open your browser with the extension loaded, then retry",
    )
}

fn open(link: Option<Rc<Link>>) -> Result<Rc<Link>, ActionError> {
    link.filter(|link| link.is_open()).ok_or_else(offline)
}

/// The tab a call names, else the run's.
fn tab_hint(input: &Value, opts: &InvokeOpts) -> Option<i64> {
    input.get("tabId").and_then(Value::as_i64).or(opts.tab_id)
}

fn timeout_of(input: &Value) -> Duration {
    Duration::from_millis(
        input
            .get("timeoutMs")
            .and_then(Value::as_u64)
            .filter(|ms| *ms > 0)
            .unwrap_or(DEFAULT_TIMEOUT_MS),
    )
}

/// The id of the tab `tab` names (the active tab for `None`).
async fn resolve_tab(
    link: &Link,
    tab: Option<i64>,
    run_id: Option<&str>,
) -> Result<i64, ActionError> {
    let listed = link
        .invoke("page.switchTab", &json!({}), tab, run_id)
        .await
        .result?;
    listed
        .get("activeTabId")
        .and_then(Value::as_i64)
        .ok_or_else(|| error("NO_ACTIVE_TAB", "No active tab to control"))
}

async fn tab_url(link: &Link, tab: i64, run_id: Option<&str>) -> Option<String> {
    link.invoke("page.query", &json!({"what": "url"}), Some(tab), run_id)
        .await
        .result
        .ok()
        .and_then(|data| data.get("value").and_then(Value::as_str).map(String::from))
}

/// Reads the tab's HAR so far into its recording and starts a new one, so a save sees the
/// requests of a recording that keeps running.
async fn checkpoint(daemon: &Daemon, link: &Link, tab: i64, wait: Duration) {
    let quiet =
        json!({"load": "networkidle", "timeoutMs": wait.as_millis().clamp(100, 60_000) as u64});
    let _ = link.invoke("page.waitFor", &quiet, Some(tab), None).await;
    let stopped = link
        .invoke("page.harStop", &json!({"inline": true}), Some(tab), None)
        .await;
    if let Ok(data) = &stopped.result
        && let Some(recording) = daemon
            .0
            .flows
            .borrow_mut()
            .list
            .iter_mut()
            .find(|r| r.install_id == link.install_id && r.tab == tab)
    {
        recording.add_har(&data["har"]);
    }
    let restarted = link
        .invoke(
            "page.harStart",
            &json!({"content": "text"}),
            Some(tab),
            None,
        )
        .await;
    if let Err(e) = restarted.result {
        log::log(&format!("flow recording on tab {tab} lost its HAR: {e}"));
    }
}

/// `bridge.flowRecord|flowSteps|flowSave|flowRun|flowList`.
pub(crate) async fn handle(
    daemon: &Daemon,
    link: Option<Rc<Link>>,
    action: &str,
    input: Value,
    opts: &InvokeOpts,
) -> Reply {
    let result = match action {
        "bridge.flowRecord" => record(daemon, link, &input, opts).await,
        "bridge.flowSteps" => steps(daemon, &input, opts),
        "bridge.flowSave" => save(daemon, link, &input, opts).await,
        "bridge.flowRun" => run(daemon, link, &input, opts).await,
        "bridge.flowList" => Ok(json!({
            "flows": list_flows(&flows_dir(), input.get("origin").and_then(Value::as_str)),
        })),
        _ => failure("UNKNOWN_ACTION", format!("Unknown action \"{action}\".")),
    };
    result.into()
}

async fn record(
    daemon: &Daemon,
    link: Option<Rc<Link>>,
    input: &Value,
    opts: &InvokeOpts,
) -> ActionResult {
    let run_id = opts.run_id.as_deref();
    match input.get("op").and_then(Value::as_str).unwrap_or("status") {
        "start" => {
            let link = open(link)?;
            let tab = resolve_tab(&link, tab_hint(input, opts), run_id).await?;
            let running =
                daemon.0.flows.borrow().list.iter().any(|r| {
                    r.install_id == link.install_id && r.tab == tab && r.recorder.active()
                });
            if running {
                return failure(
                    "HAR_ACTIVE",
                    format!("A flow recording is already running on tab {tab}; stop it first."),
                );
            }
            let page_url = tab_url(&link, tab, run_id).await;
            link.invoke(
                "page.harStart",
                &json!({"content": "text"}),
                Some(tab),
                run_id,
            )
            .await
            .result?;
            daemon
                .0
                .flows
                .borrow_mut()
                .start(&link.install_id, tab, page_url.clone());
            Ok(json!({"started": true, "tabId": tab, "url": page_url}))
        }
        "stop" => {
            let (install_id, tab) = {
                let flows = daemon.0.flows.borrow();
                let index = flows
                    .find(tab_hint(input, opts), true)
                    .ok_or_else(|| error("FLOW_NOT_CAPTURING", NOT_CAPTURING))?;
                (flows.list[index].install_id.clone(), flows.list[index].tab)
            };
            let link = open(daemon.link(&install_id))?;
            let stopped = link
                .invoke("page.harStop", &json!({"inline": true}), Some(tab), run_id)
                .await;
            let har = match stopped.result {
                Ok(data) => Some(data["har"].clone()),
                // The debugger left the tab: what was read before stays.
                Err(e) if e.code == "NO_HAR_RECORDING" => None,
                Err(e) => return Err(e),
            };
            let mut flows = daemon.0.flows.borrow_mut();
            let recording = flows
                .list
                .iter_mut()
                .find(|r| r.install_id == install_id && r.tab == tab)
                .ok_or_else(|| error("FLOW_NOT_CAPTURING", NOT_CAPTURING))?;
            if let Some(har) = &har {
                recording.add_har(har);
            }
            let unsaved = recording.recorder.end_capture();
            Ok(json!({
                "stopped": true,
                "tabId": tab,
                "steps": recording.recorder.commands.len(),
                "requests": recording.entries.len(),
                "unsaved": unsaved,
            }))
        }
        "status" => {
            let flows = daemon.0.flows.borrow();
            Ok(json!({"recordings": flows.list.iter().map(|r| r.summary()).collect::<Vec<_>>()}))
        }
        other => Err(invalid(format!(
            "op must be \"start\", \"stop\" or \"status\", not \"{other}\""
        ))),
    }
}

fn steps(daemon: &Daemon, input: &Value, opts: &InvokeOpts) -> ActionResult {
    let flows = daemon.0.flows.borrow();
    let index = flows
        .find(tab_hint(input, opts), false)
        .ok_or_else(|| error("FLOW_NOT_CAPTURING", NOT_CAPTURING))?;
    Ok(flows.list[index].recorder.steps())
}

async fn save(
    daemon: &Daemon,
    link: Option<Rc<Link>>,
    input: &Value,
    opts: &InvokeOpts,
) -> ActionResult {
    let name = input
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| invalid("flow save requires a name"))?
        .to_string();
    validate_name(&name).map_err(invalid)?;
    let params = parse_params(input).map_err(invalid)?;
    let (install_id, tab, capturing) = {
        let flows = daemon.0.flows.borrow();
        let index = flows
            .find(tab_hint(input, opts), false)
            .ok_or_else(|| error("FLOW_NOT_CAPTURING", NOT_CAPTURING))?;
        let recording = &flows.list[index];
        (
            recording.install_id.clone(),
            recording.tab,
            recording.recorder.active(),
        )
    };
    let live = daemon
        .link(&install_id)
        .or(link)
        .filter(|link| link.is_open() && link.install_id == install_id);
    let mut login_wall = false;
    if let Some(link) = &live {
        if capturing {
            checkpoint(daemon, link, tab, timeout_of(input)).await;
        }
        let probe = json!({
            "code": "[...document.querySelectorAll('input[type=password]')].some(e => e.offsetParent !== null)",
            "timeoutMs": 2000,
        });
        login_wall = link
            .invoke("page.evaluate", &probe, Some(tab), None)
            .await
            .result
            .ok()
            .and_then(|data| data.get("value").and_then(Value::as_bool))
            .unwrap_or(false);
    }
    let from = input.get("from").and_then(Value::as_u64);
    let to = input.get("to").and_then(Value::as_u64);
    let mut flows = daemon.0.flows.borrow_mut();
    let recording = flows
        .list
        .iter_mut()
        .find(|r| r.install_id == install_id && r.tab == tab)
        .ok_or_else(|| error("FLOW_NOT_CAPTURING", NOT_CAPTURING))?;
    let still_loading = recording
        .recorder
        .loading_gaps(from, to, &recording.entries);
    let entries: Vec<HarEntry> = recording.entries.clone();
    let saved = recording.recorder.save(
        &flows_dir(),
        SaveArgs {
            name: &name,
            from,
            to,
            description: input.get("description").and_then(Value::as_str),
            params: &params,
            output: input.get("output").and_then(Value::as_str),
            entries: &entries,
            login_wall,
        },
    );
    // A save that fails for want of requests names the ones still loading.
    let mut report = match saved {
        Ok(report) => report,
        Err(e) if !still_loading.is_empty() => {
            let loading: Vec<String> = still_loading
                .iter()
                .filter_map(|g| Some(format!("step {}: {}", g["step"], g["reason"].as_str()?)))
                .collect();
            return failure("ACTION_FAILED", format!("{e}; {}", loading.join("; ")));
        }
        Err(e) => return failure("ACTION_FAILED", e),
    };
    // Requests still loading when the HAR was read are not in the flow. The step
    // that started one says so instead of the generic reason.
    if let Some(gaps) = report["gaps"].as_array_mut() {
        gaps.retain(|g| {
            g["kind"] != "no-requests" || !still_loading.iter().any(|l| l["step"] == g["step"])
        });
        gaps.extend(still_loading);
        for g in gaps.iter_mut() {
            if let Some(g) = g.as_object_mut() {
                g.shift_remove("kind");
            }
        }
    }
    Ok(report)
}

fn with_save_note(message: String, saved: Option<&str>) -> String {
    match saved {
        Some(e) => format!("{message} (the flow file was not updated: {e})"),
        None => message,
    }
}

async fn run(
    daemon: &Daemon,
    link: Option<Rc<Link>>,
    input: &Value,
    opts: &InvokeOpts,
) -> ActionResult {
    let started = Instant::now();
    let run_deadline = started + timeout_of(input);
    let name = input
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| invalid("flow run requires a name"))?;
    let dir = flows_dir();
    let mut flow = load_flow(&dir, name, input.get("origin").and_then(Value::as_str))
        .map_err(|(code, message)| error(code, message))?;
    let name = flow.name.clone();
    let params = parse_params(input).map_err(invalid)?;
    let requests = prepare(&flow, &params).map_err(invalid)?;
    let allow = AllowWrite::parse(input.get("allowWrite")).map_err(invalid)?;
    allow.check(&flow).map_err(invalid)?;
    let hosts = HostPolicy::of(&flow);
    // Every request is checked before the first one is sent, on its template
    // with references unfilled; each is checked again once they are filled.
    for prepared in &requests {
        hosts.check(&prepared.url).map_err(|e| {
            error(
                "ACTION_FAILED",
                format!("flow '{name}' step {}: {e}", prepared.step),
            )
        })?;
    }
    let link = open(link)?;
    // The write gate: a write the call did not allow waits for the user (ADR-032).
    let mut approved = Vec::new();
    for w in flow.run_writes() {
        if allow.covers(&w.id) {
            continue;
        }
        daemon
            .await_confirmation(Confirmation {
                action: "bridge.flowRun",
                summary: format!("flow '{name}' sends {} {}", w.method, w.url),
                site: Some(flow.origin.clone()),
                input,
                approver: opts.approver.clone(),
            })
            .await?;
        approved.push(w.id.clone());
    }
    let run_id = opts.run_id.clone();
    let tab = resolve_tab(&link, tab_hint(input, opts), run_id.as_deref()).await?;
    let run_began = log::now_ms() as f64;
    let results = {
        let mut senders = LinkSenders {
            browser: LinkBrowser {
                daemon,
                link: Rc::clone(&link),
                tab,
                run_id,
                allow: &allow,
                approved,
                name: name.clone(),
                origin: flow.origin.clone(),
                input,
                approver: opts.approver.clone(),
                paused: false,
                next_entry: 0,
            },
            flow: &flow,
            params: &params,
            policy: &hosts,
            replay_run: None,
            run_deadline,
        };
        run_requests(&flow, &requests, &mut senders).await
    };
    // What the run sent from the tab is not part of a recording there.
    if let Some(recording) = daemon
        .0
        .flows
        .borrow_mut()
        .list
        .iter_mut()
        .find(|r| r.install_id == link.install_id && r.tab == tab)
    {
        recording
            .recorder
            .untag_between(run_began, log::now_ms() as f64);
    }
    let results = results?;
    finish(&dir, &mut flow, &results, input, started)
}

/// The run's own result comes first: writes it sent happened whether or not
/// the flow file can be updated, so a save error never hides it.
fn finish(
    dir: &std::path::Path,
    flow: &mut Flow,
    results: &[TierResult],
    input: &Value,
    started: Instant,
) -> ActionResult {
    let name = flow.name.clone();
    if let Some(outcome) = RunOutcome::stopped(results) {
        let saved = if unverify_on_shape_change(flow, results) {
            write_flow_in(dir, flow).err()
        } else {
            None
        };
        return Err(error(
            outcome.code(),
            with_save_note(outcome.message(&name), saved.as_deref()),
        ));
    }
    let shape = Shape::from_cmd(input);
    if let Some(body) = results
        .iter()
        .find(|r| r.ok && r.id == flow.output)
        .and_then(|r| r.body.as_ref())
    {
        let unknown = unknown_fields(body, &shape);
        if !unknown.is_empty() {
            return Err(invalid(format!(
                "no item has field(s) {}. Fields: {}",
                unknown.join(", "),
                flow.item_fields.join(", ")
            )));
        }
    }
    let mut response = finish_run(
        flow,
        results,
        &log::iso_now(),
        started.elapsed().as_millis(),
        &shape,
    );
    let saved = write_flow_in(dir, flow).err();
    if response["ok"] != json!(true) {
        let outcome = RunOutcome::failed(results, flow);
        return Err(error(
            outcome.code(),
            with_save_note(outcome.message(&name), saved.as_deref()),
        ));
    }
    if let Some(e) = saved {
        response["saveError"] = json!(e);
    }
    Ok(response)
}

/// The settle wait in the page: the requests at `want` answered after `since`, then two quiet
/// polls, within `limit` ms (`appear` ms for the requests to show up).
fn settle_script(want: &[(String, String)], since: i64, appear: u128, limit: u128) -> String {
    format!(
        r#"(async () => {{
    const want = {want};
    const since = {since};
    const appearBy = Date.now() + {appear};
    const until = Date.now() + {limit};
    const same = (w, g) => {{
        if (w[0] !== g[0]) return false;
        const ws = w[1].split('/'), gs = g[1].split('/');
        return ws.length === gs.length && ws.every((s, i) => s === gs[i] || (s.includes('%7B%7B') && gs[i] !== ''));
    }};
    const seen = () => performance.getEntriesByType('resource')
        .filter((e) => performance.timeOrigin + e.startTime >= since - 50)
        .map((e) => {{ try {{ const u = new URL(e.name); return [u.hostname, u.pathname]; }} catch {{ return null; }} }})
        .filter(Boolean);
    let quiet = 0, count = -1;
    for (;;) {{
        const got = seen();
        const appeared = want.every((w) => got.some((g) => same(w, g)));
        const waiting = !appeared && Date.now() < appearBy;
        quiet = !waiting && got.length === count ? quiet + 1 : 0;
        count = got.length;
        if (quiet >= 2 || Date.now() >= until) return {{ appeared, seen: got.length }};
        await new Promise((r) => setTimeout(r, 100));
    }}
}})()"#,
        want = json!(want),
    )
}

/// The flow's tab over the extension link.
struct LinkBrowser<'run> {
    daemon: &'run Daemon,
    link: Rc<Link>,
    tab: i64,
    run_id: Option<String>,
    allow: &'run AllowWrite,
    /// Writes the user approved for this run.
    approved: Vec<String>,
    name: String,
    origin: String,
    input: &'run Value,
    approver: Option<String>,
    /// A recording on this tab whose HAR the replay borrowed.
    paused: bool,
    next_entry: u64,
}

impl LinkBrowser<'_> {
    async fn invoke(&self, action: &str, input: &Value) -> ActionResult {
        self.link
            .invoke(action, input, Some(self.tab), self.run_id.as_deref())
            .await
            .result
    }

    fn recording_here(&self) -> bool {
        self.daemon.0.flows.borrow().list.iter().any(|r| {
            r.install_id == self.link.install_id && r.tab == self.tab && r.recorder.active()
        })
    }
}

impl Browser for LinkBrowser<'_> {
    async fn tab_url(&mut self) -> Option<String> {
        tab_url(&self.link, self.tab, self.run_id.as_deref()).await
    }

    async fn eval(&mut self, script: &str, timeout: Duration) -> Result<Value, ActionError> {
        let input =
            json!({"code": script, "timeoutMs": timeout.as_millis().clamp(100, 120_000) as u64});
        let data = self.invoke("page.evaluate", &input).await?;
        Ok(data.get("value").cloned().unwrap_or(Value::Null))
    }

    async fn begin_capture(&mut self) -> Result<(), String> {
        // A recording on this tab lends its HAR to the replay and gets it back after.
        if self.recording_here() {
            let stopped = self.invoke("page.harStop", &json!({"inline": true})).await;
            if let Ok(data) = &stopped
                && let Some(recording) = self
                    .daemon
                    .0
                    .flows
                    .borrow_mut()
                    .list
                    .iter_mut()
                    .find(|r| r.install_id == self.link.install_id && r.tab == self.tab)
            {
                recording.add_har(&data["har"]);
            }
            self.paused = stopped.is_ok();
        }
        self.invoke("page.harStart", &json!({"content": "text"}))
            .await
            .map(|_| ())
            .map_err(|e| format!("the replay could not record the tab's requests: {e}"))
    }

    async fn run(&mut self, cmd: &Value) -> ActionResult {
        let action = format!("page.{}", action_of(cmd));
        let mut input = cmd.clone();
        if let Some(object) = input.as_object_mut() {
            object.shift_remove("do");
            object.shift_remove("redacted");
        }
        self.invoke(&action, &input).await
    }

    async fn settle(
        &mut self,
        expected: &[(String, String)],
        since_ms: i64,
        appear: Duration,
        deadline: Instant,
    ) {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return;
        }
        let script = settle_script(expected, since_ms, appear.as_millis(), left.as_millis());
        if self.eval(&script, left).await.is_ok() {
            return;
        }
        // No page script: wait for the network to go quiet instead.
        let quiet =
            json!({"load": "networkidle", "timeoutMs": left.as_millis().clamp(100, 60_000) as u64});
        let _ = self.invoke("page.waitFor", &quiet).await;
    }

    async fn end_capture(&mut self) -> Vec<HarEntry> {
        // Bodies are fetched as their requests finish; let the last land.
        tokio::time::sleep(Duration::from_millis(300)).await;
        let stopped = self.invoke("page.harStop", &json!({"inline": true})).await;
        let entries = match &stopped {
            Ok(data) => har_entries(&data["har"], &mut self.next_entry),
            Err(_) => Vec::new(),
        };
        if self.paused {
            self.paused = false;
            if let Err(e) = self
                .invoke("page.harStart", &json!({"content": "text"}))
                .await
            {
                log::log(&format!(
                    "flow recording on tab {} lost its HAR after a replay: {e}",
                    self.tab
                ));
            }
        }
        entries
    }

    async fn write_approved(&mut self, write: &Write) -> Result<(), String> {
        if self.allow.covers(&write.id) || self.approved.contains(&write.id) {
            return Ok(());
        }
        self.daemon
            .await_confirmation(Confirmation {
                action: "bridge.flowRun",
                summary: format!("flow '{}' sends {} {}", self.name, write.method, write.url),
                site: Some(self.origin.clone()),
                input: self.input,
                approver: self.approver.clone(),
            })
            .await
            .map_err(|e| e.to_string())?;
        self.approved.push(write.id.clone());
        Ok(())
    }
}

/// One sender per tier over the same tab.
struct LinkSenders<'run> {
    browser: LinkBrowser<'run>,
    flow: &'run Flow,
    params: &'run [(String, String)],
    policy: &'run HostPolicy,
    replay_run: Option<ReplayRun>,
    /// When the run must be done, so the caller is still waiting for it.
    run_deadline: Instant,
}

impl Senders for LinkSenders<'_> {
    async fn send(&mut self, tier: Tier, req: &PreparedRequest, ctx: &SendCtx<'_>) -> TierResult {
        match tier {
            Tier::Page => {
                page_tier::send(self.flow, &mut self.browser, req, self.run_deadline).await
            }
            Tier::Replay => {
                replay::send(
                    self.flow,
                    self.params,
                    &mut self.browser,
                    &mut self.replay_run,
                    req,
                    ctx,
                    self.run_deadline,
                )
                .await
            }
        }
    }

    fn replay_ran(&self) -> bool {
        self.replay_run.is_some()
    }

    async fn check(&mut self, req: &PreparedRequest) -> Result<(), ActionError> {
        let name = &self.flow.name;
        self.policy.check(&req.url).map_err(|e| {
            error(
                "ACTION_FAILED",
                format!("flow '{name}' step {}: {e}", req.step),
            )
        })?;
        // The blocked-sites list lives in the browser.
        self.browser
            .invoke("page.checkSites", &json!({"urls": [req.url]}))
            .await
            .map(|_| ())
            .map_err(|e| {
                error(
                    &e.code,
                    format!("flow '{name}' step {}: {}", req.step, e.message),
                )
            })
    }
}
