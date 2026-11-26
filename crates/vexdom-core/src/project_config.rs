//! CLI preferences: the user's own in `~/.vexdom/config.json` under `"cli"`, overridden by a
//! project's `./vexdom.json` (the working directory only, never a parent). A project file is
//! untrusted: only these display and timing preferences are read from it.

use std::path::Path;

use serde_json::{Map, Value};

use crate::paths;

pub const PROJECT_FILE: &str = "vexdom.json";

/// The preferences a project may set.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CliPrefs {
    pub json: Option<bool>,
    pub max_output: Option<usize>,
    pub annotate: Option<bool>,
    pub screenshot_format: Option<String>,
    pub screenshot_quality: Option<u32>,
    pub timeout_ms: Option<u64>,
}

const KEYS: [&str; 6] = [
    "json",
    "maxOutput",
    "annotate",
    "screenshotFormat",
    "screenshotQuality",
    "timeoutMs",
];

impl CliPrefs {
    /// Fields of `other` that are set win.
    fn overlay(self, other: CliPrefs) -> CliPrefs {
        CliPrefs {
            json: other.json.or(self.json),
            max_output: other.max_output.or(self.max_output),
            annotate: other.annotate.or(self.annotate),
            screenshot_format: other.screenshot_format.or(self.screenshot_format),
            screenshot_quality: other.screenshot_quality.or(self.screenshot_quality),
            timeout_ms: other.timeout_ms.or(self.timeout_ms),
        }
    }
}

/// The known keys of one object; `Err` when one of them has the wrong type.
fn read(object: &Map<String, Value>) -> Result<CliPrefs, ()> {
    fn get<T>(
        object: &Map<String, Value>,
        key: &str,
        read: impl Fn(&Value) -> Option<T>,
    ) -> Result<Option<T>, ()> {
        match object.get(key) {
            None | Some(Value::Null) => Ok(None),
            Some(value) => read(value).map(Some).ok_or(()),
        }
    }
    Ok(CliPrefs {
        json: get(object, "json", Value::as_bool)?,
        max_output: get(object, "maxOutput", |v| {
            v.as_u64().and_then(|n| usize::try_from(n).ok())
        })?,
        annotate: get(object, "annotate", Value::as_bool)?,
        screenshot_format: get(object, "screenshotFormat", |v| {
            v.as_str()
                .filter(|f| ["png", "jpeg"].contains(f))
                .map(str::to_owned)
        })?,
        screenshot_quality: get(object, "screenshotQuality", |v| {
            v.as_u64()
                .filter(|q| *q <= 100)
                .and_then(|q| u32::try_from(q).ok())
        })?,
        timeout_ms: get(object, "timeoutMs", Value::as_u64)?,
    })
}

fn user_prefs() -> CliPrefs {
    let Ok(text) = std::fs::read_to_string(paths::state_dir().join("config.json")) else {
        return CliPrefs::default();
    };
    serde_json::from_str::<Value>(&text)
        .ok()
        .as_ref()
        .and_then(|config| config.get("cli"))
        .and_then(Value::as_object)
        .and_then(|cli| read(cli).ok())
        .unwrap_or_default()
}

/// The user's preferences with the project's on top, and warnings for stderr.
pub fn load(cwd: &Path) -> (CliPrefs, Vec<String>) {
    let user = user_prefs();
    let Ok(text) = std::fs::read_to_string(cwd.join(PROJECT_FILE)) else {
        return (user, Vec::new());
    };
    let invalid = || {
        format!(
            "invalid config file ./{PROJECT_FILE}: expected a JSON object with correctly typed preferences"
        )
    };
    let Some(object) = serde_json::from_str::<Value>(&text)
        .ok()
        .and_then(|value| value.as_object().cloned())
    else {
        return (user, vec![invalid()]);
    };
    let Ok(project) = read(&object) else {
        return (user, vec![invalid()]);
    };
    let ignored = object
        .keys()
        .filter(|key| *key != "$schema" && !KEYS.contains(&key.as_str()))
        .count();
    let mut warnings = Vec::new();
    if ignored > 0 {
        warnings.push(format!(
            "⚠ ignoring {ignored} unsupported settings from ./{PROJECT_FILE} (project config is untrusted). Put them in ~/.vexdom/config.json under \"cli\"."
        ));
    }
    (user.overlay(project), warnings)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("vexdom-project-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_project_sets_only_its_six_keys() {
        let dir = scratch("keys");
        std::fs::write(
            dir.join(PROJECT_FILE),
            r#"{"$schema": "x", "json": true, "maxOutput": 500, "plugins": [], "allowedDomains": []}"#,
        )
        .unwrap();
        let (prefs, warnings) = load(&dir);
        assert_eq!(prefs.json, Some(true));
        assert_eq!(prefs.max_output, Some(500));
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].starts_with("⚠ ignoring 2 unsupported settings"));
    }

    #[test]
    fn a_wrongly_typed_project_is_skipped() {
        let dir = scratch("typed");
        std::fs::write(dir.join(PROJECT_FILE), r#"{"json": "yes"}"#).unwrap();
        let (prefs, warnings) = load(&dir);
        assert_eq!(prefs.json, None);
        assert!(warnings[0].starts_with("invalid config file ./vexdom.json"));
    }
}
