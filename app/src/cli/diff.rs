//! `vexdom diff` and MCP `act {"do":"diff"}` (ADR-028): what changed on a page, against the last
//! snapshot, a saved snapshot, a saved screenshot, or between two URLs. Snapshot diffs ignore refs;
//! screenshot diffs compare pixels.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use serde_json::{Value, json};
use vexdom_core::result::{ActionError, ActionResult};
use vexdom_snapshot::diff::{diff_screenshot, diff_snapshots, snapshot_comparison_text};

use super::{Args, fail};
use crate::mcp::host::exception;
use vexdom_client::Bridge;

const USAGE: &str = "Usage: vexdom diff snapshot [--baseline FILE] [-s SEL] [-c] [-d N] | diff screenshot --baseline FILE [-o OUT] [--threshold F] [-s <t>] [--full] | diff url <u1> <u2> [--screenshot] [--full]";

/// The default colour distance below which two pixels count as the same.
const PIXEL_THRESHOLD: f64 = 0.1;

/// Who asked: an agent may only compare against files in `~/vexdom/captures`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Caller {
    User,
    Agent,
}

/// `vexdom diff …`: the flags as a `diff` input, the report printed.
pub(super) async fn run_cli(bridge: &Bridge, args: &Args) -> ExitCode {
    let words = args.positional();
    let mut input = match words[..] {
        ["snapshot"] => json!({"kind": "snapshot"}),
        ["screenshot"] => json!({"kind": "screenshot"}),
        ["url", first, second] => json!({"kind": "url", "urls": [first, second]}),
        _ => return fail(USAGE),
    };
    for (flag, key) in [
        ("--baseline", "baseline"),
        ("-s", "selector"),
        ("-o", "out"),
    ] {
        if let Some(value) = args.value(flag) {
            input[key] = json!(value);
        }
    }
    if let Some(depth) = args.value("-d").and_then(|depth| depth.parse::<i64>().ok()) {
        input["depth"] = json!(depth);
    }
    if let Some(threshold) = args.value("--threshold") {
        match threshold.parse::<f64>() {
            Ok(threshold) => input["threshold"] = json!(threshold),
            Err(_) => return fail("--threshold is a fraction between 0 and 1"),
        }
    }
    if let Some(tab) = args.value("--tab").and_then(|tab| tab.parse::<i64>().ok()) {
        input["tabId"] = json!(tab);
    }
    input["compact"] = json!(args.flag("-c"));
    input["full"] = json!(args.flag("--full"));
    input["screenshot"] = json!(args.flag("--screenshot"));
    match run(bridge, &input, Caller::User).await {
        Ok(report) => {
            print_report(&report);
            ExitCode::SUCCESS
        }
        Err(error) => fail(&format!("{}: {}", error.code, error.message)),
    }
}

fn print_report(report: &Value) {
    if let Some(text) = report.get("snapshot").and_then(Value::as_str) {
        println!("{text}");
        return;
    }
    let tree = report.get("tree").unwrap_or(report);
    if let Some(diff) = tree.get("diff").and_then(Value::as_str) {
        if !diff.is_empty() {
            println!("{}", diff.trim_end());
        }
        println!(
            "{} additions, {} removals, {} unchanged",
            tree["additions"], tree["removals"], tree["unchanged"]
        );
    }
    if let Some(pixels) = report.get("pixels") {
        println!("{pixels}");
    } else if report.get("matched").is_some() {
        println!("{report}");
    }
}

fn invalid(message: impl Into<String>) -> ActionError {
    ActionError {
        code: "INVALID_INPUT".into(),
        message: message.into(),
    }
}

fn failed(message: String) -> ActionError {
    match message.split_once(": ") {
        Some((code, rest))
            if !code.is_empty() && code.chars().all(|c| c.is_ascii_uppercase() || c == '_') =>
        {
            ActionError {
                code: code.into(),
                message: rest.into(),
            }
        }
        _ => ActionError {
            code: "DIFF_FAILED".into(),
            message,
        },
    }
}

/// `diff {kind: snapshot|screenshot|url, baseline?, selector?, compact?, depth?, threshold?,
/// out?, urls?, screenshot?, full?, tabId?}`.
pub(crate) async fn run(bridge: &Bridge, input: &Value, caller: Caller) -> ActionResult {
    match input.get("kind").and_then(Value::as_str) {
        Some("snapshot") => snapshot(bridge, input, caller).await,
        Some("screenshot") => screenshot(bridge, input, caller).await,
        Some("url") => url(bridge, input, caller).await,
        _ => Err(invalid(
            "kind is snapshot (against the last or a saved one), screenshot (against a saved image) or url (two pages)",
        )),
    }
}

/// The tree of a snapshot text, without its `snapshot rN` header line.
fn tree_of(text: &str) -> &str {
    match text.split_once('\n') {
        Some((first, rest)) if first.starts_with("snapshot r") => rest,
        _ if text.starts_with("snapshot r") => "",
        _ => text,
    }
}

/// A baseline file. An agent may only read the captures folder Vexdom writes to.
fn read_baseline(path: &str, caller: Caller) -> Result<Vec<u8>, ActionError> {
    if caller == Caller::Agent {
        let captures = vexdom_core::paths::user_dir().join("captures");
        let inside = std::fs::canonicalize(path)
            .ok()
            .zip(std::fs::canonicalize(&captures).ok())
            .is_some_and(|(file, dir)| file.starts_with(dir));
        if !inside {
            return Err(invalid(format!(
                "An agent compares only against files in {}",
                captures.display()
            )));
        }
    }
    let bytes = std::fs::read(path).map_err(|_| ActionError {
        code: "FILE_NOT_FOUND".into(),
        message: path.into(),
    })?;
    // The captures folder also holds HARs and traces, which carry cookies: an agent compares
    // against a snapshot text or a PNG only.
    let snapshot_text = bytes.starts_with(b"snapshot r") || bytes.starts_with(b"- ");
    if caller == Caller::Agent && !snapshot_text && !bytes.starts_with(b"\x89PNG") {
        return Err(invalid(
            "A baseline is a saved snapshot text or a PNG screenshot",
        ));
    }
    Ok(bytes)
}

fn tree_diff(before: &str, after: &str) -> Value {
    let result = diff_snapshots(
        &snapshot_comparison_text(tree_of(before)),
        &snapshot_comparison_text(tree_of(after)),
    );
    json!({
        "diff": result.diff,
        "additions": result.additions,
        "removals": result.removals,
        "unchanged": result.unchanged,
    })
}

fn tab_of(input: &Value) -> i64 {
    input.get("tabId").and_then(Value::as_i64).unwrap_or(-1)
}

/// The snapshot text: the delta since the last one, or the whole tree with `full`.
async fn snapshot_text(bridge: &Bridge, input: &Value, full: bool) -> Result<String, ActionError> {
    let mut request = bridge.control.snapshot_request();
    {
        let mut opts = request.get().init_opts();
        opts.set_interactive(true);
        opts.set_compact(
            input
                .get("compact")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        );
        opts.set_depth(
            input
                .get("depth")
                .and_then(Value::as_i64)
                .and_then(|depth| i32::try_from(depth).ok())
                .unwrap_or(-1),
        );
        opts.set_selector(input.get("selector").and_then(Value::as_str).unwrap_or(""));
        opts.set_full(full);
        opts.set_tab_id(tab_of(input));
        opts.set_run_id(bridge.run_id().unwrap_or(""));
    }
    let reply = request
        .send()
        .promise
        .await
        .map_err(|error| exception(&error))?;
    reply
        .get()
        .and_then(|reply| reply.get_text())
        .ok()
        .and_then(|text| text.to_string().ok())
        .ok_or_else(vexdom_client::unreachable)
}

async fn snapshot(bridge: &Bridge, input: &Value, caller: Caller) -> ActionResult {
    let Some(baseline) = input.get("baseline").and_then(Value::as_str) else {
        return Ok(json!({"snapshot": snapshot_text(bridge, input, false).await?}));
    };
    let before = String::from_utf8_lossy(&read_baseline(baseline, caller)?).into_owned();
    let after = snapshot_text(bridge, input, true).await?;
    Ok(tree_diff(&before, &after))
}

/// A PNG of the page (or of `selector`'s element).
async fn png(bridge: &Bridge, input: &Value) -> Result<Vec<u8>, ActionError> {
    let mut shot = json!({"format": "png", "fullPage": input.get("full") == Some(&json!(true))});
    if let Some(selector) = input.get("selector").and_then(Value::as_str) {
        shot["target"] = super::page::target(selector);
    }
    if let Some(tab) = input.get("tabId").and_then(Value::as_i64) {
        shot["tabId"] = json!(tab);
    }
    let invoked = bridge.invoke("page.screenshot", &shot).await;
    invoked.result?;
    invoked
        .image
        .ok_or_else(|| failed("The screenshot came back without an image".into()))
}

/// The comparison as JSON; the diff image, when there is one, is written to `out`.
fn screenshot_report(
    baseline: &[u8],
    current: &[u8],
    threshold: f64,
    out: &Path,
) -> Result<Value, ActionError> {
    let result = diff_screenshot(baseline, current, threshold).map_err(failed)?;
    let mut report = json!({
        "totalPixels": result.total_pixels,
        "differentPixels": result.different_pixels,
        "mismatchPercentage": result.mismatch_percentage,
        "matched": result.matched,
    });
    if let Some(mismatch) = result.dimension_mismatch {
        report["dimensionMismatch"] = mismatch;
    }
    if let Some(image) = result.diff_image {
        std::fs::write(out, image)
            .map_err(|error| failed(format!("Could not write {}: {error}", out.display())))?;
        report["diffImage"] = json!(out.to_string_lossy());
    }
    Ok(report)
}

fn threshold(input: &Value) -> Result<f64, ActionError> {
    match input.get("threshold") {
        None | Some(Value::Null) => Ok(PIXEL_THRESHOLD),
        Some(value) => value
            .as_f64()
            .filter(|t| (0.0..=1.0).contains(t))
            .ok_or_else(|| invalid("threshold is a fraction between 0 and 1")),
    }
}

/// Where the diff image goes: `out` for the user, a temporary file for an agent.
fn diff_path(input: &Value, caller: Caller) -> PathBuf {
    match input.get("out").and_then(Value::as_str) {
        Some(out) if caller == Caller::User => PathBuf::from(out),
        _ => std::env::temp_dir().join(format!(
            "vexdom-diff-{}.png",
            vexdom_core::downloads::stamp()
        )),
    }
}

async fn screenshot(bridge: &Bridge, input: &Value, caller: Caller) -> ActionResult {
    let baseline = input
        .get("baseline")
        .and_then(Value::as_str)
        .ok_or_else(|| invalid("screenshot needs a baseline image"))?;
    let before = read_baseline(baseline, caller)?;
    let threshold = threshold(input)?;
    let current = png(bridge, input).await?;
    screenshot_report(&before, &current, threshold, &diff_path(input, caller))
}

async fn url(bridge: &Bridge, input: &Value, caller: Caller) -> ActionResult {
    let urls: Vec<&str> = input
        .get("urls")
        .and_then(Value::as_array)
        .map(|urls| urls.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    let [first, second] = urls[..] else {
        return Err(invalid("url needs urls: [first, second]"));
    };
    let with_shot = input.get("screenshot") == Some(&json!(true));
    let threshold = threshold(input)?;
    let mut sides = Vec::new();
    for url in [first, second] {
        let mut open = json!({"url": url});
        if let Some(tab) = input.get("tabId") {
            open["tabId"] = tab.clone();
        }
        bridge.invoke("page.navigate", &open).await.result?;
        let tree = snapshot_text(bridge, input, true).await?;
        let shot = if with_shot {
            Some(png(bridge, input).await?)
        } else {
            None
        };
        sides.push((tree, shot));
    }
    let mut report = json!({"tree": tree_diff(&sides[0].0, &sides[1].0)});
    if let (Some(before), Some(after)) = (&sides[0].1, &sides[1].1) {
        report["pixels"] = screenshot_report(before, after, threshold, &diff_path(input, caller))?;
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn headers_are_not_part_of_the_tree() {
        assert_eq!(
            tree_of("snapshot r3\n- button \"Go\" [ref=e1]"),
            "- button \"Go\" [ref=e1]"
        );
        assert_eq!(tree_of("- heading \"A\""), "- heading \"A\"");
    }

    #[test]
    fn an_agent_reads_baselines_only_from_the_captures_folder() {
        let error = read_baseline("/etc/hosts", Caller::Agent).unwrap_err();
        assert_eq!(error.code, "INVALID_INPUT");
    }
}
