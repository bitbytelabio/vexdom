//! `config.json` (`src/daemon/agent/config.ts`): read on every use, written read-modify-write so
//! every key someone else put there survives, in its place.

use std::path::PathBuf;

use serde_json::{Map, Value, json};

use crate::guardrails::GuardrailConfig;
use crate::guardrails::settings::{FENCE_SETTING, UNATTENDED_SETTING};
use crate::{agents, log, pairing, paths};

pub fn config_path() -> PathBuf {
    paths::state_dir().join("config.json")
}

pub const THEMES: [&str; 5] = ["neon", "ember", "midnight", "phosphor", "daylight"];

pub fn is_theme_id(value: &str) -> bool {
    THEMES.contains(&value)
}

/// One agent CLI's settings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentSettings {
    pub bin: String,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub transport: Option<String>,
}

/// The parsed config: typed fields the Bridge reads, plus the raw object for everything else.
#[derive(Debug, Clone, PartialEq)]
pub struct AgentConfig {
    pub agent: String,
    pub claude: AgentSettings,
    pub require_approval: Vec<String>,
    pub theme: Option<String>,
    pub guardrails: GuardrailConfig,
    pub raw: Map<String, Value>,
}

impl AgentConfig {
    fn text_field(&self, key: &str) -> Option<String> {
        self.raw.get(key).and_then(Value::as_str).map(str::to_owned)
    }

    pub fn screenshot_dir(&self) -> Option<String> {
        self.text_field("screenshotDir")
    }

    pub fn download_dir(&self) -> Option<String> {
        self.text_field("downloadDir")
    }

    pub fn skills_dir(&self) -> Option<String> {
        self.text_field("skillsDir")
    }

    pub fn extension_dir(&self) -> Option<String> {
        self.text_field("extensionDir")
    }

    pub fn download_ttl_days(&self) -> Option<f64> {
        self.raw.get("downloadTtlDays").and_then(Value::as_f64)
    }

    /// The active agent's settings (`activeAgent`).
    pub fn active(&self) -> (&str, &AgentSettings) {
        (&self.agent, &self.claude)
    }
}

fn read_stored() -> Map<String, Value> {
    std::fs::read(config_path())
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        .and_then(|value| match value {
            Value::Object(map) => Some(map),
            _ => None,
        })
        .unwrap_or_default()
}

fn write(config: &Map<String, Value>) {
    let path = config_path();
    let result = paths::ensure_private_dir(&paths::state_dir()).and_then(|()| {
        let mut text = serde_json::to_string_pretty(config).map_err(std::io::Error::other)?;
        text.push('\n');
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
        std::io::Write::write_all(&mut options.open(&path)?, text.as_bytes())
    });
    if let Err(error) = result {
        log::log_detail("could not write config.json", &error.to_string());
    }
}

/// config.json as an object for a writer that must not lose what is there: no file is empty; a
/// file that is not a JSON object is an error, never replaced.
pub(crate) fn read_strict() -> Result<Map<String, Value>, String> {
    let path = config_path();
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Map::new()),
        Err(error) => {
            return Err(format!("Failed to read config {}: {error}", path.display()));
        }
    };
    match serde_json::from_slice::<Value>(&bytes) {
        Ok(Value::Object(map)) => Ok(map),
        Ok(_) => Err(format!("Config {} must be a JSON object", path.display())),
        Err(error) => Err(format!(
            "Config {} is invalid JSON: {error}",
            path.display()
        )),
    }
}

/// Sets `plugins` (`vexdom plugin add`), every other key kept in its place.
pub fn write_plugins(plugins: &Value) -> Result<(), String> {
    let mut stored = read_strict()?;
    stored.insert("plugins".into(), plugins.clone());
    let path = config_path();
    paths::write_private_json(&path, &stored)
        .map_err(|error| format!("Failed to write config {}: {error}", path.display()))
}

fn text(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_owned)
}

/// A hand-edited model a CLI could read as a flag is left out.
fn model_of(kind: &str, value: Option<&Value>) -> Option<String> {
    let model = text(value)?;
    if agents::is_model_id(&model) {
        return Some(model);
    }
    log::log(&format!(
        "ignoring {kind} model {} — a model id starts with a letter or digit and has no spaces",
        Value::String(model)
    ));
    None
}

pub fn read_agent_config() -> AgentConfig {
    let stored = read_stored();
    let scoped = stored
        .get("agents")
        .and_then(|agents| agents.get("claude"))
        .cloned()
        .unwrap_or(Value::Null);
    let claude = AgentSettings {
        bin: text(scoped.get("bin"))
            .or_else(|| text(stored.get("claudeBin")))
            .unwrap_or_else(|| agents::CLAUDE.bin.to_owned()),
        model: model_of("claude", scoped.get("model"))
            .or_else(|| model_of("claude", stored.get("model")))
            .or_else(|| Some("sonnet".to_owned())),
        effort: text(scoped.get("effort")).or_else(|| text(stored.get("effort"))),
        transport: text(scoped.get("transport")),
    };
    let agent = stored
        .get("agent")
        .and_then(Value::as_str)
        .filter(|kind| agents::is_agent_kind(kind))
        .unwrap_or(agents::DEFAULT_AGENT)
        .to_owned();
    let require_approval = match stored.get("requireApproval") {
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(|item| item.as_str().map(str::to_owned))
            .collect(),
        _ => vec!["page.submitForm".to_owned()],
    };
    let theme = stored
        .get("theme")
        .and_then(Value::as_str)
        .filter(|theme| is_theme_id(theme))
        .map(str::to_owned);
    let guardrails = stored
        .get("guardrails")
        .and_then(|value| serde_json::from_value(value.clone()).ok())
        .unwrap_or_default();
    AgentConfig {
        agent,
        claude,
        require_approval,
        theme,
        guardrails,
        raw: stored,
    }
}

/// How a browser pairs through the native host; an unknown hand-edited mode pairs by code only.
pub fn pairing_mode(config: &AgentConfig) -> String {
    let mode = match config.raw.get("pairing") {
        Some(Value::Object(object)) => object.get("mode"),
        other => other,
    };
    match mode {
        None => pairing::DEFAULT_PAIRING_MODE.to_owned(),
        Some(Value::String(mode)) if pairing::is_pairing_mode(mode) => mode.clone(),
        Some(other) => {
            log::log(&format!(
                "pairing mode {other} is not auto, ask or code; pairing by code only"
            ));
            "code".to_owned()
        }
    }
}

pub fn write_pairing_mode(mode: &str) {
    let mut stored = read_stored();
    let mut object = match stored.get("pairing") {
        Some(Value::Object(object)) => object.clone(),
        _ => Map::new(),
    };
    object.insert("mode".into(), Value::String(mode.to_owned()));
    stored.insert("pairing".into(), Value::Object(object));
    write(&stored);
}

fn clamp(value: Option<&Value>, fallback: u64, max: u64) -> u64 {
    match value.and_then(Value::as_f64) {
        Some(number) if number.is_finite() && number >= 1.0 => (number.floor() as u64).min(max),
        _ => fallback,
    }
}

pub fn max_concurrent_runs(config: &AgentConfig) -> u64 {
    clamp(config.raw.get("maxConcurrentRuns"), 3, 8)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SiteMapSettings {
    pub research: bool,
    pub allow_clicks: bool,
    pub max_pages: u64,
    pub max_screenshots: u64,
    pub timeout_ms: u64,
}

pub fn site_map_settings(config: &AgentConfig) -> SiteMapSettings {
    let stored = config.raw.get("siteMap").cloned().unwrap_or(json!({}));
    SiteMapSettings {
        research: stored.get("research") != Some(&Value::Bool(false)),
        allow_clicks: stored.get("allowClicks") == Some(&Value::Bool(true)),
        max_pages: clamp(stored.get("maxPages"), 15, 40),
        max_screenshots: clamp(stored.get("maxScreenshots"), 10, 24),
        timeout_ms: clamp(stored.get("timeoutMs"), 10 * 60_000, 30 * 60_000),
    }
}

pub fn write_active_agent(kind: &str) {
    let mut stored = read_stored();
    stored.insert("agent".into(), Value::String(kind.to_owned()));
    write(&stored);
}

/// Sets or clears one agent's model. False, and nothing written, for an id a CLI could misread.
pub fn write_agent_model(kind: &str, model: Option<&str>) -> bool {
    let value = model.map(str::trim).filter(|model| !model.is_empty());
    if value.is_some_and(|value| !agents::is_model_id(value)) {
        return false;
    }
    let mut stored = read_stored();
    let mut agents_map = match stored.get("agents") {
        Some(Value::Object(map)) => map.clone(),
        _ => Map::new(),
    };
    let mut scoped = match agents_map.get(kind) {
        Some(Value::Object(map)) => map.clone(),
        _ => Map::new(),
    };
    match value {
        Some(value) => {
            scoped.insert("model".into(), Value::String(value.to_owned()));
        }
        None => {
            scoped.shift_remove("model");
        }
    }
    if scoped.is_empty() {
        agents_map.shift_remove(kind);
    } else {
        agents_map.insert(kind.to_owned(), Value::Object(scoped));
    }
    if agents_map.is_empty() {
        stored.shift_remove("agents");
    } else {
        stored.insert("agents".into(), Value::Object(agents_map));
    }
    if kind == "claude" {
        stored.shift_remove("model");
    }
    write(&stored);
    true
}

/// One guardrail override; `null` removes it.
pub fn write_guardrail_setting(setting: &str, value: &Value) {
    let mut stored = read_stored();
    let mut guardrails = match stored.get("guardrails") {
        Some(Value::Object(map)) => map.clone(),
        _ => Map::new(),
    };
    if setting == FENCE_SETTING {
        if value.is_null() {
            guardrails.shift_remove("fence");
        } else {
            guardrails.insert("fence".into(), Value::Bool(value == &Value::Bool(true)));
        }
    } else if setting == UNATTENDED_SETTING {
        if value.is_null() {
            guardrails.shift_remove("unattended");
        } else {
            let effect = if value.as_str() == Some("allow") {
                "allow"
            } else {
                "deny"
            };
            guardrails.insert("unattended".into(), Value::String(effect.into()));
        }
    } else {
        let mut rules = match guardrails.get("rules") {
            Some(Value::Object(map)) => map.clone(),
            _ => Map::new(),
        };
        if value.is_null() {
            rules.shift_remove(setting);
        } else {
            rules.insert(setting.to_owned(), value.clone());
        }
        if rules.is_empty() {
            guardrails.shift_remove("rules");
        } else {
            guardrails.insert("rules".into(), Value::Object(rules));
        }
    }
    if guardrails.is_empty() {
        stored.shift_remove("guardrails");
    } else {
        stored.insert("guardrails".into(), Value::Object(guardrails));
    }
    write(&stored);
}

pub fn write_theme(theme: &str) {
    let mut stored = read_stored();
    stored.insert("theme".into(), Value::String(theme.to_owned()));
    write(&stored);
}

/// Remembers where `setup --dir` installed.
pub fn remember_extension_dir(dir: Option<&str>) {
    let mut stored = read_stored();
    match dir {
        Some(dir) => {
            stored.insert("extensionDir".into(), Value::String(dir.to_owned()));
        }
        None => {
            stored.shift_remove("extensionDir");
        }
    }
    write(&stored);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth_store::tests::scratch_home;

    #[test]
    fn defaults_and_legacy() {
        let _home = scratch_home("config");
        let config = read_agent_config();
        assert_eq!(config.agent, "claude");
        assert_eq!(config.claude.model.as_deref(), Some("sonnet"));
        assert_eq!(config.require_approval, ["page.submitForm"]);
        assert_eq!(pairing_mode(&config), "auto");
        paths::write_private_json(
            &config_path(),
            &json!({"claudeBin": "/x/claude", "model": "-bad", "pairing": "nope", "keep": 1}),
        )
        .unwrap();
        let config = read_agent_config();
        assert_eq!(config.claude.bin, "/x/claude");
        assert_eq!(config.claude.model.as_deref(), Some("sonnet"));
        assert_eq!(pairing_mode(&config), "code");
        write_pairing_mode("ask");
        assert!(write_agent_model("claude", Some("opus")));
        assert!(!write_agent_model("claude", Some("--x")));
        write_guardrail_setting("raw-html-read", &json!("allow"));
        write_guardrail_setting("fence", &json!(false));
        let text = std::fs::read_to_string(config_path()).unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(&text).unwrap(),
            json!({"claudeBin": "/x/claude", "pairing": {"mode": "ask"}, "keep": 1,
                   "agents": {"claude": {"model": "opus"}},
                   "guardrails": {"rules": {"raw-html-read": "allow"}, "fence": false}})
        );
        assert!(text.starts_with("{\n  \"claudeBin\""));
        write_guardrail_setting("raw-html-read", &Value::Null);
        write_guardrail_setting("fence", &Value::Null);
        assert!(read_stored().get("guardrails").is_none());
        let config = read_agent_config();
        assert_eq!(max_concurrent_runs(&config), 3);
        assert_eq!(site_map_settings(&config).max_pages, 15);
    }
}
