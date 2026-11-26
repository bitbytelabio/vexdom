//! The verbs over the debugger features (ADR-030) and WebMCP: diagnostics reads, network routes and
//! HAR, the mouse, dialogs, emulation (`set`), PDF, video, traces, vitals, React, accessibility,
//! init scripts. Each becomes one page action, or a few in order.

use std::path::PathBuf;

use serde_json::{Value, json};

use super::Args;
use super::page::{Step, invoke, need};

/// Verbs this module turns into steps.
pub(super) const VERBS: &[&str] = &[
    "console",
    "errors",
    "network",
    "mouse",
    "dialog",
    "set",
    "pdf",
    "record",
    "trace",
    "profiler",
    "vitals",
    "react",
    "a11y",
    "initscript",
];

/// An absolute path for the Bridge, resolved against the working directory.
pub(super) fn absolute(path: &str) -> String {
    let path = PathBuf::from(path);
    if path.is_absolute() {
        return path.to_string_lossy().into_owned();
    }
    std::env::current_dir()
        .map(|cwd| cwd.join(&path))
        .unwrap_or(path)
        .to_string_lossy()
        .into_owned()
}

fn number<T: std::str::FromStr>(args: &Args, flag: &str, what: &str) -> Result<Option<T>, String> {
    args.value(flag)
        .map(|value| value.parse().map_err(|_| format!("{flag} is {what}")))
        .transpose()
}

fn csv(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(str::to_owned)
        .collect()
}

/// A JSON value given inline or as `@file`.
pub(super) fn json_arg(text: &str, flag: &str) -> Result<Value, String> {
    let text = match text.strip_prefix('@') {
        Some(path) => std::fs::read_to_string(path)
            .map_err(|error| format!("Could not read {path}: {error}"))?,
        None => text.to_owned(),
    };
    serde_json::from_str(&text).map_err(|error| format!("{flag} is not JSON: {error}"))
}

fn text_arg(text: &str) -> Result<String, String> {
    match text.strip_prefix('@') {
        Some(path) => {
            std::fs::read_to_string(path).map_err(|error| format!("Could not read {path}: {error}"))
        }
        None => Ok(text.to_owned()),
    }
}

pub(super) fn step(verb: &str, args: &Args) -> Result<Step, String> {
    let words = args.positional();
    match verb {
        "console" => {
            let mut input = json!({"drain": args.flag("--clear")});
            if let Some(level) = args.value("--level") {
                input["level"] = json!(level);
            }
            if let Some(contains) = args.value("--contains") {
                input["contains"] = json!(contains);
            }
            if let Some(limit) = number::<u64>(args, "--limit", "a number")? {
                input["limit"] = json!(limit);
            }
            Ok(Step::Diagnostics {
                action: "page.readConsole".into(),
                input,
            })
        }
        "errors" => Ok(Step::Diagnostics {
            action: "page.readConsole".into(),
            input: json!({"level": "error", "drain": args.flag("--clear")}),
        }),
        "network" => network_step(&words, args),
        "mouse" => mouse_step(&words, args),
        "dialog" => dialog_step(&words, args),
        "set" => set_step(&words, args),
        "pdf" => {
            let mut input = json!({
                "landscape": args.flag("--landscape"),
                "printBackground": !args.flag("--no-background"),
                "preferCSSPageSize": args.flag("--css-page-size"),
            });
            if let Some(path) = words.first() {
                input["path"] = json!(absolute(path));
            }
            Ok(invoke("page.pdf", input))
        }
        "record" => record_step(&words, args),
        "trace" | "profiler" => {
            let usage = format!("{verb} start [--categories csv] | {verb} stop [path]");
            need(&words, 1, &usage)?;
            match words[0] {
                "start" => {
                    let mut input =
                        json!({"mode": if verb == "trace" { "trace" } else { "profile" }});
                    if let Some(categories) = args.value("--categories") {
                        input["categories"] = json!(csv(categories));
                    }
                    Ok(invoke("page.traceStart", input))
                }
                "stop" => Ok(invoke(
                    "page.traceStop",
                    words
                        .get(1)
                        .map_or_else(|| json!({}), |path| json!({"path": absolute(path)})),
                )),
                _ => Err(format!("Usage: vexdom {usage}")),
            }
        }
        "vitals" => Ok(invoke(
            "page.vitals",
            json!({"reload": args.flag("--reload")}),
        )),
        "react" => react_step(&words, args),
        "a11y" => {
            let mut input = json!({});
            if let Some(tags) = args.value("--tags") {
                input["tags"] = json!(csv(tags));
            }
            if let Some(selector) = args.value("--selector") {
                input["selector"] = json!(selector);
            }
            Ok(invoke("page.a11yAudit", input))
        }
        "initscript" => {
            let usage = "initscript add <file|-> | initscript remove <id> | initscript list";
            match words[..] {
                ["add", file] => Ok(invoke(
                    "page.initScript",
                    json!({"op": "add", "source": script_source(file)?}),
                )),
                ["remove", id] => Ok(invoke(
                    "page.initScript",
                    json!({"op": "remove", "identifier": id}),
                )),
                ["list"] => Ok(invoke("page.initScript", json!({"op": "list"}))),
                _ => Err(format!("Usage: vexdom {usage}")),
            }
        }
        other => Err(format!("Unknown command \"{other}\"")),
    }
}

/// A script file, or stdin for `-`.
pub(super) fn script_source(file: &str) -> Result<String, String> {
    if file == "-" {
        use std::io::Read as _;
        let mut source = String::new();
        std::io::stdin()
            .read_to_string(&mut source)
            .map_err(|error| format!("Could not read stdin: {error}"))?;
        return Ok(source);
    }
    std::fs::read_to_string(file).map_err(|error| format!("Could not read {file}: {error}"))
}

fn network_step(words: &[&str], args: &Args) -> Result<Step, String> {
    let usage = "network requests [--filter s] [--type csv] [--method m] [--status s] [--clear] | network request <id> | network route <pattern> [--abort | --body <json|@file> …] | network unroute [pattern] | network har start [--content all|text|none] | network har stop [path]";
    match words {
        ["requests"] => {
            let mut input = json!({"drain": args.flag("--clear")});
            if let Some(filter) = args.value("--filter") {
                input["urlContains"] = json!(filter);
            }
            if let Some(types) = args.value("--type") {
                input["resourceTypes"] = json!(csv(types));
            }
            if let Some(method) = args.value("--method") {
                input["method"] = json!(method);
            }
            if let Some(status) = args.value("--status") {
                input["statusCode"] = json!(status);
            }
            Ok(Step::Diagnostics {
                action: "page.readNetwork".into(),
                input,
            })
        }
        ["request", id] => Ok(Step::Diagnostics {
            action: "page.readNetwork".into(),
            input: json!({"requestId": id, "includeBodies": true, "includeHeaders": true}),
        }),
        ["route", pattern] => {
            let mut input = json!({"pattern": pattern, "abort": args.flag("--abort")});
            if !args.flag("--abort") {
                let mut response = json!({});
                if let Some(body) = args.value("--body") {
                    response["body"] = json!(text_arg(body)?);
                }
                if let Some(status) = number::<u16>(args, "--status", "an HTTP status")? {
                    response["status"] = json!(status);
                }
                if let Some(kind) = args.value("--content-type") {
                    response["contentType"] = json!(kind);
                }
                if let Some(origin) = args.value("--allow-origin") {
                    response["allowOrigin"] = json!(origin);
                }
                input["response"] = response;
            }
            if let Some(types) = args.value("--resource-type") {
                input["resourceTypes"] = json!(csv(types));
            }
            Ok(invoke("page.routeNetwork", input))
        }
        ["unroute"] => Ok(invoke("page.unrouteNetwork", json!({}))),
        ["unroute", pattern] => Ok(invoke("page.unrouteNetwork", json!({"pattern": pattern}))),
        ["har", "start"] => Ok(invoke(
            "page.harStart",
            json!({"content": args.value("--content").unwrap_or("text")}),
        )),
        ["har", "stop"] => Ok(invoke("page.harStop", json!({}))),
        ["har", "stop", path] => Ok(invoke("page.harStop", json!({"path": absolute(path)}))),
        _ => Err(format!("Usage: vexdom {usage}")),
    }
}

fn coordinate(word: &str, what: &str) -> Result<f64, String> {
    word.parse()
        .map_err(|_| format!("{what} is a number of pixels"))
}

fn mouse_step(words: &[&str], args: &Args) -> Result<Step, String> {
    let usage = "mouse move <x> <y> [--human] [--seed n] [--steps n] [--duration ms] | mouse down|up [button] | mouse wheel <dy> [dx]";
    match words {
        ["move", x, y] => {
            let mut input = json!({
                "op": "move",
                "x": coordinate(x, "x")?,
                "y": coordinate(y, "y")?,
                "human": args.flag("--human"),
            });
            if let Some(seed) = number::<u64>(args, "--seed", "a number")? {
                input["seed"] = json!(seed);
            }
            if let Some(steps) = number::<u64>(args, "--steps", "a number")? {
                input["steps"] = json!(steps);
            }
            if let Some(ms) = number::<u64>(args, "--duration", "milliseconds")? {
                input["durationMs"] = json!(ms);
            }
            Ok(invoke("page.mouse", input))
        }
        [op @ ("down" | "up"), rest @ ..] => {
            let button = rest.first().copied().unwrap_or("left");
            if !["left", "middle", "right"].contains(&button) {
                return Err("The button is left, middle or right".into());
            }
            Ok(invoke("page.mouse", json!({"op": op, "button": button})))
        }
        ["wheel", dy, rest @ ..] => {
            let dx = rest.first().map_or(Ok(0.0), |dx| coordinate(dx, "dx"))?;
            Ok(invoke(
                "page.mouse",
                json!({"op": "wheel", "deltaY": coordinate(dy, "dy")?, "deltaX": dx}),
            ))
        }
        _ => Err(format!("Usage: vexdom {usage}")),
    }
}

fn dialog_step(words: &[&str], args: &Args) -> Result<Step, String> {
    let usage = "dialog [status] | dialog accept [text] | dialog dismiss | dialog auto on|off [--alert] [--beforeunload]";
    match words {
        [] | ["status"] => Ok(invoke("page.dialog", json!({"op": "status"}))),
        ["accept"] => Ok(invoke("page.dialog", json!({"op": "accept"}))),
        ["accept", text @ ..] => Ok(invoke(
            "page.dialog",
            json!({"op": "accept", "promptText": text.join(" ")}),
        )),
        ["dismiss"] => Ok(invoke("page.dialog", json!({"op": "dismiss"}))),
        ["auto", "off"] => Ok(invoke(
            "page.dialog",
            json!({"op": "auto", "alert": false, "beforeunload": false}),
        )),
        ["auto", "on"] => {
            let (alert, beforeunload) = (args.flag("--alert"), args.flag("--beforeunload"));
            let both = !alert && !beforeunload;
            Ok(invoke(
                "page.dialog",
                json!({"op": "auto", "alert": alert || both, "beforeunload": beforeunload || both}),
            ))
        }
        _ => Err(format!("Usage: vexdom {usage}")),
    }
}

fn set_step(words: &[&str], args: &Args) -> Result<Step, String> {
    let usage = "set viewport <w> <h> [--scale n] [--mobile] | set device <name> | set geo <lat> <lon> [--accuracy n] | set offline [on|off] | set headers <json|@file> | set credentials <user> <pass> | set media [screen|print] [--color-scheme s] [--reduced-motion s] | set reset";
    let emulate = |input: Value| Ok(invoke("page.emulate", input));
    match words {
        ["viewport", w, h] => {
            let mut viewport = json!({
                "width": w.parse::<u32>().map_err(|_| "The width is a number of pixels")?,
                "height": h.parse::<u32>().map_err(|_| "The height is a number of pixels")?,
                "mobile": args.flag("--mobile"),
            });
            if let Some(scale) = number::<f64>(args, "--scale", "a number")? {
                viewport["deviceScaleFactor"] = json!(scale);
            }
            emulate(json!({"viewport": viewport}))
        }
        ["device", name @ ..] if !name.is_empty() => emulate(json!({"device": name.join(" ")})),
        ["geo", lat, lon] => {
            let mut geolocation = json!({
                "latitude": lat.parse::<f64>().map_err(|_| "The latitude is a number")?,
                "longitude": lon.parse::<f64>().map_err(|_| "The longitude is a number")?,
            });
            if let Some(accuracy) = number::<f64>(args, "--accuracy", "a number of metres")? {
                geolocation["accuracy"] = json!(accuracy);
            }
            emulate(json!({"geolocation": geolocation}))
        }
        ["offline"] | ["offline", "on"] => emulate(json!({"offline": true})),
        ["offline", "off"] => emulate(json!({"offline": false})),
        ["headers", value] => emulate(json!({"headers": json_arg(value, "headers")?})),
        ["credentials", user, pass] => {
            emulate(json!({"credentials": {"username": user, "password": pass}}))
        }
        ["media", rest @ ..] => {
            let mut media = json!({});
            if let Some(kind) = rest.first() {
                if !["screen", "print"].contains(kind) {
                    return Err("The media type is screen or print".into());
                }
                media["type"] = json!(kind);
            }
            if let Some(scheme) = args.value("--color-scheme") {
                media["colorScheme"] = json!(scheme);
            }
            if let Some(motion) = args.value("--reduced-motion") {
                media["reducedMotion"] = json!(motion);
            }
            emulate(json!({"media": media}))
        }
        ["reset"] => emulate(json!({"reset": true})),
        _ => Err(format!("Usage: vexdom {usage}")),
    }
}

fn record_input(words: &[&str], args: &Args) -> Result<Value, String> {
    need(
        words,
        1,
        "record start <path> [--fps n] [--cursor] [--contact-sheet] [--contact-sheet-threshold x]",
    )?;
    let mut input = json!({
        "path": absolute(words[0]),
        "cursor": args.flag("--cursor"),
        "contactSheet": args.flag("--contact-sheet") || args.value("--contact-sheet-threshold").is_some(),
    });
    if let Some(fps) = number::<u32>(args, "--fps", "frames per second")? {
        input["fps"] = json!(fps);
    }
    if let Some(threshold) = number::<f64>(args, "--contact-sheet-threshold", "a fraction")? {
        input["contactSheetThreshold"] = json!(threshold);
    }
    Ok(input)
}

fn record_step(words: &[&str], args: &Args) -> Result<Step, String> {
    match words {
        ["start", rest @ ..] => Ok(invoke("page.recordStart", record_input(rest, args)?)),
        ["stop"] => Ok(invoke("page.recordStop", json!({}))),
        ["restart", rest @ ..] => Ok(Step::Many(vec![
            ("page.recordStop".into(), json!({})),
            ("page.recordStart".into(), record_input(rest, args)?),
        ])),
        _ => Err(
            "Usage: vexdom record start <path> … | record stop | record restart <path> …".into(),
        ),
    }
}

fn react_step(words: &[&str], args: &Args) -> Result<Step, String> {
    let json = args.flag("--json");
    let react = |input: Value| Ok(invoke("page.react", input));
    match words {
        ["install"] => react(json!({"op": "install"})),
        ["tree"] => react(json!({"op": "tree", "json": json})),
        ["inspect", id] => react(json!({
            "op": "inspect",
            "fiberId": id.parse::<i64>().map_err(|_| "The component id is a number")?,
        })),
        ["renders", "start"] => react(json!({"op": "rendersStart"})),
        ["renders", "stop"] => react(json!({"op": "rendersStop", "json": json})),
        ["suspense"] => react(json!({
            "op": "suspense",
            "onlyDynamic": args.flag("--only-dynamic"),
            "json": json,
        })),
        _ => Err("Usage: vexdom react install | react tree [--json] | react inspect <id> | react renders start|stop [--json] | react suspense [--only-dynamic] [--json]".into()),
    }
}

/// `webmcp …`.
pub(super) fn webmcp_step(words: &[&str], args: &Args) -> Result<Step, String> {
    let frame = || -> Result<Option<i64>, String> {
        args.value("--frame")
            .map(|id| id.parse().map_err(|_| "--frame is a frame id".to_owned()))
            .transpose()
    };
    match words {
        ["list", rest @ ..] => {
            let mut input = json!({});
            if let Some(tool) = rest.first() {
                input["tool"] = json!(tool);
            }
            if let Some(frame) = frame()? {
                input["frameId"] = json!(frame);
            }
            if args.flag("--json") {
                input["withSchemas"] = json!(true);
            }
            Ok(invoke("page.listSiteTools", input))
        }
        ["invoke", tool] => {
            let params = args
                .value("--params")
                .map_or(Ok(json!({})), |text| json_arg(text, "--params"))?;
            let mut input = json!({"tool": tool, "args": params, "detach": args.flag("--detach")});
            if let Some(frame) = frame()? {
                input["frameId"] = json!(frame);
            }
            Ok(invoke("page.callSiteTool", input))
        }
        ["result", id] => {
            let mut input = json!({"invocationId": id});
            if let Some(ms) = args.value("--wait") {
                input["waitMs"] = json!(ms.parse::<u64>().map_err(|_| "--wait is milliseconds")?);
            }
            Ok(invoke("page.siteToolResult", input))
        }
        ["cancel", id] => Ok(invoke("page.cancelSiteTool", json!({"invocationId": id}))),
        _ => Err("Usage: vexdom webmcp list [tool] [--frame id] [--json] | webmcp invoke <tool> [--frame id] [--params json|@file] [--detach] | webmcp result <id> [--wait ms] | webmcp cancel <id>".into()),
    }
}

#[cfg(test)]
mod tests {
    use std::assert_matches;

    use super::*;

    fn args(line: &str) -> Args {
        Args {
            all: line.split_whitespace().map(str::to_owned).collect(),
        }
    }

    fn call(line: &str) -> (String, Value) {
        let args = args(line);
        match step(&args.all[0], &args) {
            Ok(Step::Invoke { action, input, .. }) => (action, input),
            other => panic!("{line}: {other:?}"),
        }
    }

    #[test]
    fn network_and_emulation_verbs() {
        assert_eq!(
            call("network route *api* --body {\"x\":1}"),
            (
                "page.routeNetwork".into(),
                json!({"pattern": "*api*", "abort": false, "response": {"body": "{\"x\":1}"}})
            )
        );
        assert_eq!(call("network unroute").1, json!({}));
        assert_eq!(
            call("set viewport 400 800").1,
            json!({"viewport": {"width": 400, "height": 800, "mobile": false}})
        );
        assert_eq!(
            call("set device iPhone 15").1,
            json!({"device": "iPhone 15"})
        );
        assert_eq!(
            call("dialog auto on").1,
            json!({"op": "auto", "alert": true, "beforeunload": true})
        );
        assert_eq!(
            call("mouse wheel 120").1,
            json!({"op": "wheel", "deltaY": 120.0, "deltaX": 0.0})
        );
        assert_eq!(call("pdf /tmp/p.pdf").1["path"], json!("/tmp/p.pdf"));
        assert!(
            call("pdf out.pdf").1["path"]
                .as_str()
                .unwrap()
                .ends_with("/out.pdf")
        );
    }

    #[test]
    fn console_reads_start_a_capture_when_needed() {
        let args = args("console --level error");
        assert_matches!(
            step("console", &args),
            Ok(Step::Diagnostics { action, .. }) if action == "page.readConsole"
        );
        assert_matches!(args_step("record restart /tmp/r.webm"), Step::Many(steps) if steps.len() == 2);
    }

    fn args_step(line: &str) -> Step {
        let args = args(line);
        step(&args.all[0], &args).unwrap()
    }
}
