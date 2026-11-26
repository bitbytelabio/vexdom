//! `vexdom flow …` and `vexdom auth …`: saved flows and the login vault, answered by the Bridge
//! (`bridge.flow*`, `bridge.auth*`). Text output follows agent-browser's (`cli/src/output.rs`
//! `format_flow_text`, f0a9835, Apache-2.0, see NOTICE); `--json` prints the envelope.

use std::process::ExitCode;

use serde_json::{Map, Value, json};
use vexdom_client::block_on;
use vexdom_core::result::ActionResult;

use super::page::{json_output, print_json};
use super::{Args, connect, fail};

pub(super) const FLOW_USAGE: &str = "Usage: vexdom flow record start|stop|status | flow steps | flow save <name> [--from n] [--to n] [--param k=v]... [--output id] [--description t] [--timeout ms] | flow run <name> [--param k=v]... [--origin h] [--allow-write all|id,id] [--where f=v]... [--fields a,b] [--limit n] [--timeout ms] | flow list [--origin h]";
pub(super) const AUTH_USAGE: &str = "Usage: vexdom auth save <name> --url u --username u (--password p | --password-stdin) [--username-selector s] [--password-selector s] [--submit-selector s] | auth login <name> [--url u] [--no-navigate] [--credential-provider p --item i] [--timeout ms] | auth list | auth show <name> | auth delete <name>";

/// `k=v` pairs of a repeated flag as an object.
fn pairs(args: &Args, flag: &str) -> Result<Map<String, Value>, String> {
    let mut out = Map::new();
    for pair in args.values(flag) {
        let (key, value) = pair
            .split_once('=')
            .ok_or_else(|| format!("{flag} takes key=value, not {pair}"))?;
        out.insert(key.to_owned(), json!(value));
    }
    Ok(out)
}

fn number(args: &Args, flag: &str) -> Result<Option<u64>, String> {
    args.value(flag)
        .map(|value| {
            value
                .parse()
                .map_err(|_| format!("{flag} takes a whole number"))
        })
        .transpose()
}

fn flow_input(words: &[&str], args: &Args) -> Result<(&'static str, Value), String> {
    let mut input = json!({});
    let action = match words {
        ["record", op @ ("start" | "stop" | "status")] => {
            input["op"] = json!(op);
            "bridge.flowRecord"
        }
        ["steps"] => "bridge.flowSteps",
        ["list"] => {
            if let Some(origin) = args.value("--origin") {
                input["origin"] = json!(origin);
            }
            "bridge.flowList"
        }
        ["save", name] => {
            input["name"] = json!(name);
            for (flag, key) in [
                ("--from", "from"),
                ("--to", "to"),
                ("--timeout", "timeoutMs"),
            ] {
                if let Some(value) = number(args, flag)? {
                    input[key] = json!(value);
                }
            }
            for (flag, key) in [("--output", "output"), ("--description", "description")] {
                if let Some(value) = args.value(flag) {
                    input[key] = json!(value);
                }
            }
            let params = pairs(args, "--param")?;
            if !params.is_empty() {
                input["params"] = Value::Object(params);
            }
            "bridge.flowSave"
        }
        ["run", name] => {
            input["name"] = json!(name);
            let params = pairs(args, "--param")?;
            if !params.is_empty() {
                input["params"] = Value::Object(params);
            }
            let filters = pairs(args, "--where")?;
            if !filters.is_empty() {
                input["where"] = Value::Object(filters);
            }
            if let Some(origin) = args.value("--origin") {
                input["origin"] = json!(origin);
            }
            match args.value("--allow-write") {
                Some("all") => input["allowWrite"] = json!("all"),
                Some(ids) => input["allowWrite"] = json!(ids.split(',').collect::<Vec<_>>()),
                None => {}
            }
            if let Some(fields) = args.value("--fields") {
                input["fields"] = json!(fields.split(',').collect::<Vec<_>>());
            }
            for (flag, key) in [("--limit", "limit"), ("--timeout", "timeoutMs")] {
                if let Some(value) = number(args, flag)? {
                    input[key] = json!(value);
                }
            }
            "bridge.flowRun"
        }
        _ => return Err(FLOW_USAGE.into()),
    };
    Ok((action, input))
}

/// Reads the password from stdin's first line (`--password-stdin`).
fn password_from_stdin() -> Result<String, String> {
    let mut line = String::new();
    std::io::stdin()
        .read_line(&mut line)
        .map_err(|_| "Failed to read password from stdin".to_owned())?;
    let password = line.trim_end_matches('\n').trim_end_matches('\r');
    if password.is_empty() {
        return Err("Password from stdin is empty".into());
    }
    Ok(password.to_owned())
}

fn auth_input(words: &[&str], args: &Args) -> Result<(&'static str, Value), String> {
    let mut input = json!({});
    let selectors = [
        ("--username-selector", "usernameSelector"),
        ("--password-selector", "passwordSelector"),
        ("--submit-selector", "submitSelector"),
    ];
    let action = match words {
        ["save", name] => {
            input["name"] = json!(name);
            for (flag, key) in [("--url", "url"), ("--username", "username")] {
                input[key] = json!(args.value(flag).ok_or(AUTH_USAGE)?);
            }
            input["password"] = json!(match (
                args.value("--password"),
                args.flag("--password-stdin")
            ) {
                (_, true) => password_from_stdin()?,
                (Some(password), false) => {
                    eprintln!(
                        "⚠ Passwords on the command line may be visible in process listings and shell history. Use --password-stdin instead."
                    );
                    password.to_owned()
                }
                (None, false) => return Err(AUTH_USAGE.into()),
            });
            for (flag, key) in selectors {
                if let Some(value) = args.value(flag) {
                    input[key] = json!(value);
                }
            }
            "bridge.authSave"
        }
        ["login", rest @ ..] if rest.len() <= 1 => {
            if let Some(name) = rest.first() {
                input["name"] = json!(name);
            }
            for (flag, key) in [
                ("--url", "url"),
                ("--credential-provider", "credentialProvider"),
                ("--item", "item"),
            ]
            .into_iter()
            .chain(selectors)
            {
                if let Some(value) = args.value(flag) {
                    input[key] = json!(value);
                }
            }
            if args.flag("--no-navigate") {
                input["noNavigate"] = json!(true);
            }
            if let Some(ms) = number(args, "--timeout")? {
                input["timeoutMs"] = json!(ms);
            }
            "bridge.authLogin"
        }
        ["list"] => "bridge.authList",
        ["show", name] => {
            input["name"] = json!(name);
            "bridge.authShow"
        }
        ["delete", name] => {
            input["name"] = json!(name);
            "bridge.authDelete"
        }
        _ => return Err(AUTH_USAGE.into()),
    };
    Ok((action, input))
}

/// `flow …` and `auth …`.
pub(super) fn run(verb: &str, args: &Args) -> ExitCode {
    let words = args.positional();
    let built = match verb {
        "flow" => flow_input(&words, args),
        _ => auth_input(&words, args),
    };
    let (action, input) = match built {
        Ok(built) => built,
        Err(message) => return fail(&message),
    };
    block_on(async {
        let bridge = match connect().await {
            Ok(bridge) => bridge,
            Err(code) => return code,
        };
        let result = bridge.invoke(action, &input).await.result;
        if json_output(args) {
            return print_json(&result);
        }
        print_text(action, &result)
    })
}

fn print_text(action: &str, result: &ActionResult) -> ExitCode {
    match result {
        Ok(data) => {
            match flow_text(action, data) {
                Some(text) => println!("{text}"),
                None => println!("{data}"),
            }
            ExitCode::SUCCESS
        }
        Err(error) => fail(&format!("{}: {}", error.code, error.message)),
    }
}

fn str_list(value: &Value) -> String {
    value
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_default()
}

/// agent-browser's `format_flow_text`, by Bridge action.
fn flow_text(action: &str, data: &Value) -> Option<String> {
    match action {
        "bridge.flowSteps" => {
            let steps = data.get("steps")?.as_array()?;
            if steps.is_empty() {
                return Some("No steps recorded yet.".to_owned());
            }
            let mut out = String::new();
            for step in steps {
                out.push_str(&format!(
                    "{:>3} {} {}  {}  requests: {}{}\n",
                    step["seq"].as_u64().unwrap_or(0),
                    if step["ok"] == true {
                        "ok    "
                    } else {
                        "FAILED"
                    },
                    step["command"].as_str().unwrap_or(""),
                    step["url"].as_str().unwrap_or("-"),
                    step["requests"].as_u64().unwrap_or(0),
                    if step["saved"] == true {
                        "  (saved)"
                    } else {
                        ""
                    }
                ));
            }
            out.push_str(&format!(
                "Last save covered step {}; {} step(s) not saved.",
                data["lastSave"].as_u64().unwrap_or(0),
                data["unsaved"].as_u64().unwrap_or(0)
            ));
            Some(out)
        }
        "bridge.flowSave" => {
            let mut out = format!(
                "Saved flow '{}' for {} to {}\n",
                data.get("name")?.as_str()?,
                data.get("origin")?.as_str()?,
                data.get("path")?.as_str()?
            );
            let output = data.get("output").and_then(Value::as_str).unwrap_or("");
            for request in data.get("requests")?.as_array()? {
                let id = request["id"].as_str().unwrap_or("");
                out.push_str(&format!(
                    "  {} {} {} {} ({} bytes)\n",
                    if id == output { "*" } else { " " },
                    id,
                    request["method"].as_str().unwrap_or(""),
                    request["url"].as_str().unwrap_or(""),
                    request["bytes"].as_u64().unwrap_or(0)
                ));
            }
            out.push_str("  * = output. Change it with `flow save --output <id>`.\n");
            let params = str_list(&data["params"]);
            out.push_str(&format!(
                "Params: {}\n",
                if params.is_empty() { "none" } else { &params }
            ));
            if let Some(gaps) = data
                .get("gaps")
                .and_then(Value::as_array)
                .filter(|gaps| !gaps.is_empty())
            {
                out.push_str("Not captured:\n");
                for gap in gaps {
                    out.push_str(&format!(
                        "  step {} `{}`: {}\n",
                        gap["step"],
                        gap["command"].as_str().unwrap_or(""),
                        gap["reason"].as_str().unwrap_or("")
                    ));
                }
            }
            if let Some(writes) = data
                .get("writes")
                .and_then(Value::as_array)
                .filter(|writes| !writes.is_empty())
            {
                out.push_str(
                    "Writes (a run needs `--allow-write all` or the ids, or asks to confirm each):\n",
                );
                for write in writes {
                    out.push_str(&format!(
                        "  {} {} {} {}\n",
                        write["id"].as_str().unwrap_or(""),
                        write["method"].as_str().unwrap_or(""),
                        write["url"].as_str().unwrap_or(""),
                        write["kind"].as_str().unwrap_or("")
                    ));
                }
            }
            let dropped = str_list(&data["droppedHeaders"]);
            if !dropped.is_empty() {
                out.push_str(&format!(
                    "Not saved (taken from the live session at run time): {dropped}\n"
                ));
            }
            out.push_str("Unverified until the first successful `flow run`.");
            Some(out)
        }
        "bridge.flowRun" => {
            let tiers: Vec<String> = data
                .get("steps")?
                .as_array()?
                .iter()
                .map(|step| {
                    let fell_back: Vec<String> = step["fellBack"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .map(|fell| {
                            format!(
                                "{}: {}",
                                fell["tier"].as_str().unwrap_or("-"),
                                fell["error"].as_str().unwrap_or("")
                            )
                        })
                        .collect();
                    let mut line = format!(
                        "{} via {} in {}ms",
                        step["id"].as_str().unwrap_or(""),
                        step["tier"].as_str().unwrap_or("-"),
                        step["ms"]
                    );
                    if !fell_back.is_empty() {
                        line.push_str(&format!(" after {}", fell_back.join("; ")));
                    }
                    line
                })
                .collect();
            let body = data.get("data").cloned().unwrap_or(Value::Null);
            let status = match data.get("drift") {
                Some(drift) => format!(
                    "unverified: {} on the {} tier",
                    drift["reason"].as_str().unwrap_or("response changed"),
                    drift["tier"].as_str().unwrap_or("-")
                ),
                None if data.get("empty") == Some(&Value::Bool(true)) => {
                    "no results, so the flow's verified state is unchanged".to_owned()
                }
                None => "verified".to_owned(),
            };
            let saved = data
                .get("saveError")
                .and_then(Value::as_str)
                .map(|error| format!("\nThe flow file was not updated: {error}"))
                .unwrap_or_default();
            Some(format!(
                "Flow '{}' ran in {}ms ({}), {status}{saved}\n{}",
                data.get("name")?.as_str()?,
                data.get("ms")?,
                tiers.join(", "),
                serde_json::to_string_pretty(&body).unwrap_or_default()
            ))
        }
        "bridge.flowList" => {
            let flows = data.get("flows")?.as_array()?;
            if flows.is_empty() {
                return Some(
                    "No saved flows. Run `vexdom flow record start`, do the task, then `vexdom flow save <name>`."
                        .to_owned(),
                );
            }
            Some(
                flows
                    .iter()
                    .map(|flow| {
                        let params = str_list(&flow["params"]);
                        format!(
                            "{} ({}) params: {} {}",
                            flow["name"].as_str().unwrap_or(""),
                            flow["origin"].as_str().unwrap_or(""),
                            if params.is_empty() { "none" } else { &params },
                            if flow["verifiedAt"].is_null() {
                                "unverified"
                            } else {
                                "verified"
                            }
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n"),
            )
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(line: &str) -> Args {
        Args {
            all: line.split_whitespace().map(str::to_owned).collect(),
        }
    }

    #[test]
    fn flow_flags_become_bridge_inputs() {
        let parsed = args(
            "flow run items --param q=shoes --param page=2 --allow-write w1,w2 --fields a,b --limit 5",
        );
        let words = parsed.positional();
        let (action, input) = flow_input(&words, &parsed).unwrap();
        assert_eq!(action, "bridge.flowRun");
        assert_eq!(
            input,
            json!({"name": "items", "params": {"q": "shoes", "page": "2"}, "allowWrite": ["w1", "w2"],
                "fields": ["a", "b"], "limit": 5})
        );
        let parsed = args("flow record start");
        let words = parsed.positional();
        assert_eq!(
            flow_input(&words, &parsed).unwrap(),
            ("bridge.flowRecord", json!({"op": "start"}))
        );
        let parsed = args("flow run x --param nope");
        let words = parsed.positional();
        assert!(flow_input(&words, &parsed).is_err());
    }

    #[test]
    fn steps_read_like_agent_browser() {
        let data = json!({"steps": [{"seq": 1, "ok": true, "command": "click @e1", "url": "https://a.test/", "requests": 2, "saved": true}], "lastSave": 1, "unsaved": 0});
        assert_eq!(
            flow_text("bridge.flowSteps", &data).unwrap(),
            "  1 ok     click @e1  https://a.test/  requests: 2  (saved)\nLast save covered step 1; 0 step(s) not saved."
        );
    }
}
