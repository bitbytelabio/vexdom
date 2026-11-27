//! The agent-browser-style page commands (ADR-025, ADR-028): `snapshot`, element verbs on a ref, a
//! selector or a `find` locator, navigation, reading one fact (`get`, `is`), waiting, tabs, windows,
//! frames, `eval`, storage, cookies, the clipboard, WebMCP, `read`, `screenshot` and `batch`.
//! Output: the result as one line of JSON, then the snapshot delta the action caused, if any.

use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

use serde_json::{Map, Value, json};
use vexdom_core::result::{ActionError, ActionResult};
use vexdom_snapshot::{parse_ref, truncate};

use super::{Args, connect, curl_cookies, diff, fail, state, verbs};
use crate::mcp::host::{announcement, compact, exception, snapshot_webmcp, split_webmcp};
use vexdom_client::{Bridge, block_on};
use vexdom_snapshot::webmcp_text::Surface;

/// Verbs that become one or two page actions.
pub(super) const VERBS: &[&str] = &[
    "click",
    "dblclick",
    "fill",
    "type",
    "select",
    "press",
    "key",
    "keydown",
    "keyup",
    "keyboard",
    "hover",
    "focus",
    "submit",
    "check",
    "uncheck",
    "scroll",
    "scrollintoview",
    "drag",
    "upload",
    "download",
    "highlight",
    "get",
    "is",
    "find",
    "wait",
    "tab",
    "window",
    "frame",
    "eval",
    "storage",
    "cookies",
    "clipboard",
    "webmcp",
    "open",
    "goto",
    "navigate",
    "back",
    "forward",
    "reload",
    "pushstate",
];

/// Verbs that act on one element, so `find` can hand them a locator.
pub(super) const ELEMENT_VERBS: &[&str] = &[
    "click",
    "dblclick",
    "fill",
    "type",
    "select",
    "hover",
    "focus",
    "check",
    "uncheck",
    "scrollintoview",
    "upload",
    "download",
    "highlight",
    "drag",
];

/// Page commands with runners of their own rather than steps.
pub(super) const OWN_VERBS: &[&str] = &["snapshot", "read", "screenshot", "batch", "diff", "state"];

/// The longest `wait <ms>` may sleep.
const MAX_SLEEP_MS: u64 = 60_000;

pub fn is_page_command(verb: &str) -> bool {
    VERBS.contains(&verb) || verbs::VERBS.contains(&verb) || OWN_VERBS.contains(&verb)
}

/// `@e5`, `e5` and `ref=e5` name a ref; anything else is a CSS selector.
pub(crate) fn target(word: &str) -> Value {
    match parse_ref(word) {
        Some(id) if id.len() > 1 && id[1..].bytes().all(|b| b.is_ascii_digit()) => {
            json!({ "ref": id })
        }
        _ => json!({ "selector": word }),
    }
}

/// What a command prints on success: the whole result, or one field of it bare.
#[derive(Debug, Clone, PartialEq)]
pub(super) enum Print {
    Result,
    Field(&'static str),
}

/// One command as what to do.
#[derive(Debug, Clone, PartialEq)]
pub(super) enum Step {
    Invoke {
        action: String,
        input: Value,
        print: Print,
    },
    /// `wait <ms>`: the CLI sleeps.
    Sleep(u64),
    /// `click --new-tab`: read the link's href, then open it in a new tab.
    NewTab(Value),
    /// `download` and `wait --download`: capture, then copy the file to `copy_to` if given.
    Download {
        input: Value,
        copy_to: Option<PathBuf>,
    },
    /// Several calls in order, stopping at the first failure; the last answer is the command's.
    Many(Vec<(String, Value)>),
    /// `cookies set --curl`: one `page.cookies` set per cookie, answered as `{"set": N}`.
    SetCookies(Vec<Value>),
    /// A diagnostics read that starts the capture when none runs on the tab yet.
    Diagnostics { action: String, input: Value },
    /// `snapshot` inside a batch: its words, flags included.
    Snapshot(Vec<String>),
}

pub(super) fn invoke(action: &str, input: Value) -> Step {
    Step::Invoke {
        action: action.to_owned(),
        input,
        print: Print::Result,
    }
}

fn field(action: &str, input: Value, name: &'static str) -> Step {
    Step::Invoke {
        action: action.to_owned(),
        input,
        print: Print::Field(name),
    }
}

pub(super) fn need(words: &[&str], count: usize, usage: &str) -> Result<(), String> {
    if words.len() < count {
        Err(format!("Usage: vexdom {usage}"))
    } else {
        Ok(())
    }
}

/// `eval <js>`, `eval -b <base64>` or `eval --stdin`.
fn eval_code(words: &[&str], args: &Args) -> Result<String, String> {
    use base64::Engine as _;
    if let Some(encoded) = args.value("-b").or_else(|| args.value("--base64")) {
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(encoded.trim())
            .map_err(|_| "Invalid base64 encoding".to_owned())?;
        return String::from_utf8(bytes).map_err(|_| "Base64 decoded to invalid UTF-8".to_owned());
    }
    if args.flag("--stdin") {
        let mut code = String::new();
        std::io::stdin()
            .read_to_string(&mut code)
            .map_err(|error| format!("Could not read stdin: {error}"))?;
        return Ok(code);
    }
    need(words, 1, "eval <js> | eval -b <base64> | eval --stdin")?;
    Ok(words.join(" "))
}

fn modifier_of(name: &str) -> Result<&'static str, String> {
    match name.to_ascii_lowercase().as_str() {
        "control" | "ctrl" => Ok("ctrl"),
        "shift" => Ok("shift"),
        "alt" | "option" => Ok("alt"),
        "meta" | "cmd" | "command" => Ok("meta"),
        other => Err(format!(
            "Unknown modifier \"{other}\": use ctrl, shift, alt or meta"
        )),
    }
}

/// `--modifiers ctrl,shift` or `ctrl+shift`.
fn modifiers_flag(args: &Args) -> Result<Vec<&'static str>, String> {
    args.value("--modifiers").map_or(Ok(Vec::new()), |list| {
        list.split([',', '+'])
            .filter(|part| !part.is_empty())
            .map(modifier_of)
            .collect()
    })
}

/// `Control+a` as the key `a` with `ctrl` held; `Shift++` is `+` with `shift`.
fn key_combo(word: &str) -> Result<(String, Vec<&'static str>), String> {
    if word.len() < 2 || !word.contains('+') {
        return Ok((word.to_owned(), Vec::new()));
    }
    let (held, key) = match word.strip_suffix("++") {
        Some(held) => (held, "+"),
        None => word.rsplit_once('+').expect("contains a +"),
    };
    let modifiers = held
        .split('+')
        .filter(|part| !part.is_empty())
        .map(modifier_of)
        .collect::<Result<Vec<_>, _>>()?;
    Ok((key.to_owned(), modifiers))
}

fn key_step(
    target: Option<Value>,
    word: &str,
    phase: Option<&str>,
    trusted: bool,
) -> Result<Step, String> {
    let (key, modifiers) = key_combo(word)?;
    let mut input = json!({ "key": key });
    if trusted {
        input["trusted"] = json!(true);
    }
    if !modifiers.is_empty() {
        input["modifiers"] = json!(modifiers);
    }
    if let Some(target) = target {
        input["target"] = target;
    }
    if let Some(phase) = phase {
        input["phase"] = json!(phase);
    }
    Ok(invoke("page.pressKey", input))
}

/// A command and its words as what to do.
pub(super) fn action_for(verb: &str, args: &Args) -> Result<Step, String> {
    let words = args.positional();
    match verb {
        "snapshot" => Ok(Step::Snapshot(args.all.clone())),
        "read" | "screenshot" | "diff" => Err(format!(
            "\"{verb}\" prints or writes on its own; run it outside batch"
        )),
        "open" | "goto" | "navigate" => {
            need(&words, 1, "navigate <url> [--init-script <file>]")?;
            let navigate = ("page.navigate".to_owned(), json!({"url": words[0]}));
            match args.value("--init-script") {
                Some(file) => Ok(Step::Many(vec![
                    (
                        "page.initScript".into(),
                        json!({"op": "add", "source": verbs::script_source(file)?}),
                    ),
                    navigate,
                ])),
                None => Ok(invoke(&navigate.0, navigate.1)),
            }
        }
        "back" | "forward" | "reload" => Ok(invoke("page.navigate", json!({"action": verb}))),
        "pushstate" => {
            need(&words, 1, "pushstate <url>")?;
            Ok(invoke("page.pushState", json!({"url": words[0]})))
        }
        "press" | "key" => {
            need(&words, 1, "press [<ref|selector>] <key> [--trusted]")?;
            match words[..] {
                [key] => key_step(None, key, None, args.flag("--trusted")),
                [element, key, ..] => {
                    key_step(Some(target(element)), key, None, args.flag("--trusted"))
                }
                [] => unreachable!("checked above"),
            }
        }
        "keydown" | "keyup" => {
            need(&words, 1, &format!("{verb} <key>"))?;
            key_step(
                None,
                words[0],
                Some(if verb == "keydown" { "down" } else { "up" }),
                args.flag("--trusted"),
            )
        }
        "keyboard" => {
            need(&words, 2, "keyboard type|inserttext <text>")?;
            let text = words[1..].join(" ");
            match words[0] {
                "type" => Ok(invoke(
                    "page.typeText",
                    json!({"text": text, "clear": false, "trusted": args.flag("--trusted")}),
                )),
                "inserttext" => Ok(invoke("page.insertText", json!({"text": text}))),
                other => Err(format!(
                    "Unknown keyboard command \"{other}\": use type or inserttext"
                )),
            }
        }
        "submit" => Ok(match words.first() {
            Some(word) => invoke("page.submitForm", json!({"target": target(word)})),
            None => invoke("page.submitForm", json!({})),
        }),
        "scroll" => {
            let direction = words.first().copied().unwrap_or("down");
            if !["up", "down", "left", "right", "top", "bottom"].contains(&direction) {
                return Err(
                    "Usage: vexdom scroll up|down|left|right|top|bottom [px] [--selector <sel>]"
                        .into(),
                );
            }
            let mut input = json!({"direction": direction});
            if let Some(amount) = words.get(1) {
                let amount: f64 = amount
                    .parse()
                    .ok()
                    .filter(|px: &f64| *px > 0.0)
                    .ok_or("The amount is a number of pixels")?;
                input["amount"] = json!(amount);
            }
            if let Some(selector) = args.value("--selector") {
                input["container"] = target(selector);
            }
            Ok(invoke("page.scrollTo", input))
        }
        "get" => {
            need(
                &words,
                1,
                "get text|html|value|attr|count|box|styles|title|url [<ref|selector>] [name]",
            )?;
            query_step(
                words[0],
                words.get(1).map(|word| target(word)),
                words.get(2).copied(),
            )
        }
        "is" => {
            need(&words, 2, "is visible|enabled|checked <ref|selector>")?;
            is_step(words[0], target(words[1]))
        }
        "find" => find_step(&words, args),
        "wait" => wait_step(&words, args),
        "tab" => tab_step(&words, args),
        "window" => {
            need(&words, 1, "window new [url]")?;
            if words[0] != "new" {
                return Err("Usage: vexdom window new [url]".into());
            }
            Ok(invoke(
                "page.openWindow",
                words
                    .get(1)
                    .map_or_else(|| json!({}), |url| json!({"url": url})),
            ))
        }
        "frame" => {
            need(&words, 1, "frame <ref|selector>|main")?;
            // No `to`: it defaults to the top, and a string `to` would read as a selector.
            Ok(match words[0] {
                "main" | "top" => invoke("page.switchFrame", json!({})),
                word => invoke("page.switchFrame", json!({"frame": target(word)})),
            })
        }
        "eval" => Ok(invoke(
            "page.evaluate",
            json!({"code": eval_code(&words, args)?}),
        )),
        "storage" => storage_step(&words),
        "cookies" => cookies_step(&words, args),
        "clipboard" => {
            need(&words, 1, "clipboard read|write <text>|copy|paste")?;
            match words[0] {
                "read" | "copy" | "paste" => Ok(invoke("page.clipboard", json!({"op": words[0]}))),
                "write" => {
                    need(&words, 2, "clipboard write <text>")?;
                    Ok(invoke(
                        "page.clipboard",
                        json!({"op": "write", "text": words[1..].join(" ")}),
                    ))
                }
                other => Err(format!("Unknown clipboard command \"{other}\"")),
            }
        }
        "webmcp" => verbs::webmcp_step(&words, args),
        verb if verbs::VERBS.contains(&verb) => verbs::step(verb, args),
        verb if ELEMENT_VERBS.contains(&verb) => {
            need(&words, 1, &format!("{verb} <ref|selector> …"))?;
            element_step(verb, target(words[0]), &words[1..], args)
        }
        other => Err(format!("Unknown command \"{other}\"")),
    }
}

/// A verb on one element, however the element was named.
fn element_step(verb: &str, element: Value, rest: &[&str], args: &Args) -> Result<Step, String> {
    let modifiers = modifiers_flag(args)?;
    Ok(match verb {
        "click" => {
            let button = args.value("--button");
            if args.flag("--new-tab") {
                if button.is_some() || !modifiers.is_empty() {
                    return Err(
                        "--new-tab opens the link's address; it takes no --button or --modifiers"
                            .into(),
                    );
                }
                return Ok(Step::NewTab(element));
            }
            let mut input = json!({"target": element});
            if args.flag("--force") {
                input["force"] = json!(true);
            }
            if let Some(button) = button {
                if !["left", "right", "middle"].contains(&button) {
                    return Err("--button is left, right or middle".into());
                }
                input["button"] = json!(button);
            }
            if !modifiers.is_empty() {
                input["modifiers"] = json!(modifiers);
            }
            invoke("page.clickElement", input)
        }
        "dblclick" => {
            let mut input = json!({"target": element, "count": 2});
            if !modifiers.is_empty() {
                input["modifiers"] = json!(modifiers);
            }
            invoke("page.clickElement", input)
        }
        "fill" => {
            need(rest, 1, "fill <ref|selector> <value>")?;
            invoke(
                "page.fillInput",
                json!({"target": element, "value": rest.join(" ")}),
            )
        }
        "type" => {
            need(rest, 1, "type <ref|selector> <text> [--clear] [--delay ms]")?;
            let mut input = json!({
                "target": element,
                "text": rest.join(" "),
                "clear": args.flag("--clear"),
                "trusted": args.flag("--trusted"),
            });
            if let Some(delay) = args.value("--delay") {
                let delay: u64 = delay.parse().map_err(|_| "--delay is milliseconds")?;
                input["charDelayMs"] = json!(delay);
            }
            invoke("page.typeText", input)
        }
        "select" => {
            need(rest, 1, "select <ref|selector> <value…> [--by value|label]")?;
            if rest.len() > 1 {
                invoke(
                    "page.selectOption",
                    json!({"target": element, "values": rest}),
                )
            } else {
                let by = args.value("--by").unwrap_or("value");
                if !["value", "label"].contains(&by) {
                    return Err("--by is value or label".into());
                }
                let mut input = json!({"target": element});
                input[by] = json!(rest[0]);
                invoke("page.selectOption", input)
            }
        }
        "hover" => invoke(
            "page.hoverElement",
            forced(json!({"target": element}), args),
        ),
        "focus" => invoke("page.focusInput", json!({"target": element})),
        "highlight" => invoke("page.highlightElement", json!({"target": element})),
        "scrollintoview" => invoke("page.scrollTo", json!({"target": element})),
        "check" | "uncheck" => invoke(
            "page.setChecked",
            forced(json!({"target": element, "checked": verb == "check"}), args),
        ),
        "drag" => {
            need(rest, 1, "drag <from> <to>")?;
            invoke(
                "page.dragElement",
                json!({"from": element, "to": target(rest[0])}),
            )
        }
        "upload" => {
            need(rest, 1, "upload <ref|selector> <file…>")?;
            let paths = rest
                .iter()
                .map(|path| local_file(path))
                .collect::<Result<Vec<_>, _>>()?;
            invoke(
                "page.attachFile",
                json!({"target": element, "paths": paths}),
            )
        }
        "download" => Step::Download {
            input: json!({"target": element}),
            copy_to: rest.first().map(PathBuf::from),
        },
        other => return Err(format!("Unknown command \"{other}\"")),
    })
}

/// An existing file, as an absolute path resolved against the working directory.
fn local_file(path: &str) -> Result<String, String> {
    let resolved = std::env::current_dir()
        .map(|cwd| cwd.join(path))
        .unwrap_or_else(|_| PathBuf::from(path));
    if !resolved.is_file() {
        return Err(format!("FILE_NOT_FOUND: {path}"));
    }
    Ok(resolved.to_string_lossy().into_owned())
}

fn query_step(what: &str, element: Option<Value>, name: Option<&str>) -> Result<Step, String> {
    let usage = "get text|html|value|attr|count|box|styles|title|url [<ref|selector>] [name]";
    match what {
        "title" | "url" => Ok(field("page.query", json!({"what": what}), "value")),
        "html" => {
            let element = element.ok_or(format!("Usage: vexdom {usage}"))?;
            Ok(field(
                "page.extractText",
                json!({"target": element, "format": "html"}),
                "content",
            ))
        }
        "text" | "value" | "attr" | "count" | "box" | "styles" => {
            let element = element.ok_or(format!("Usage: vexdom {usage}"))?;
            let mut input = json!({"target": element, "what": what});
            match (what, name) {
                ("attr", None) => return Err("Usage: vexdom get attr <ref|selector> <name>".into()),
                (_, Some(name)) => input["name"] = json!(name),
                _ => {}
            }
            Ok(field("page.query", input, "value"))
        }
        other => Err(format!("Unknown fact \"{other}\": {usage}")),
    }
}

fn is_step(what: &str, element: Value) -> Result<Step, String> {
    if !["visible", "enabled", "checked"].contains(&what) {
        return Err("Usage: vexdom is visible|enabled|checked <ref|selector>".into());
    }
    Ok(field(
        "page.query",
        json!({"target": element, "what": what}),
        "value",
    ))
}

/// `find <how> <what> <verb> [arg]`: a locator instead of a ref or a selector.
fn find_step(words: &[&str], args: &Args) -> Result<Step, String> {
    let usage = "find role|text|label|placeholder|alt|title|testid <value> <verb> [arg] [--name n] | find first|last <sel> <verb> [arg] | find nth <n> <sel> <verb> [arg]";
    need(words, 3, usage)?;
    let (element, rest) = match words[0] {
        "first" | "last" => {
            let mut element = json!({"selector": words[1]});
            if words[0] == "last" {
                element["last"] = json!(true);
            }
            (element, &words[2..])
        }
        "nth" => {
            need(words, 4, usage)?;
            let nth: u64 = words[1].parse().map_err(|_| "find nth takes a number")?;
            (json!({"selector": words[2], "nth": nth}), &words[3..])
        }
        how @ ("role" | "text" | "label" | "placeholder" | "alt" | "title" | "testid") => {
            let key = if how == "testid" { "testId" } else { how };
            let mut element = Value::Object(Map::from_iter([(key.to_owned(), json!(words[1]))]));
            if let Some(name) = args.value("--name") {
                element["name"] = json!(name);
            }
            if args.flag("--exact") {
                element["exact"] = json!(true);
            }
            (element, &words[2..])
        }
        other => return Err(format!("Unknown locator \"{other}\": {usage}")),
    };
    let (verb, rest) = rest.split_first().ok_or(format!("Usage: vexdom {usage}"))?;
    match *verb {
        "get" => {
            need(rest, 1, "find … get <fact> [name]")?;
            query_step(rest[0], Some(element), rest.get(1).copied())
        }
        "is" => {
            need(rest, 1, "find … is visible|enabled|checked")?;
            is_step(rest[0], element)
        }
        "press" => {
            need(rest, 1, "find … press <key>")?;
            key_step(Some(element), rest[0], None, args.flag("--trusted"))
        }
        verb if ELEMENT_VERBS.contains(&verb) => element_step(verb, element, rest, args),
        other => Err(format!("\"{other}\" does not act on an element")),
    }
}

/// `--timeout`, or the project's `timeoutMs`.
/// `--force` skips the actionability checks.
fn forced(mut input: Value, args: &Args) -> Value {
    if args.flag("--force") {
        input["force"] = json!(true);
    }
    input
}

fn timeout_flag(args: &Args) -> Result<Option<u64>, String> {
    args.value("--timeout")
        .map(|ms| {
            ms.parse()
                .map_err(|_| "--timeout is milliseconds".to_owned())
        })
        .transpose()
        .map(|ms| ms.or(prefs().timeout_ms))
}

fn wait_step(words: &[&str], args: &Args) -> Result<Step, String> {
    if args.flag("--download") {
        let mut input = json!({});
        if let Some(ms) = timeout_flag(args)? {
            input["timeoutMs"] = json!(ms);
        }
        return Ok(Step::Download {
            input,
            copy_to: words.first().map(PathBuf::from),
        });
    }
    let conditions = [
        ("--text", "text"),
        ("--url", "url"),
        ("--load", "load"),
        ("--fn", "fn"),
    ];
    if conditions
        .iter()
        .any(|(flag, _)| args.value(flag).is_some())
    {
        let mut input = json!({});
        for (flag, key) in conditions {
            if let Some(value) = args.value(flag) {
                input[key] = json!(value);
            }
        }
        if let Some(ms) = timeout_flag(args)? {
            input["timeoutMs"] = json!(ms);
        }
        return Ok(invoke("page.waitFor", input));
    }
    need(
        words,
        1,
        "wait <ms> | wait <ref|selector> [--state s] [--timeout ms] | wait --text|--url|--load|--fn <v> | wait --download [path]",
    )?;
    if words[0].bytes().all(|b| b.is_ascii_digit()) {
        let ms: u64 = words[0].parse().map_err(|_| "wait takes milliseconds")?;
        if ms > MAX_SLEEP_MS {
            return Err(format!("wait sleeps at most {MAX_SLEEP_MS} ms"));
        }
        return Ok(Step::Sleep(ms));
    }
    let mut input = json!({"target": target(words[0])});
    if let Some(state) = args.value("--state") {
        input["state"] = json!(state);
    }
    if let Some(ms) = timeout_flag(args)? {
        input["timeoutMs"] = json!(ms);
    }
    Ok(invoke("page.waitForElement", input))
}

/// `12` and `t12` are tab ids; anything else matches a title or address.
fn tab_ref(words: &[&str]) -> Value {
    let word = words.join(" ");
    let digits = word.strip_prefix('t').unwrap_or(&word);
    match digits.parse::<i64>() {
        Ok(id) if !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()) => {
            json!({"tabId": id})
        }
        _ => json!({"match": word}),
    }
}

fn tab_step(words: &[&str], args: &Args) -> Result<Step, String> {
    let label = args.value("--label");
    Ok(match words {
        [] => invoke("page.switchTab", json!({})),
        ["new", url, ..] => {
            let mut input = json!({"url": url});
            if let Some(label) = label {
                input["label"] = json!(label);
            }
            invoke("page.openTab", input)
        }
        ["new"] => return Err("Usage: vexdom tab new [--label <name>] <url>".into()),
        ["close"] => invoke("page.closeTab", json!({})),
        ["close", rest @ ..] => invoke("page.closeTab", tab_ref(rest)),
        ["keep", mode, rest @ ..] => {
            if !["live", "throttled", "frozen"].contains(mode) {
                return Err("Usage: vexdom tab keep live|throttled|frozen [tab]".into());
            }
            let mut input = json!({"mode": mode});
            if let Some(id) = rest
                .first()
                .and_then(|id| id.trim_start_matches('t').parse::<i64>().ok())
            {
                input["tabId"] = json!(id);
            }
            invoke("page.tabKeep", input)
        }
        rest => invoke("page.switchTab", tab_ref(rest)),
    })
}

fn storage_step(words: &[&str]) -> Result<Step, String> {
    let usage = "storage local|session [key] | storage local|session set <key> <value> | … remove <key> | … clear";
    need(words, 1, usage)?;
    let area = words[0];
    if !["local", "session"].contains(&area) {
        return Err(format!("Usage: vexdom {usage}"));
    }
    Ok(invoke(
        "page.storage",
        match &words[1..] {
            [] => json!({"area": area, "op": "get"}),
            ["set", key, value @ ..] if !value.is_empty() => {
                json!({"area": area, "op": "set", "key": key, "value": value.join(" ")})
            }
            ["remove", key] => json!({"area": area, "op": "remove", "key": key}),
            ["clear"] => json!({"area": area, "op": "clear"}),
            [key] => json!({"area": area, "op": "get", "key": key}),
            _ => return Err(format!("Usage: vexdom {usage}")),
        },
    ))
}

fn cookies_step(words: &[&str], args: &Args) -> Result<Step, String> {
    if words == ["set"]
        && let Some(file) = args.value("--curl")
    {
        let raw = std::fs::read_to_string(file)
            .map_err(|error| format!("Could not read {file}: {error}"))?;
        let cookies = curl_cookies::parse_curl_cookies(&raw)?;
        let mut extra = json!({});
        for (flag, key) in [("--domain", "domain"), ("--url", "url")] {
            if let Some(value) = args.value(flag) {
                extra[key] = json!(value);
            }
        }
        return Ok(Step::SetCookies(
            cookies
                .into_iter()
                .map(|cookie| {
                    let mut input =
                        json!({"op": "set", "name": cookie["name"], "value": cookie["value"]});
                    for (key, value) in extra.as_object().into_iter().flatten() {
                        input[key] = value.clone();
                    }
                    input
                })
                .collect(),
        ));
    }
    let mut input = match words {
        [] | ["get"] => json!({"op": "get"}),
        ["get", name] => json!({"op": "get", "name": name}),
        ["set", name, value @ ..] if !value.is_empty() => {
            json!({"op": "set", "name": name, "value": value.join(" ")})
        }
        ["clear"] => json!({"op": "clear"}),
        ["clear", name] => json!({"op": "clear", "name": name}),
        _ => {
            return Err("Usage: vexdom cookies [get [name]] | cookies set <name> <value> [--url --domain --path --secure --http-only --same-site --expires] | cookies clear [name]".into());
        }
    };
    if let Some(url) = args.value("--url") {
        input["url"] = json!(url);
    }
    if input["op"] == "set" {
        for (flag, key) in [
            ("--domain", "domain"),
            ("--path", "path"),
            ("--same-site", "sameSite"),
        ] {
            if let Some(value) = args.value(flag) {
                input[key] = json!(value);
            }
        }
        if args.flag("--secure") {
            input["secure"] = json!(true);
        }
        if args.flag("--http-only") {
            input["httpOnly"] = json!(true);
        }
        if let Some(expires) = args.value("--expires") {
            let expires: f64 = expires.parse().map_err(|_| "--expires is epoch seconds")?;
            input["expires"] = json!(expires);
        }
    }
    Ok(invoke("page.cookies", input))
}

/// The input with `--tab`, and the tag that lets this process find its own approval requests.
fn with_tab(mut input: Value, args: &Args) -> Value {
    if let Some(tab) = args.value("--tab").and_then(|tab| tab.parse::<i64>().ok()) {
        input["tabId"] = json!(tab);
    }
    if let Some(object) = input.as_object_mut() {
        object.insert("_approver".into(), json!(super::confirm::approver_tag()));
    }
    input
}

fn max_output(args: &Args) -> Option<usize> {
    args.value("--max-output")
        .and_then(|max| max.parse().ok())
        .or_else(|| prefs().max_output)
}

/// `--json`, or the project's `json` preference.
pub(super) fn json_output(args: &Args) -> bool {
    args.json() || prefs().json == Some(true)
}

/// The user's and the project's CLI preferences (`./vexdom.json`), read once; warnings on stderr.
pub(crate) fn prefs() -> &'static vexdom_core::project_config::CliPrefs {
    static PREFS: std::sync::OnceLock<vexdom_core::project_config::CliPrefs> =
        std::sync::OnceLock::new();
    PREFS.get_or_init(|| {
        let cwd = std::env::current_dir().unwrap_or_default();
        let (prefs, warnings) = vexdom_core::project_config::load(&cwd);
        for warning in warnings {
            eprintln!("{warning}");
        }
        prefs
    })
}

fn failed(code: &str, message: impl Into<String>) -> ActionResult {
    Err(ActionError {
        code: code.to_owned(),
        message: message.into(),
    })
}

/// Copies a captured download's `savedTo` to `to`, and reports that path instead.
fn copy_download(mut data: Value, to: &Path) -> ActionResult {
    let Some(saved) = data.get("savedTo").and_then(Value::as_str) else {
        return Ok(data);
    };
    if let Err(error) = std::fs::copy(saved, to) {
        return failed(
            "ACTION_FAILED",
            format!("Could not copy {saved} to {}: {error}", to.display()),
        );
    }
    data["savedTo"] = json!(to.to_string_lossy());
    Ok(data)
}

/// Runs one step; returns what to call it in `batch`, and its result.
pub(super) async fn perform(bridge: &Bridge, step: &Step, args: &Args) -> (String, ActionResult) {
    match step {
        Step::Invoke { action, input, .. } => {
            let invoked = bridge.invoke(action, &with_tab(input.clone(), args)).await;
            (action.clone(), invoked.result)
        }
        Step::Sleep(ms) => {
            tokio::time::sleep(Duration::from_millis(*ms)).await;
            ("wait".into(), Ok(json!({"waitedMs": ms})))
        }
        Step::NewTab(element) => {
            let href = bridge
                .invoke(
                    "page.query",
                    &with_tab(
                        json!({"target": element, "what": "attr", "name": "href"}),
                        args,
                    ),
                )
                .await
                .result;
            let url = match href {
                Ok(data) => match data.get("value").and_then(Value::as_str) {
                    Some(url) if !url.is_empty() => url.to_owned(),
                    _ => {
                        return (
                            "page.openTab".into(),
                            failed(
                                "NOT_A_LINK",
                                "That element has no href to open in a new tab.",
                            ),
                        );
                    }
                },
                Err(error) => return ("page.query".into(), Err(error)),
            };
            let opened = bridge
                .invoke("page.openTab", &with_tab(json!({"url": url}), args))
                .await;
            ("page.openTab".into(), opened.result)
        }
        Step::Download { input, copy_to } => {
            let invoked = bridge
                .invoke("page.captureDownload", &with_tab(input.clone(), args))
                .await;
            let result = match (invoked.result, copy_to) {
                (Ok(data), Some(to)) => copy_download(data, to),
                (result, _) => result,
            };
            ("page.captureDownload".into(), result)
        }
        Step::Many(calls) => {
            let mut last = ("".to_owned(), Ok(Value::Null));
            for (action, input) in calls {
                let result = bridge
                    .invoke(action, &with_tab(input.clone(), args))
                    .await
                    .result;
                let failed = result.is_err();
                last = (action.clone(), result);
                if failed {
                    break;
                }
            }
            last
        }
        Step::SetCookies(inputs) => {
            for input in inputs {
                let result = bridge
                    .invoke("page.cookies", &with_tab(input.clone(), args))
                    .await
                    .result;
                if let Err(error) = result {
                    return ("page.cookies".into(), Err(error));
                }
            }
            ("page.cookies".into(), Ok(json!({"set": inputs.len()})))
        }
        Step::Snapshot(words) => {
            let given = Args { all: words.clone() };
            let full = given.flag("--full");
            let result = snapshot_reply(bridge, &given, full)
                .await
                .map(|(text, webmcp)| match webmcp {
                    Some(context) => json!({"snapshot": text, "webmcp": context}),
                    None => Value::String(text),
                })
                .map_err(|message| match message.split_once(": ") {
                    Some((code, rest)) => ActionError {
                        code: code.into(),
                        message: rest.into(),
                    },
                    None => ActionError {
                        code: "SNAPSHOT_FAILED".into(),
                        message,
                    },
                });
            ("snapshot".into(), result)
        }
        Step::Diagnostics { action, input } => {
            let result = bridge
                .invoke(action, &with_tab(input.clone(), args))
                .await
                .result;
            match result {
                Err(error) if error.code == "DIAGNOSTICS_NOT_FOUND" => {
                    let started = bridge
                        .invoke(
                            "page.startDiagnostics",
                            &with_tab(json!({"capture": ["console", "network"]}), args),
                        )
                        .await
                        .result;
                    (
                        "page.startDiagnostics".into(),
                        started.map(|_| json!(CAPTURE_STARTED)),
                    )
                }
                other => (action.clone(), other),
            }
        }
    }
}

const CAPTURE_STARTED: &str = "Capture started on this tab; nothing was recorded before now. Run the command again after the page does something.";

/// The value printed bare when it is a string, as JSON otherwise.
fn bare(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        other => compact(other),
    }
}

/// `--json`: `{"ok":true,"data":…}` or `{"ok":false,"error":{"code","message"}}` on stdout.
pub(super) fn print_json(result: &ActionResult) -> ExitCode {
    match result {
        Ok(data) => {
            println!("{}", json!({"ok": true, "data": data}));
            ExitCode::SUCCESS
        }
        Err(error) => {
            println!(
                "{}",
                json!({"ok": false, "error": {"code": error.code, "message": error.message}})
            );
            ExitCode::FAILURE
        }
    }
}

fn print_result(result: &ActionResult, print: &Print, max: Option<usize>, json: bool) -> ExitCode {
    if json {
        return print_json(result);
    }
    match result {
        Ok(data) => {
            let (data, webmcp) = split_webmcp(data);
            let data = &data;
            let shown = match (print, data.get("unserializable").and_then(Value::as_str)) {
                // `eval NaN`: the token JSON cannot hold, bare.
                (_, Some(token)) if data.get("value") == Some(&Value::Null) => token.to_owned(),
                (Print::Result, _) => compact(data),
                (Print::Field(name), _) => data.get(*name).map_or_else(|| compact(data), bare),
            };
            println!("{}", truncate(&shown, max));
            if let Some(note) = webmcp.and_then(|context| announcement(&context, Surface::Cli, max))
            {
                println!("{note}");
            }
            ExitCode::SUCCESS
        }
        Err(error) => fail(&format!("{}: {}", error.code, error.message)),
    }
}

fn print_announcement(webmcp: Option<&Value>, args: &Args) {
    if let Some(note) =
        webmcp.and_then(|context| announcement(context, Surface::Cli, max_output(args)))
    {
        println!("{note}");
    }
}

pub fn run(verb: &str, args: &Args) -> ExitCode {
    // Listing and tidying saved states needs no browser.
    if verb == "state" {
        return state::run_cli(args);
    }
    block_on(async {
        let bridge = match connect().await {
            Ok(bridge) => bridge,
            Err(code) => return code,
        };
        let _waiting = super::confirm::announce_waiting(&bridge).await;
        match verb {
            "snapshot" => snapshot(&bridge, args).await,
            "read" => read(&bridge, args).await,
            "screenshot" => screenshot(&bridge, args).await,
            "batch" => batch(&bridge, args).await,
            "diff" => diff::run_cli(&bridge, args).await,
            _ => match action_for(verb, args) {
                Ok(step) => {
                    let (_, result) = perform(&bridge, &step, args).await;
                    let print = match &step {
                        Step::Invoke { print, .. } => print.clone(),
                        _ => Print::Result,
                    };
                    print_result(&result, &print, max_output(args), json_output(args))
                }
                Err(usage) => fail(&usage),
            },
        }
    })
}

/// Snapshot text from the daemon (the delta since the last one, or the whole tree with `full`) and
/// the WebMCP context it announced with it.
async fn snapshot_reply(
    bridge: &Bridge,
    args: &Args,
    full: bool,
) -> Result<(String, Option<Value>), String> {
    let mut request = bridge.control.snapshot_request();
    {
        let mut opts = request.get().init_opts();
        opts.set_interactive(!args.flag("--all"));
        opts.set_json(args.json());
        opts.set_compact(args.flag("-c"));
        opts.set_depth(args.value("-d").and_then(|d| d.parse().ok()).unwrap_or(-1));
        opts.set_selector(args.value("-s").unwrap_or(""));
        opts.set_urls(args.flag("-u"));
        opts.set_full(full);
        opts.set_tab_id(
            args.value("--tab")
                .and_then(|tab| tab.parse().ok())
                .unwrap_or(-1),
        );
    }
    match request.send().promise.await {
        Ok(reply) => match reply.get().and_then(|reply| {
            let text = reply.get_text()?.to_string();
            Ok(text.map(|text| (text, snapshot_webmcp(&reply))))
        }) {
            Ok(Ok(reply)) => Ok(reply),
            _ => Err("DAEMON_UNREACHABLE: The Vexdom daemon did not respond".into()),
        },
        Err(error) => {
            let error = exception(&error);
            Err(format!("{}: {}", error.code, error.message))
        }
    }
}

async fn snapshot(bridge: &Bridge, args: &Args) -> ExitCode {
    match snapshot_reply(bridge, args, args.flag("--full")).await {
        Ok((text, webmcp)) if args.json() => {
            let mut data = serde_json::from_str(&text).unwrap_or(Value::String(text));
            if let (Some(context), Some(object)) = (webmcp, data.as_object_mut()) {
                object.insert("webmcp".into(), context);
            }
            print_json(&Ok(data))
        }
        Ok((text, webmcp)) => {
            println!("{}", truncate(&text, max_output(args)));
            print_announcement(webmcp.as_ref(), args);
            ExitCode::SUCCESS
        }
        Err(message) => fail(&message),
    }
}

async fn read(bridge: &Bridge, args: &Args) -> ExitCode {
    let offset = match args.value("--offset").map(str::parse::<usize>) {
        None => 0,
        Some(Ok(n)) => n,
        Some(Err(_)) => {
            return fail(&format!(
                "Invalid read offset: {}",
                args.value("--offset").unwrap_or("")
            ));
        }
    };
    if let Some(url) = args.positional().first() {
        return read_url(bridge, url, args, offset).await;
    }
    let mut request = bridge.control.read_request();
    {
        let mut opts = request.get().init_opts();
        opts.set_outline(args.flag("--outline"));
        opts.set_filter(args.value("--filter").unwrap_or(""));
        opts.set_max_chars(max_output(args).map_or(0, |max| max.min(u32::MAX as usize) as u32));
        opts.set_offset(offset.min(u32::MAX as usize) as u32);
        if let Some(word) = args.value("--target") {
            opts.set_target(target(word).to_string());
        }
    }
    match request.send().promise.await {
        Ok(reply) => match reply
            .get()
            .and_then(|reply| reply.get_text())
            .map(|text| text.to_string())
        {
            Ok(Ok(text)) => {
                println!("{text}");
                ExitCode::SUCCESS
            }
            _ => fail("DAEMON_UNREACHABLE: The Vexdom daemon did not respond"),
        },
        Err(error) => {
            let error = exception(&error);
            fail(&format!("{}: {}", error.code, error.message))
        }
    }
}

/// `read <url>`: over HTTP, never touching a tab.
async fn read_url(bridge: &Bridge, url: &str, args: &Args, offset: usize) -> ExitCode {
    let mut input = json!({
        "raw": args.flag("--raw"),
        "requireMd": args.flag("--require-md"),
        "outline": args.flag("--outline"),
    });
    if let Some(filter) = args.value("--filter") {
        input["filter"] = json!(filter);
    }
    if let Some(mode) = args.value("--llms") {
        input["llms"] = json!(mode);
    }
    if let Some(ms) = args.value("--timeout") {
        match crate::read_url::parse_timeout_ms(ms) {
            Ok(ms) => input["timeout"] = json!(ms),
            Err(message) => return fail(&message),
        }
    }
    let read = match crate::read_url::options_from_command(&input) {
        // The user at their own CLI may read their own network.
        Ok(options) => {
            let options = crate::read_url::ReadOptions {
                allow_private: true,
                ..options
            };
            crate::read_url::run_read(bridge, url, options).await
        }
        Err(message) => Err(message),
    };
    let result: ActionResult = read.map_err(|message| match message.split_once(": ") {
        Some((code, rest)) if code.chars().all(|c| c.is_ascii_uppercase() || c == '_') => {
            ActionError {
                code: code.into(),
                message: rest.into(),
            }
        }
        _ => ActionError {
            code: "READ_FAILED".into(),
            message,
        },
    });
    if json_output(args) {
        return print_json(&result.map(|mut read| {
            read["content"] = json!(vexdom_snapshot::page(
                &crate::read_url::sealed_content(&read),
                offset,
                None
            ));
            read
        }));
    }
    match result {
        Ok(read) => {
            println!(
                "{}",
                vexdom_snapshot::page(
                    &crate::read_url::sealed_content(&read),
                    offset,
                    max_output(args)
                )
            );
            ExitCode::SUCCESS
        }
        Err(error) => fail(&format!("{}: {}", error.code, error.message)),
    }
}

/// A capture as raw bytes and the result describing it; a string result (unchanged) is `Ok(None)`.
pub(super) async fn capture(
    bridge: &Bridge,
    input: Value,
    args: &Args,
) -> Result<Option<(Value, Vec<u8>)>, String> {
    let invoked = bridge
        .invoke("page.screenshot", &with_tab(input, args))
        .await;
    let data = invoked
        .result
        .map_err(|error| format!("{}: {}", error.code, error.message))?;
    if data.is_string() {
        println!("{}", bare(&data));
        return Ok(None);
    }
    match invoked.image {
        Some(image) => Ok(Some((data, image))),
        None => Err(format!("The screenshot came back without an image: {data}")),
    }
}

/// `screenshot`'s input from its flags.
pub(super) fn screenshot_input(args: &Args) -> Result<Value, String> {
    let mut input = json!({});
    if args.flag("--if-changed") {
        input["ifChanged"] = json!(true);
    }
    if let Some(threshold) = args.value("--threshold") {
        let threshold: f64 = threshold
            .parse()
            .ok()
            .filter(|t: &f64| (0.0..=1.0).contains(t))
            .ok_or("--threshold is a fraction between 0 and 1")?;
        input["ifChanged"] = json!(true);
        input["threshold"] = json!(threshold);
    }
    if args.flag("--annotate") || prefs().annotate == Some(true) {
        input["annotate"] = json!(true);
    }
    if args.flag("--full") {
        input["fullPage"] = json!(true);
    }
    if let Some(element) = args.value("-s") {
        input["target"] = target(element);
    }
    // `-o shot.png` asks for a PNG; the extension's own default is JPEG.
    let named = args.value("-o").map(str::to_ascii_lowercase);
    let format = args.value("--screenshot-format").or(prefs()
        .screenshot_format
        .as_deref()
        .filter(|_| named.is_none()));
    match (format, named.as_deref()) {
        (Some(format @ ("png" | "jpeg")), _) => input["format"] = json!(format),
        (Some(_), _) => return Err("--screenshot-format is png or jpeg".into()),
        (None, Some(path)) if path.ends_with(".png") => input["format"] = json!("png"),
        (None, Some(path)) if path.ends_with(".jpg") || path.ends_with(".jpeg") => {
            input["format"] = json!("jpeg");
        }
        _ => {}
    }
    let quality = args
        .value("--screenshot-quality")
        .map(str::to_owned)
        .or_else(|| {
            prefs()
                .screenshot_quality
                .map(|quality| quality.to_string())
        });
    if let Some(quality) = quality {
        let quality: u64 = quality
            .parse()
            .map_err(|_| "--screenshot-quality is 0-100")?;
        input["quality"] = json!(quality);
    }
    Ok(input)
}

async fn screenshot(bridge: &Bridge, args: &Args) -> ExitCode {
    let input = match screenshot_input(args) {
        Ok(input) => input,
        Err(usage) => return fail(&usage),
    };
    let (data, image) = match capture(bridge, input, args).await {
        Ok(Some(captured)) => captured,
        Ok(None) => return ExitCode::SUCCESS,
        Err(message) => return fail(&message),
    };
    let extension = match data.get("format").and_then(Value::as_str) {
        Some("jpeg" | "jpg") => "jpg",
        Some("webp") => "webp",
        _ => "png",
    };
    let name = format!(
        "vexdom-screenshot-{}.{extension}",
        vexdom_core::downloads::stamp()
    );
    let path = match (args.value("-o"), args.value("--screenshot-dir")) {
        (Some(path), _) => PathBuf::from(path),
        (None, Some(dir)) => {
            if let Err(error) = std::fs::create_dir_all(dir) {
                return fail(&format!("Could not create {dir}: {error}"));
            }
            Path::new(dir).join(name)
        }
        (None, None) => std::env::temp_dir().join(name),
    };
    if let Err(error) = std::fs::write(&path, &image) {
        return fail(&format!("Could not write {}: {error}", path.display()));
    }
    let mut out = data.clone();
    out["savedTo"] = json!(path.to_string_lossy());
    println!("{out}");
    ExitCode::SUCCESS
}

/// One batch line: `{"action","input"}` JSON, or the CLI form (`click @e3`, flags included).
fn step_of(line: &str) -> Result<Step, String> {
    let line = line.trim();
    if line.starts_with('{') {
        let step: Value =
            serde_json::from_str(line).map_err(|error| format!("not JSON: {error}"))?;
        let named = step
            .get("action")
            .and_then(Value::as_str)
            .ok_or("a step needs an \"action\"")?;
        let action = vexdom_core::tools::resolve_action(named).unwrap_or_else(|| named.to_owned());
        return Ok(invoke(
            &action,
            step.get("input")
                .cloned()
                .filter(Value::is_object)
                .unwrap_or_else(|| json!({})),
        ));
    }
    let args = Args {
        all: shell_words(line),
    };
    let verb = args.all.first().ok_or("an empty step")?.clone();
    action_for(&verb, &args)
}

/// Splits on spaces, keeping "double" or 'single' quoted words whole.
fn shell_words(line: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    let mut started = false;
    for c in line.chars() {
        match quote {
            Some(open) if c == open => quote = None,
            Some(_) => current.push(c),
            None if c == '"' || c == '\'' => {
                quote = Some(c);
                started = true;
            }
            None if c.is_whitespace() => {
                if started || !current.is_empty() {
                    words.push(std::mem::take(&mut current));
                    started = false;
                }
            }
            None => current.push(c),
        }
    }
    if started || !current.is_empty() {
        words.push(current);
    }
    words
}

/// One batch entry: the words it was given as, and the step they make.
struct Entry {
    command: Vec<String>,
    step: Result<Step, String>,
}

/// `batch`: commands as arguments, a JSON array of string arrays on stdin, or one per stdin line
/// (JSON `{"action","input"}` or the CLI form).
fn batch_entries(args: &Args) -> Result<Vec<Entry>, String> {
    let given = args.positional();
    let of_words = |command: Vec<String>| {
        let parsed = Args {
            all: command.clone(),
        };
        let step = parsed
            .all
            .first()
            .ok_or_else(|| "an empty step".to_owned())
            .and_then(|verb| action_for(verb, &parsed));
        Entry { command, step }
    };
    if !given.is_empty() {
        return Ok(given
            .iter()
            .map(|line| of_words(shell_words(line)))
            .collect());
    }
    let mut input = String::new();
    std::io::stdin()
        .read_to_string(&mut input)
        .map_err(|error| format!("Failed to read stdin: {error}"))?;
    if input.trim_start().starts_with('[') {
        let commands: Vec<Vec<String>> = serde_json::from_str(&input).map_err(|error| {
            format!("Invalid JSON input: {error}. Expected an array of string arrays.")
        })?;
        return Ok(commands.into_iter().map(of_words).collect());
    }
    Ok(input
        .lines()
        .filter(|line| !line.trim().is_empty() && !line.trim_start().starts_with('#'))
        .map(|line| Entry {
            command: shell_words(line),
            step: step_of(line),
        })
        .collect())
}

async fn batch(bridge: &Bridge, args: &Args) -> ExitCode {
    let bail = args.flag("--bail");
    let json = json_output(args);
    let omit = args.flag("--omit-command");
    let max = max_output(args);
    let entries = match batch_entries(args) {
        Ok(entries) => entries,
        Err(message) => return fail(&message),
    };
    let mut failed = false;
    let mut results = Vec::new();
    for (at, entry) in entries.into_iter().enumerate() {
        let number = at + 1;
        let (action, result) = match &entry.step {
            Ok(step) => perform(bridge, step, args).await,
            Err(message) => (
                String::new(),
                Err(ActionError {
                    code: "INVALID_INPUT".into(),
                    message: message.clone(),
                }),
            ),
        };
        failed |= result.is_err();
        if json {
            let mut item = if omit {
                json!({"index": at})
            } else {
                json!({"command": entry.command})
            };
            item["success"] = json!(result.is_ok());
            match &result {
                Ok(data) => item["result"] = data.clone(),
                Err(error) => item["error"] = json!(format!("{}: {}", error.code, error.message)),
            }
            results.push(item);
        } else {
            match &result {
                Ok(data) => {
                    let (data, webmcp) = split_webmcp(data);
                    println!("{number}. {action} ok: {}", truncate(&compact(&data), max));
                    if let Some(note) =
                        webmcp.and_then(|context| announcement(&context, Surface::Cli, max))
                    {
                        println!("{note}");
                    }
                }
                Err(error) if action.is_empty() => {
                    println!("{number}. failed: {}: {}", error.code, error.message);
                }
                Err(error) => println!(
                    "{number}. {action} failed: {}: {}",
                    error.code, error.message
                ),
            }
            let _ = std::io::stdout().flush();
        }
        if failed && bail {
            if !json {
                println!("Stopped after step {number}.");
            }
            break;
        }
    }
    if json {
        println!("{}", Value::Array(results));
    }
    if failed {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The action and input of a step that is one call.
    fn call(line: &str) -> (String, Value) {
        match step_of(line).unwrap_or_else(|error| panic!("{line}: {error}")) {
            Step::Invoke { action, input, .. } => (action, input),
            other => panic!("{line} is not one call: {other:?}"),
        }
    }

    #[test]
    fn targets_and_steps() {
        assert_eq!(target("@e5"), json!({"ref": "e5"}));
        assert_eq!(target("ref=e12"), json!({"ref": "e12"}));
        assert_eq!(target("#email"), json!({"selector": "#email"}));
        assert_eq!(
            target("button.primary"),
            json!({"selector": "button.primary"})
        );
        assert_eq!(
            call("fill @e7 \"Ada Lovelace\""),
            (
                "page.fillInput".to_owned(),
                json!({"target": {"ref": "e7"}, "value": "Ada Lovelace"})
            )
        );
        assert_eq!(
            call(r#"{"action":"page_clickElement","input":{"target":"e3"}}"#),
            ("page.clickElement".to_owned(), json!({"target": "e3"}))
        );
        assert_eq!(call("press Enter").1, json!({"key": "Enter"}));
        assert!(step_of("click").is_err());

        assert_eq!(
            call("dblclick @e2").1,
            json!({"target": {"ref": "e2"}, "count": 2})
        );
        assert_eq!(
            call("press Control+a").1,
            json!({"key": "a", "modifiers": ["ctrl"]})
        );
        assert_eq!(
            call("find label Email fill a@b.c"),
            (
                "page.fillInput".to_owned(),
                json!({"target": {"label": "Email"}, "value": "a@b.c"})
            )
        );
        assert_eq!(call("type @e1 hi").1["clear"], json!(false));
        assert_eq!(
            call("scroll down 300 --selector .list"),
            (
                "page.scrollTo".to_owned(),
                json!({"direction": "down", "amount": 300.0, "container": {"selector": ".list"}})
            )
        );
    }

    #[test]
    fn more_verbs() {
        assert_eq!(
            call("click @e1 --button right --modifiers ctrl,shift").1,
            json!({"target": {"ref": "e1"}, "button": "right", "modifiers": ["ctrl", "shift"]})
        );
        assert_eq!(
            step_of("click a.x --new-tab").unwrap(),
            Step::NewTab(json!({"selector": "a.x"}))
        );
        assert_eq!(
            call("keydown Shift").1,
            json!({"key": "Shift", "phase": "down"})
        );
        assert_eq!(
            call("select @e4 a b").1,
            json!({"target": {"ref": "e4"}, "values": ["a", "b"]})
        );
        assert_eq!(
            call("select @e4 Blue --by label").1,
            json!({"target": {"ref": "e4"}, "label": "Blue"})
        );
        assert_eq!(
            call("uncheck @e3").1,
            json!({"target": {"ref": "e3"}, "checked": false})
        );
        assert_eq!(
            call("get attr @e2 href").1,
            json!({"target": {"ref": "e2"}, "what": "attr", "name": "href"})
        );
        assert_eq!(call("get title").1, json!({"what": "title"}));
        assert_eq!(
            call("find role button click --name Submit").1,
            json!({"target": {"role": "button", "name": "Submit"}})
        );
        assert_eq!(
            call("find nth 2 li.item get text").1,
            json!({"target": {"selector": "li.item", "nth": 2}, "what": "text"})
        );
        assert_eq!(step_of("wait 250").unwrap(), Step::Sleep(250));
        assert!(step_of("wait 60001").is_err());
        assert_eq!(
            call("wait --text \"Firefox\" --timeout 2000"),
            (
                "page.waitFor".to_owned(),
                json!({"text": "Firefox", "timeoutMs": 2000})
            )
        );
        assert_eq!(
            call("tab t12"),
            ("page.switchTab".to_owned(), json!({"tabId": 12}))
        );
        assert_eq!(call("tab close Inbox").1, json!({"match": "Inbox"}));
        assert_eq!(call("frame main").1, json!({}));
        assert_eq!(
            call("storage local set k v"),
            (
                "page.storage".to_owned(),
                json!({"area": "local", "op": "set", "key": "k", "value": "v"})
            )
        );
        assert_eq!(
            call("cookies set c1 v1 --secure"),
            (
                "page.cookies".to_owned(),
                json!({"op": "set", "name": "c1", "value": "v1", "secure": true})
            )
        );
        assert_eq!(call("eval -- 1 + 1").1, json!({"code": "1 + 1"}));
        assert_eq!(
            call("keyboard inserttext hi there").1,
            json!({"text": "hi there"})
        );
        assert_eq!(
            step_of("upload #f /definitely/not/here.txt").unwrap_err(),
            "FILE_NOT_FOUND: /definitely/not/here.txt"
        );
        assert!(step_of("frobnicate").is_err());
    }
}
