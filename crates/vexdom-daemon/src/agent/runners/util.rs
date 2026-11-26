//! Helpers every runner shares (`runners/util.ts`).

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use serde_json::Value;
use vexdom_core::config::AgentSettings;
use vexdom_core::{agents, log};

const RUN_DIR_TTL: Duration = Duration::from_secs(24 * 60 * 60);

/// What the user reads when the agent CLI started but the browser tools it was given did not.
pub fn browser_tools_did_not_start(label: &str) -> String {
    format!(
        "{label} could not start Vexdom's browser tools, so this run could not reach the page. Run \"vexdom restart\", then send the message again."
    )
}

/// How to install an agent.
pub fn install_hint(kind: &str) -> &'static str {
    agents::agent(kind).map_or("", |agent| agent.install)
}

/// Concurrent runs each get their own workspace; yesterday's are nobody's.
pub fn sweep_run_dirs(base: &Path, ttl: Option<Duration>) {
    let Ok(entries) = std::fs::read_dir(base) else {
        return;
    };
    let cutoff = SystemTime::now()
        .checked_sub(ttl.unwrap_or(RUN_DIR_TTL))
        .unwrap_or(SystemTime::UNIX_EPOCH);
    for entry in entries.flatten() {
        let path = entry.path();
        let stale = std::fs::metadata(&path)
            .and_then(|meta| meta.modified())
            .is_ok_and(|modified| modified < cutoff);
        if stale && path.is_dir() {
            let _ = std::fs::remove_dir_all(&path);
        } else if stale {
            let _ = std::fs::remove_file(&path);
        }
    }
}

/// One path per conversation, spelled so it is safe as a path segment.
pub fn conversation_dir(base: &Path, key: &str) -> PathBuf {
    let safe: String = key
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    base.join(safe)
}

/// Drops a reasoning-effort name the CLI would reject rather than letting it fail the run.
pub fn effort_of(settings: &AgentSettings, accepted: &[&str]) -> Option<String> {
    let effort = settings
        .effort
        .as_deref()
        .filter(|effort| !effort.is_empty())?;
    if accepted.contains(&effort) {
        return Some(effort.to_owned());
    }
    log::log(&format!(
        "ignoring effort \"{effort}\" — accepted values are {}",
        accepted.join(", ")
    ));
    None
}

/// Pulls the outermost JSON object out of text a CLI may have wrapped in prose.
pub fn parse_json_blob(text: &str) -> Option<Value> {
    let start = text.find('{')?;
    let end = text.rfind('}')?;
    if end <= start {
        return None;
    }
    serde_json::from_str(&text[start..=end]).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::test_support::scratch;
    use serde_json::json;

    fn aged(base: &Path, name: &str, hours_ago: u64) -> PathBuf {
        let dir = base.join(name);
        std::fs::create_dir_all(&dir).unwrap();
        let at = SystemTime::now() - Duration::from_secs(hours_ago * 60 * 60);
        std::fs::File::open(&dir).unwrap().set_modified(at).unwrap();
        dir
    }

    #[test]
    fn sweeping_keeps_what_is_newer_than_a_day() {
        let home = scratch("sweep");
        let base = home.path().join("sweep-test");
        let (old, recent) = (aged(&base, "yesterday", 25), aged(&base, "this-morning", 3));
        sweep_run_dirs(&base, None);
        assert_eq!([old.exists(), recent.exists()], [false, true]);
        let hour = aged(&base, "an-hour-ago", 1);
        sweep_run_dirs(&base, Some(Duration::from_secs(30 * 60)));
        assert!(
            !hour.exists(),
            "a shortened age limit sweeps an hour-old folder"
        );
        sweep_run_dirs(&home.path().join("never-made"), None);
    }

    #[test]
    fn efforts_a_cli_would_reject_are_dropped_and_logged() {
        let _home = scratch("effort");
        let settings = |effort: Option<&str>| AgentSettings {
            bin: "x".into(),
            model: None,
            effort: effort.map(str::to_owned),
            transport: None,
        };
        assert_eq!(
            effort_of(&settings(Some("high")), &["low", "high"]).as_deref(),
            Some("high")
        );
        assert_eq!(effort_of(&settings(None), &["low", "high"]), None);
        assert_eq!(effort_of(&settings(Some("turbo")), &["low", "high"]), None);
        let logged = std::fs::read_to_string(vexdom_core::paths::log_path()).unwrap();
        assert!(logged.contains("ignoring effort \"turbo\" — accepted values are low, high"));
    }

    #[test]
    fn json_wrapped_in_prose_is_found() {
        assert_eq!(
            parse_json_blob(
                "Here is the summary:\n{\"title\":\"Pricing\",\"pages\":3}\nHope that helps!"
            ),
            Some(json!({"title": "Pricing", "pages": 3}))
        );
        assert_eq!(parse_json_blob("no braces here"), None);
        assert_eq!(parse_json_blob("} backwards {"), None);
        assert_eq!(parse_json_blob("{\"broken\": }"), None);
    }

    #[test]
    fn conversation_folders_are_safe_path_segments() {
        assert_eq!(
            conversation_dir(Path::new("/p"), "run-1/../x y"),
            Path::new("/p/run-1____x_y")
        );
        assert_eq!(install_hint("claude"), agents::CLAUDE.install);
    }
}
