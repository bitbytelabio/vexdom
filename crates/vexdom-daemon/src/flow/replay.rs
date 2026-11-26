//! The replay tier: the recorded commands run once in the flow's tab, requests read from what
//! they sent.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use vexdom_core::result::ActionError;
use vexdom_core::tools;

use super::outcome::{Failure, TierResult, tier_result, with_retry_after};
use super::send::{Browser, SendCtx};
use super::template::{self, Slot};
use super::{
    Flow, HarEntry, HostPolicy, PreparedRequest, Request, Step, Tier, action_of, is_redacted_url,
    is_telemetry, url_encode,
};

/// The recorded commands with the run's params swapped in. A param whose
/// recorded value no command contains cannot be replayed: the browser would
/// answer with the recorded value instead.
pub(crate) fn replay_commands(
    flow: &Flow,
    cmd_params: &[(String, String)],
) -> Result<Vec<Value>, String> {
    let given = |name: &str| cmd_params.iter().find(|(k, _)| k == name).map(|(_, v)| v);
    // The params this run changes, as (name, recorded value): wherever the
    // recorded value stands in a command it becomes the param's placeholder.
    let swaps: Vec<(String, String)> = flow
        .params
        .iter()
        .filter_map(|p| {
            let example = p.example.as_deref()?;
            let value = given(&p.name)?;
            (!example.is_empty() && example != value.as_str())
                .then(|| (p.name.clone(), example.to_string()))
        })
        .collect();
    // Every placeholder takes the run's value, or the recorded example when
    // the run gives none.
    let values: HashMap<String, String> = flow
        .params
        .iter()
        .filter_map(|p| {
            let value = given(&p.name).cloned().or_else(|| p.example.clone())?;
            Some((p.name.clone(), value))
        })
        .collect();
    let opens_page = flow
        .steps
        .iter()
        .any(|s| action_of(&s.command) == "navigate");
    let mut recorded = serde_json::to_string(
        &flow
            .steps
            .iter()
            .map(|s| s.command.clone())
            .collect::<Vec<_>>(),
    )
    .unwrap_or_default();
    if !opens_page {
        // A flow recorded on an already open page starts from that page, so its
        // start URL is part of what the replay sends.
        recorded.push(' ');
        recorded.push_str(&flow.start_url);
    }
    for p in &flow.params {
        if given(&p.name).is_none()
            && p.example.is_none()
            && recorded.contains(&format!("{{{{{}}}}}", p.name))
        {
            return Err(format!(
                "missing param {}: the recording did not keep the typed value",
                p.name
            ));
        }
    }
    let in_commands = |text: &str| recorded.contains(text) || recorded.contains(&url_encode(text));
    if let Some((name, example)) = swaps.iter().find(|(name, example)| {
        let named = recorded.contains(&format!("{{{{{name}}}}}"));
        !named && !in_commands(example)
    }) {
        return Err(format!(
            "param '{name}' is not in the recorded commands, so a replay would return the recorded value {example}"
        ));
    }
    if flow.steps.iter().any(|s| s.command["redacted"] == true) {
        return Err(
            "the recording typed a secret, which is not stored, so the commands cannot be replayed"
                .to_string(),
        );
    }
    // The replay opens the start URL itself; one whose credential was not
    // stored would land on a login or error page.
    if !opens_page && is_redacted_url(&flow.start_url) {
        return Err(
            "the start URL carried a credential, which is not stored, so the commands cannot be replayed"
                .to_string(),
        );
    }
    Ok(replayed_steps(flow)
        .into_iter()
        .map(|step| match step {
            Some(step) => step.command.clone(),
            // A flow recorded on an already open page starts with the URL it
            // was recorded on.
            None => json!({ "do": "navigate", "url": flow.start_url }),
        })
        .map(|cmd| render_command(&interaction_only(cmd), None, &swaps, &values))
        .collect())
}

/// The recorded step behind each command a replay sends, in order. The
/// navigation a replay adds for a flow recorded on an already open page has
/// no step.
fn replayed_steps(flow: &Flow) -> Vec<Option<&Step>> {
    let opens_page = flow
        .steps
        .iter()
        .any(|s| action_of(&s.command) == "navigate");
    let mut steps: Vec<Option<&Step>> = if opens_page { Vec::new() } else { vec![None] };
    steps.extend(flow.steps.iter().filter(|s| s.ok).map(Some));
    steps
}

/// A recorded command with only the fields its action declares (plus any script field, so the
/// replay check still refuses the command instead of running it without one).
fn interaction_only(mut cmd: Value) -> Value {
    let action = format!("page.{}", action_of(&cmd));
    let declared: Vec<String> = tools::chromium()
        .tool(&action)
        .and_then(|tool| tool.pointer("/inputSchema/properties"))
        .and_then(Value::as_object)
        .map(|props| props.keys().cloned().collect())
        .unwrap_or_default();
    if let Some(obj) = cmd.as_object_mut() {
        obj.retain(|k, _| {
            k == "do" || declared.iter().any(|d| d == k) || SCRIPT_KEYS.contains(&k.as_str())
        });
    }
    cmd
}

/// One command with the run's values in place: every string is read once,
/// the recorded values become placeholders, and each placeholder is filled
/// with the value encoded for where it lands, a URL or plain text.
fn render_command(
    v: &Value,
    key: Option<&str>,
    swaps: &[(String, String)],
    values: &HashMap<String, String>,
) -> Value {
    match v {
        Value::String(text) => {
            let slot = if key.is_some_and(|k| URL_KEYS.contains(&k)) {
                Slot::Url
            } else {
                Slot::Raw
            };
            let templated = template::templatize(text, swaps, slot);
            Value::String(template::bind_known(
                &template::parse(&templated),
                slot,
                values,
            ))
        }
        Value::Array(items) => Value::Array(
            items
                .iter()
                .map(|i| render_command(i, key, swaps, values))
                .collect(),
        ),
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(k, i)| (k.clone(), render_command(i, Some(k), swaps, values)))
                .collect(),
        ),
        other => other.clone(),
    }
}

/// Actions a replay may send: plain interactions in one tab. Anything that runs page script,
/// moves to another tab, or that the recorder does not know to be a plain interaction, is left
/// out.
const REPLAY_ACTIONS: &[&str] = &[
    "navigate",
    "clickElement",
    "trustedClick",
    "hoverElement",
    "focusInput",
    "fillInput",
    "typeText",
    "insertText",
    "pressKey",
    "selectOption",
    "setChecked",
    "submitForm",
    "scrollTo",
    "waitFor",
    "waitForElement",
];

const SCRIPT_KEYS: &[&str] = &["script", "expression", "function", "fn", "js", "code"];

const URL_KEYS: &[&str] = &["url", "href", "src", "uri", "location", "origin"];

/// A URL pattern with `*` (a `waitFor` url glob): its host part is checked
/// with each `*` standing for a letter, so a pattern can only reach the
/// site's own hosts; a pattern whose host cannot be read is refused. A
/// pattern with no host is a path and stays on the page's site.
fn check_glob(policy: &HostPolicy, text: &str) -> Result<(), String> {
    let refused = || format!("'{}' is not a URL on the flow's site", text.trim());
    let text: String = text
        .trim_start_matches(|c: char| c.is_control() || c == ' ')
        .replace('\\', "/");
    let rest = match text.split_once("://") {
        Some((scheme, rest))
            if matches!(scheme.to_ascii_lowercase().as_str(), "http" | "https") =>
        {
            rest
        }
        Some(_) => return Err(refused()),
        None if text.starts_with("//") => text.trim_start_matches('/'),
        None => return Ok(()),
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let hostport = authority.rsplit('@').next().unwrap_or("");
    let host = match hostport.rsplit_once(':') {
        Some((host, port)) if !port.contains(']') => host,
        _ => hostport,
    };
    if host.is_empty() || !policy.allows_host(&host.replace('*', "a")) {
        return Err(refused());
    }
    Ok(())
}

/// Check one value the way a browser reads it: the URL parser drops tabs,
/// newlines and leading control characters, so a string that parses as an
/// http(s) URL is host-checked whatever it looks like before parsing. A value
/// under a URL key is resolved against the flow's start URL first, so
/// `//host/x`, `///host/x` and `\\host/x` are checked as the browser opens
/// them; one that does not resolve is refused.
fn check_value(
    policy: &HostPolicy,
    base: Option<&url::Url>,
    v: &Value,
    key: Option<&str>,
    scripts: &mut bool,
) -> Result<(), String> {
    match v {
        Value::String(text) => {
            let forced = key.is_some_and(|k| URL_KEYS.contains(&k));
            let parsed = match (url::Url::parse(text), forced) {
                (Ok(parsed), _) => Some(parsed),
                (Err(_), true) if text.contains('*') => {
                    check_glob(policy, text)?;
                    None
                }
                (Err(_), true) => Some(
                    base.and_then(|b| b.join(text).ok())
                        .ok_or_else(|| format!("'{}' is not a valid URL", text.trim()))?,
                ),
                (Err(_), false) => None,
            };
            if let Some(parsed) = parsed {
                match parsed.scheme() {
                    "http" | "https" => policy.check(parsed.as_str())?,
                    "about" if parsed.as_str() == "about:blank" => {}
                    scheme
                        if forced
                            || matches!(
                                scheme,
                                "ws" | "wss" | "ftp" | "file" | "javascript" | "data" | "blob"
                            ) =>
                    {
                        return Err(format!("'{}' is not an http(s) URL", text.trim()));
                    }
                    _ => {}
                }
            }
            Ok(())
        }
        Value::Array(items) => items
            .iter()
            .try_for_each(|i| check_value(policy, base, i, key, scripts)),
        Value::Object(map) => {
            for (k, item) in map {
                if SCRIPT_KEYS.contains(&k.as_str()) && item.is_string() {
                    *scripts = true;
                }
                check_value(policy, base, item, Some(k), scripts)?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

/// Check that every command a replay would send is a plain interaction and
/// every URL in it stays on the flow's site.
pub(crate) fn check_replay_commands(flow: &Flow, commands: &[Value]) -> Result<(), String> {
    let policy = HostPolicy::of(flow);
    let base = url::Url::parse(&flow.start_url).ok();
    for cmd in commands {
        let action = action_of(cmd);
        if !REPLAY_ACTIONS.contains(&action) {
            return Err(format!(
                "the recorded command '{action}' is not allowed in a replay"
            ));
        }
        let mut scripts = false;
        check_value(&policy, base.as_ref(), cmd, None, &mut scripts)?;
        if scripts {
            return Err(format!(
                "the recorded command '{action}' carries a script, which a replay does not run"
            ));
        }
    }
    Ok(())
}

/// The host and path of a URL, which a replayed request shares with the
/// recording whatever the query or body holds.
pub(crate) fn place(u: &str) -> Option<(String, String)> {
    url::Url::parse(u)
        .ok()
        .map(|u| (u.host_str().unwrap_or("").to_string(), u.path().to_string()))
}

/// Whether two places are the same. A path segment that still has a
/// reference to fill matches any segment.
fn same_place(want: &Option<(String, String)>, got: Option<(String, String)>) -> bool {
    match (want, got) {
        (Some((wh, wp)), Some((gh, gp))) => {
            let (ws, gs): (Vec<&str>, Vec<&str>) =
                (wp.split('/').collect(), gp.split('/').collect());
            *wh == gh
                && ws.len() == gs.len()
                && ws
                    .iter()
                    .zip(&gs)
                    .all(|(w, g)| w == g || (w.contains("%7B%7B") && !g.is_empty()))
        }
        (w, g) => *w == g,
    }
}

/// The entry captured during a replay that stands for a recorded request:
/// same method, host and path, the same recorded query, and for requests with
/// a body the same body or GraphQL operation (one endpoint serves every
/// query). Query parameters the replay added and the recording never had,
/// such as cache-busters, are ignored. An exact URL wins; otherwise exactly
/// one entry must match, so an ambiguous or missing match is an error. A
/// reference still unfilled in the path or query matches any value.
pub(crate) fn replay_lookup<'a>(
    entries: &'a [HarEntry],
    req: &PreparedRequest,
) -> Result<&'a HarEntry, String> {
    let operation = |b: Option<&str>| {
        b.and_then(|b| serde_json::from_str::<Value>(b).ok())
            .and_then(|v| v["operationName"].as_str().map(String::from))
    };
    let want = place(&req.url);
    let want_op = operation(req.body.as_deref());
    let same_body = |e: &HarEntry| match &req.body {
        None => true,
        // A body that still has references to fill cannot be compared yet.
        Some(b) if b.contains("{{@") => true,
        Some(b) => {
            let same_json = || {
                let (a, c) = (
                    serde_json::from_str::<Value>(b).ok()?,
                    serde_json::from_str::<Value>(e.post_data.as_deref()?).ok()?,
                );
                Some(a == c)
            };
            e.post_data.as_deref() == Some(b.as_str())
                || same_json() == Some(true)
                || (want_op.is_some() && operation(e.post_data.as_deref()) == want_op)
        }
    };
    let candidates: Vec<&HarEntry> = entries
        .iter()
        .rev()
        .filter(|e| e.method == req.method && same_place(&want, place(&e.url)) && same_body(e))
        .collect();
    if let Some(exact) = candidates.iter().find(|e| e.url == req.url) {
        return Ok(exact);
    }
    let query_of = |u: &str| -> Vec<(String, String)> {
        url::Url::parse(u)
            .map(|u| {
                u.query_pairs()
                    .map(|(k, v)| (k.into_owned(), v.into_owned()))
                    .collect()
            })
            .unwrap_or_default()
    };
    let wanted_query = query_of(&req.url);
    let has_query = |e: &HarEntry| {
        let pairs = query_of(&e.url);
        wanted_query.iter().all(|(k, v)| {
            pairs
                .iter()
                .any(|(pk, pv)| pk == k && (v.contains("{{") || pv == v))
        })
    };
    let matching: Vec<&HarEntry> = candidates.into_iter().filter(|e| has_query(e)).collect();
    match matching.as_slice() {
        [] => Err(format!(
            "no replayed request matches {} {} with its recorded query",
            req.method, req.url
        )),
        [one] => Ok(one),
        _ => Err(format!(
            "{} {} matches several replayed requests, so none is used",
            req.method, req.url
        )),
    }
}

pub(crate) fn replay_result(
    req: &PreparedRequest,
    started: Instant,
    entry: Option<&HarEntry>,
) -> TierResult {
    // A write the replay may have sent is never drift: without a readable
    // answer its outcome is unknown, and the agent must not repeat it.
    let outcome = match entry {
        None if req.write => Err(Failure::unknown_outcome(
            "the replay may have sent the write, but it was not captured".to_string(),
        )),
        None => Err(Failure::new(
            true,
            None,
            "replaying the recorded commands did not send this request".to_string(),
        )),
        Some(e) => match (e.status, e.response_body.as_deref()) {
            (Some(status), Some(body)) if !e.response_body_base64 => {
                Ok((status as u16, body.to_string()))
            }
            _ if req.write => Err(Failure::unknown_outcome(
                "the replayed write has no readable answer".to_string(),
            )),
            (Some(status), _) => Err(Failure::new(
                true,
                Some(status as u16),
                "the replayed request has no readable body".to_string(),
            )),
            _ => Err(Failure::new(
                true,
                None,
                "the replayed request got no response".to_string(),
            )),
        },
    };
    let retry_after = entry.and_then(|e| {
        e.response_headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case("retry-after"))
            .map(|(_, v)| v.clone())
    });
    with_retry_after(
        tier_result(req, Tier::Replay, started, outcome),
        retry_after,
    )
}

/// What the single replay of a run captured.
pub(crate) struct ReplayRun {
    entries: Vec<HarEntry>,
    failure: Option<String>,
    timed_out: bool,
}

/// Why a replay or request stopped for lack of time, which says nothing
/// about the site.
const TIME_UP: &str = "the run's time is up";

/// How long one replayed command, and the final wait for answers, may wait
/// for the page. The run's own deadline caps the sum.
const REPLAY_WAIT: Duration = Duration::from_secs(20);

/// How long a replay waits, after a command, for the requests that command
/// was recorded sending to show up.
const REPLAY_REQUEST_APPEAR: Duration = Duration::from_secs(2);

/// The replay tier over the flow's tab. The recording runs at most once per run;
/// every request that falls back to it reads from that capture.
pub(crate) async fn send<B: Browser>(
    flow: &Flow,
    params: &[(String, String)],
    browser: &mut B,
    run: &mut Option<ReplayRun>,
    req: &PreparedRequest,
    ctx: &SendCtx<'_>,
    run_deadline: Instant,
) -> TierResult {
    let started = Instant::now();
    let fail = |f: Failure| TierResult::fail(req, Tier::Replay, started, f);
    // A write that was already sent is only read back, never sent again.
    if req.write && ctx.ledger.already_sent(&req.id) {
        return match run.as_ref().map(|r| replay_lookup(&r.entries, req)) {
            Some(Ok(entry)) => replay_result(req, started, Some(entry)),
            Some(Err(reason)) => fail(Failure::unknown_outcome(format!(
                "the write was already sent in this run and its answer cannot be read back: {reason}"
            ))),
            None => fail(Failure::unknown_outcome(
                "the write was already sent in this run".to_string(),
            )),
        };
    }
    if run.is_none() {
        // A replay repeats every recorded command, so it also sends the
        // writes no kept request covers; each must be approved.
        for w in flow.replay_only_writes() {
            if let Err(reason) = browser.write_approved(w).await {
                return fail(Failure::unavailable(format!(
                    "replaying the recorded commands would also send {} {} ({}): {reason}",
                    w.method, w.url, w.id
                )));
            }
        }
        // A write another tier already sent would go out a second time.
        if let Some(id) = ctx.ledger.first_repeated_by_replay(flow) {
            return fail(Failure::unavailable(format!(
                "replaying the recorded commands would send write {id} again; it was already sent in this run"
            )));
        }
        match capture(flow, params, browser, ctx.wanted, REPLAY_WAIT, run_deadline).await {
            Ok(captured) => *run = Some(captured),
            // The replay cannot run, or the capture could not start: neither says the site
            // changed.
            Err(reason) => return fail(Failure::unavailable(reason)),
        }
    }
    let Some(captured) = run.as_ref() else {
        return fail(Failure::unavailable("the replay did not run".to_string()));
    };
    // The replay ran the commands that send a write, even when a later
    // command failed: the capture answers it, and no capture does not say
    // the write was not sent.
    if req.write {
        return match replay_lookup(&captured.entries, req) {
            Ok(entry) => replay_result(req, started, Some(entry)),
            Err(reason) => fail(Failure::unknown_outcome(format!(
                "the replay may have sent the write, but no answer for it was captured: {reason}{}",
                captured
                    .failure
                    .as_deref()
                    .map(|f| format!(" ({f})"))
                    .unwrap_or_default()
            ))),
        };
    }
    if let Some(reason) = captured.failure.clone() {
        // Running out of time says nothing about the site.
        return fail(if captured.timed_out {
            Failure::unavailable(reason)
        } else {
            Failure::drift(reason)
        });
    }
    match replay_lookup(&captured.entries, req) {
        Ok(entry) => replay_result(req, started, Some(entry)),
        Err(reason) => fail(Failure::drift(reason)),
    }
}

/// The places of the first-party requests a step was recorded sending.
fn expected_places(policy: &HostPolicy, recorded: &[Request]) -> Vec<(String, String)> {
    recorded
        .iter()
        .filter(|r| policy.allows_url(&r.url) && !is_telemetry(&r.url))
        .filter_map(|r| place(&r.url))
        .collect()
}

fn time_up() -> ActionError {
    ActionError {
        code: "TIMEOUT".into(),
        message: TIME_UP.into(),
    }
}

/// Run the recorded commands once and capture the requests they send. `Err` means the replay
/// could not run at all.
async fn capture<B: Browser>(
    flow: &Flow,
    params: &[(String, String)],
    browser: &mut B,
    wanted: &[&PreparedRequest],
    wait: Duration,
    run_deadline: Instant,
) -> Result<ReplayRun, String> {
    let commands = replay_commands(flow, params)?;
    check_replay_commands(flow, &commands)?;
    browser.begin_capture().await?;
    let steps = replayed_steps(flow);
    let policy = HostPolicy::of(flow);
    let began = vexdom_core::log::now_ms();
    let mut failed = None;
    let mut timed_out = false;
    for (i, mut c) in commands.into_iter().enumerate() {
        if Instant::now() >= run_deadline {
            failed = Some(format!(
                "replay stopped before command {}: {TIME_UP}",
                i + 1
            ));
            timed_out = true;
            break;
        }
        // A command waits on its own clock, within the run's.
        let deadline = (Instant::now() + wait).min(run_deadline);
        let since = vexdom_core::log::now_ms();
        // A page opened by the replay may still be rendering the element.
        let resp = loop {
            let left = run_deadline.saturating_duration_since(Instant::now());
            // A command that waits gets no more than the time left.
            if action_of(&c).starts_with("wait") {
                let own = c["timeoutMs"].as_u64().unwrap_or(10_000);
                c["timeoutMs"] = json!(own.min(left.as_millis() as u64).max(100));
            }
            let resp = match tokio::time::timeout(left, browser.run(&c)).await {
                Ok(resp) => resp,
                Err(_) => Err(time_up()),
            };
            let missing = matches!(&resp, Err(e) if e.code == "TARGET_NOT_FOUND");
            if !missing || Instant::now() >= deadline {
                break resp;
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        };
        // The next command may depend on what this one fetched (a token or a
        // cookie the page keeps), so wait for the requests the recording says
        // this command sent to answer, then for the page to go quiet.
        let recorded = steps
            .get(i)
            .and_then(|step| *step)
            .map(|step| step.requests.as_slice())
            .unwrap_or_default();
        let expected = expected_places(&policy, recorded);
        let appear = REPLAY_REQUEST_APPEAR.min(deadline.saturating_duration_since(Instant::now()));
        browser.settle(&expected, since, appear, deadline).await;
        // The replay stops as soon as the tab leaves the flow's site.
        let left = run_deadline.saturating_duration_since(Instant::now());
        let tab = match tokio::time::timeout(left, browser.tab_url()).await {
            Ok(tab) => tab,
            Err(_) => {
                failed = Some(format!("replay stopped after command {}: {TIME_UP}", i + 1));
                timed_out = true;
                break;
            }
        };
        if let Some(url) = tab.filter(|u| u.starts_with("http"))
            && let Err(reason) = policy.check(&url)
        {
            failed = Some(format!("replay stopped after command {}: {reason}", i + 1));
            break;
        }
        if let Err(error) = resp {
            timed_out = Instant::now() >= run_deadline;
            failed = Some(if timed_out {
                format!("replay stopped at command {}: {TIME_UP}", i + 1)
            } else {
                format!("replaying command {} failed: {error}", i + 1)
            });
            break;
        }
    }
    // Answers arrive after the commands return; wait within the request timeout
    // until every request that may fall back has answered.
    if failed.is_none() {
        let places: Vec<(String, String)> = wanted.iter().filter_map(|w| place(&w.url)).collect();
        let deadline = (Instant::now() + wait).min(run_deadline);
        let appear = deadline.saturating_duration_since(Instant::now());
        browser.settle(&places, began, appear, deadline).await;
    }
    let entries = browser.end_capture().await;
    if failed.is_none() && Instant::now() >= run_deadline {
        let answered = |w: &&PreparedRequest| {
            replay_lookup(&entries, w).is_ok_and(|e| {
                e.response_body.is_some() || e.status.is_some_and(|s| !(200..300).contains(&s))
            })
        };
        if !wanted.iter().all(answered) {
            failed = Some(format!(
                "replay stopped before every answer arrived: {TIME_UP}"
            ));
            timed_out = true;
        }
    }
    Ok(ReplayRun {
        entries,
        failure: failed,
        timed_out,
    })
}
