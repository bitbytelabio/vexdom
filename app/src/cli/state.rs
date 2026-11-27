//! `vexdom state` and MCP `act {"do":"state"}` (ADR-028): a tab's cookies and web storage saved to
//! a JSON file and replayed later, one file per name in `~/.vexdom/states/`. Values read as credentials come back sealed, so
//! a saved handle is released again only in the browser session that sealed it.

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, SystemTime};

use serde_json::{Value, json};
use vexdom_core::result::{ActionError, ActionResult};
use vexdom_core::{log, paths};

use super::diff::Caller;
use super::{Args, connect, fail};
use vexdom_client::{Bridge, block_on};

const USAGE: &str = "Usage: vexdom state save|load|show <name|path> | state list | state rename <old> <new> | state clear [name] | state clear --all | state clean --older-than <days>";

const AREAS: [(&str, &str); 2] = [("local", "localStorage"), ("session", "sessionStorage")];

/// A word with a `/` or a `.json` ending is a path; anything else names a file in the states
/// folder. An agent names files in that folder only.
fn path_of(name: &str, caller: Caller) -> Result<PathBuf, String> {
    if name.is_empty() {
        return Err(USAGE.into());
    }
    let is_path = name.contains('/') || name.contains('\\') || name.ends_with(".json");
    match (is_path, caller) {
        (true, Caller::User) => Ok(PathBuf::from(name)),
        (true, Caller::Agent) => Err(format!(
            "INVALID_INPUT: name a saved state (letters, digits, - and _), not a path: {name}"
        )),
        (false, _) if name.starts_with('.') => {
            Err(format!("INVALID_INPUT: {name} is not a state name"))
        }
        (false, _) => Ok(paths::states_dir().join(format!("{name}.json"))),
    }
}

/// `vexdom state …`: the words as a `state` input, the answer printed.
pub(super) fn run_cli(args: &Args) -> ExitCode {
    let words = args.positional();
    let mut input = match words[..] {
        [op @ ("save" | "load" | "show"), name] => json!({"op": op, "name": name}),
        [] | ["list"] => json!({"op": "list"}),
        ["rename", old, new] => json!({"op": "rename", "name": old, "newName": new}),
        ["clear"] if args.flag("--all") => json!({"op": "clear", "all": true}),
        ["clear", name] => json!({"op": "clear", "name": name}),
        ["clean"] => match args
            .value("--older-than")
            .and_then(|days| days.parse::<f64>().ok())
        {
            Some(days) => json!({"op": "clean", "olderThanDays": days}),
            None => return fail("Usage: vexdom state clean --older-than <days>"),
        },
        _ => return fail(USAGE),
    };
    if let Some(tab) = args.value("--tab").and_then(|tab| tab.parse::<i64>().ok()) {
        input["tabId"] = json!(tab);
    }
    let outcome = if matches!(words.first(), Some(&("save" | "load"))) {
        block_on(async {
            let Ok(bridge) = connect().await else {
                // `connect` has already said why it failed.
                return None;
            };
            Some(run(Some(&bridge), &input, Caller::User).await)
        })
    } else {
        Some(block_on(run(None, &input, Caller::User)))
    };
    match outcome {
        None => ExitCode::FAILURE,
        Some(Ok(Value::Array(states))) => {
            if states.is_empty() {
                println!("No saved states. Save one with: vexdom state save <name>");
            }
            for state in states {
                println!(
                    "{}\t{} bytes\t{}",
                    state["name"].as_str().unwrap_or(""),
                    state["bytes"],
                    state["modified"].as_str().unwrap_or("")
                );
            }
            ExitCode::SUCCESS
        }
        Some(Ok(answer)) => {
            println!("{answer}");
            ExitCode::SUCCESS
        }
        Some(Err(error)) => fail(&format!("{}: {}", error.code, error.message)),
    }
}

fn as_error(message: String) -> ActionError {
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
            code: "STATE_FAILED".into(),
            message,
        },
    }
}

/// `state {op: save|load|list|show|rename|clear|clean, name?, newName?, all?, olderThanDays?,
/// tabId?}`. `save` and `load` need the bridge.
pub(crate) async fn run(bridge: Option<&Bridge>, input: &Value, caller: Caller) -> ActionResult {
    let text = |key: &str| input.get(key).and_then(Value::as_str).unwrap_or("");
    let tab = input.get("tabId").and_then(Value::as_i64);
    let needs_bridge = || {
        bridge.ok_or_else(|| ActionError {
            code: "EXTENSION_OFFLINE".into(),
            message: "Saving or loading a state needs the browser".into(),
        })
    };
    let answer = match text("op") {
        "save" => save(needs_bridge()?, text("name"), tab, caller).await,
        "load" => load(needs_bridge()?, text("name"), tab, caller).await,
        "list" => Ok(list()),
        "show" => show(text("name"), caller),
        "rename" => rename(text("name"), text("newName"), caller),
        "clear" if input.get("all") == Some(&json!(true)) => clear_all(),
        "clear" => path_of(text("name"), caller).and_then(|path| remove(&path)),
        "clean" => clean(input.get("olderThanDays").and_then(Value::as_f64)),
        _ => Err(format!(
            "INVALID_INPUT: op is save, load, list, show, rename, clear or clean. {USAGE}"
        )),
    };
    answer.map_err(as_error)
}

async fn call(
    bridge: &Bridge,
    action: &str,
    mut input: Value,
    tab: Option<i64>,
) -> Result<Value, String> {
    if let Some(tab) = tab {
        input["tabId"] = json!(tab);
    }
    bridge
        .invoke(action, &input)
        .await
        .result
        .map_err(|error| format!("{}: {}", error.code, error.message))
}

/// `scheme://host[:port]` of a URL.
fn origin_of(url: &str) -> String {
    let Some((scheme, rest)) = url.split_once("://") else {
        return String::new();
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host = authority
        .rsplit_once('@')
        .map_or(authority, |(_, host)| host);
    format!("{scheme}://{host}")
}

fn read_state(path: &Path) -> Result<Value, String> {
    let text =
        std::fs::read_to_string(path).map_err(|_| format!("FILE_NOT_FOUND: {}", path.display()))?;
    serde_json::from_str(&text)
        .map_err(|error| format!("{} is not a saved state: {error}", path.display()))
}

fn count(value: &Value) -> usize {
    match value {
        Value::Array(items) => items.len(),
        Value::Object(map) => map.len(),
        _ => 0,
    }
}

async fn save(
    bridge: &Bridge,
    name: &str,
    tab: Option<i64>,
    caller: Caller,
) -> Result<Value, String> {
    let path = path_of(name, caller)?;
    let url = call(bridge, "page.query", json!({"what": "url"}), tab).await?;
    let url = url
        .get("value")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned();
    let cookies = call(bridge, "page.cookies", json!({"op": "get"}), tab).await?;
    let mut state = json!({
        "version": 1,
        "url": url,
        "origin": origin_of(&url),
        "savedAt": log::now_ms(),
        "cookies": cookies.get("cookies").cloned().unwrap_or_else(|| json!([])),
    });
    for (area, key) in AREAS {
        let read = call(
            bridge,
            "page.storage",
            json!({"area": area, "op": "get"}),
            tab,
        )
        .await?;
        state[key] = read.get("entries").cloned().unwrap_or_else(|| json!({}));
    }
    if path.parent() == Some(paths::states_dir().as_path()) {
        paths::ensure_private_dir(&paths::states_dir()).map_err(|error| error.to_string())?;
    }
    let text = serde_json::to_string_pretty(&state).map_err(|error| error.to_string())?;
    paths::write_private(&path, text.as_bytes())
        .map_err(|error| format!("Could not write {}: {error}", path.display()))?;
    Ok(
        json!({"savedTo": path.to_string_lossy(), "cookies": count(&state["cookies"]),
        "localStorage": count(&state["localStorage"]), "sessionStorage": count(&state["sessionStorage"])}),
    )
}

const SAME_SITE: [&str; 3] = ["strict", "lax", "no_restriction"];

async fn load(
    bridge: &Bridge,
    name: &str,
    tab: Option<i64>,
    caller: Caller,
) -> Result<Value, String> {
    let state = read_state(&path_of(name, caller)?)?;
    let url = state
        .get("url")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned();
    let mut cookies = 0;
    for cookie in state
        .get("cookies")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let mut input = json!({"op": "set", "url": url});
        for key in ["name", "value", "path", "secure", "httpOnly", "expires"] {
            if let Some(value) = cookie.get(key).filter(|value| !value.is_null()) {
                input[key] = value.clone();
            }
        }
        // A host-only cookie has no leading dot; giving its domain would widen it to subdomains.
        if let Some(domain) = cookie
            .get("domain")
            .and_then(Value::as_str)
            .filter(|d| d.starts_with('.'))
        {
            input["domain"] = json!(domain);
        }
        if let Some(same) = cookie
            .get("sameSite")
            .and_then(Value::as_str)
            .filter(|s| SAME_SITE.contains(s))
        {
            input["sameSite"] = json!(same);
        }
        call(bridge, "page.cookies", input, tab).await?;
        cookies += 1;
    }
    // Web storage belongs to an origin: the tab goes there first if it is somewhere else.
    let here = call(bridge, "page.query", json!({"what": "url"}), tab).await?;
    let here = here.get("value").and_then(Value::as_str).unwrap_or("");
    if !url.is_empty() && origin_of(here) != origin_of(&url) {
        call(bridge, "page.navigate", json!({"url": url}), tab).await?;
    }
    let mut counts = json!({"cookies": cookies});
    for (area, key) in AREAS {
        let mut set = 0;
        for (entry, value) in state
            .get(key)
            .and_then(Value::as_object)
            .into_iter()
            .flatten()
        {
            let Some(value) = value.as_str() else {
                continue;
            };
            call(
                bridge,
                "page.storage",
                json!({"area": area, "op": "set", "key": entry, "value": value}),
                tab,
            )
            .await?;
            set += 1;
        }
        counts[key] = json!(set);
    }
    Ok(counts)
}

pub(super) fn saved_files() -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(paths::states_dir()) else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
        .collect();
    files.sort();
    files
}

fn modified(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path)
        .and_then(|meta| meta.modified())
        .ok()
}

fn list() -> Value {
    saved_files()
        .into_iter()
        .map(|path| {
            let size = std::fs::metadata(&path).map_or(0, |meta| meta.len());
            let when = modified(&path)
                .and_then(|at| jiff::Timestamp::try_from(at).ok())
                .map_or_else(String::new, |at| at.to_string());
            let name = path
                .file_stem()
                .map_or_else(String::new, |stem| stem.to_string_lossy().into_owned());
            json!({"name": name, "bytes": size, "modified": when})
        })
        .collect()
}

fn show(name: &str, caller: Caller) -> Result<Value, String> {
    let state = read_state(&path_of(name, caller)?)?;
    Ok(json!({
        "origin": state.get("origin"),
        "url": state.get("url"),
        "savedAt": state.get("savedAt"),
        "cookies": count(&state["cookies"]),
        "localStorage": count(&state["localStorage"]),
        "sessionStorage": count(&state["sessionStorage"]),
    }))
}

fn rename(old: &str, new: &str, caller: Caller) -> Result<Value, String> {
    let (from, to) = (path_of(old, caller)?, path_of(new, caller)?);
    if !from.is_file() {
        return Err(format!("FILE_NOT_FOUND: {}", from.display()));
    }
    if to.exists() {
        return Err(format!("{} already exists", to.display()));
    }
    std::fs::rename(&from, &to).map_err(|error| error.to_string())?;
    Ok(json!({"renamed": to.to_string_lossy()}))
}

fn remove(path: &Path) -> Result<Value, String> {
    std::fs::remove_file(path).map_err(|_| format!("FILE_NOT_FOUND: {}", path.display()))?;
    Ok(json!({"removed": 1}))
}

fn clear_all() -> Result<Value, String> {
    let files = saved_files();
    for path in &files {
        std::fs::remove_file(path).map_err(|error| error.to_string())?;
    }
    Ok(json!({"removed": files.len()}))
}

fn clean(days: Option<f64>) -> Result<Value, String> {
    let days: f64 = days
        .filter(|days: &f64| days.is_finite() && *days >= 0.0)
        .ok_or("INVALID_INPUT: olderThanDays is a number of days, 0 or more")?;
    let cutoff = SystemTime::now()
        .checked_sub(Duration::from_secs_f64(days * 86_400.0))
        .unwrap_or(SystemTime::UNIX_EPOCH);
    let mut removed = 0;
    for path in saved_files() {
        if modified(&path).is_some_and(|at| at < cutoff) {
            std::fs::remove_file(&path).map_err(|error| error.to_string())?;
            removed += 1;
        }
    }
    Ok(json!({"removed": removed}))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_paths_and_origins() {
        assert!(
            path_of("work", Caller::User)
                .unwrap()
                .ends_with("states/work.json")
        );
        assert_eq!(
            path_of("./a.json", Caller::User).unwrap(),
            PathBuf::from("./a.json")
        );
        assert!(path_of("../a.json", Caller::Agent).is_err());
        assert!(path_of("/tmp/x", Caller::Agent).is_err());
        assert_eq!(
            origin_of("https://ex.com:8443/a?b#c"),
            "https://ex.com:8443"
        );
        assert_eq!(origin_of("http://u:p@ex.com/"), "http://ex.com");
    }
}
