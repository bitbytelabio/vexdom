//! The settings the extension's settings page and the desktop app share (`src/daemon/preferences.ts`,
//! `src/lib/settings/preferences.ts`): config.json and profile.json are the only copy.

use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::config::{self, config_path, is_theme_id, read_agent_config};
use crate::guardrails::settings::{guardrail_settings, setting_writable};
use crate::log;
use crate::profile;
use vexdom_pure::profile::{PROFILE_FIELDS, UserProfile, as_user_profile, is_user_profile};
use vexdom_pure::secrets::find_secrets;

/// A result in `ActionResult` terms: `Ok(data)` or `Err((code, message))`.
pub type Outcome = Result<Value, (String, String)>;

/// `preferencesNow`: `{theme, guardrails, profile, pairing}`.
pub fn preferences_now() -> Value {
    let config = read_agent_config();
    json!({
        "theme": config.theme,
        "guardrails": guardrail_settings(&config.guardrails, &config.require_approval, &config_path().to_string_lossy()),
        "profile": profile::read_profile(),
        "pairing": {"mode": config::pairing_mode(&config)},
    })
}

fn invalid(message: String) -> Outcome {
    Err(("INVALID_INPUT".into(), message))
}

/// `applyPreference`.
pub fn apply_preference(change: &Value) -> Outcome {
    let kind = change.get("kind").and_then(Value::as_str);
    match kind {
        Some("theme") if change.get("theme").and_then(Value::as_str).is_some_and(is_theme_id) => {
            let theme = change["theme"].as_str().unwrap_or_default();
            if read_agent_config().theme.as_deref() != Some(theme) {
                config::write_theme(theme);
                log::log(&format!("theme → {theme}"));
            }
            Ok(preferences_now())
        }
        Some("profile") if change.get("profile").is_some_and(is_user_profile) => {
            save_profile(&as_user_profile(&change["profile"]))
        }
        Some("guardrail")
            if change.get("setting").is_some_and(Value::is_string)
                && change.get("value").is_some_and(|value| {
                    value.is_null()
                        || value.is_boolean()
                        || matches!(value.as_str(), Some("allow" | "confirm" | "deny"))
                }) =>
        {
            let setting = change["setting"].as_str().unwrap_or_default();
            let value = &change["value"];
            if !setting_writable(setting, value) {
                return invalid(format!(
                    "\"{setting}\" is not a guardrail the settings screen may set. Locked rules are edited in {}.",
                    config_path().display()
                ));
            }
            config::write_guardrail_setting(setting, value);
            let shown = match value {
                Value::Null => "default".to_owned(),
                Value::String(text) => text.clone(),
                other => other.to_string(),
            };
            log::log(&format!("guardrail {setting} → {shown}"));
            Ok(preferences_now())
        }
        _ => invalid(
            "Expected {kind:\"theme\", theme}, {kind:\"guardrail\", setting, value} or {kind:\"profile\", profile}."
                .into(),
        ),
    }
}

fn save_profile(next: &UserProfile) -> Outcome {
    let next = next.normalized();
    if let Some(label) = secret_in(&next) {
        return invalid(format!(
            "“{label}” looks like a password, key or card number. Vexdom keeps those sealed in the browser and never hands them to a model — leave it out of your profile."
        ));
    }
    if next != profile::read_profile() {
        if let Err(error) = profile::write_profile(&next) {
            return Err(("INTERNAL".into(), error.to_string()));
        }
        log::log(&format!(
            "profile saved ({} details, {} instructions)",
            next.fields.len() + next.details.len(),
            if next.instructions.is_empty() {
                "no"
            } else {
                "with"
            }
        ));
    }
    Ok(preferences_now())
}

fn secret_in(profile: &UserProfile) -> Option<String> {
    let mut entries: Vec<(String, String, bool)> = PROFILE_FIELDS
        .iter()
        .map(|(id, label)| {
            (
                (*label).to_owned(),
                profile.field(id).to_owned(),
                *id == "phone",
            )
        })
        .collect();
    entries.extend(profile.details.iter().map(|detail| {
        (
            detail.label.clone(),
            format!("{}: {}", detail.label, detail.value),
            false,
        )
    }));
    entries.push(("Instructions".into(), profile.instructions.clone(), false));
    entries
        .into_iter()
        .find(|(_, text, phone)| {
            find_secrets(text, &[])
                .iter()
                .any(|span| !(*phone && span.kind == "card"))
        })
        .map(|(label, _, _)| label)
}

/// What config.json and profile.json hold right now (`markConfig`): a digest of both files, and the
/// part a change of agent or model moves.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigMark {
    pub file: String,
    pub agent: String,
}

pub fn mark_config() -> ConfigMark {
    let text = std::fs::read_to_string(config_path()).unwrap_or_default();
    let mut hash = Sha256::new();
    hash.update(text.as_bytes());
    hash.update(b"\0");
    hash.update(profile::read_profile_text().as_bytes());
    let file = hash
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    let config = read_agent_config();
    let agent = format!(
        "{}|{}|{:?}|{:?}|{:?}",
        config.agent,
        config.claude.bin,
        config.claude.model,
        config.claude.effort,
        config.claude.transport
    );
    ConfigMark { file, agent }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth_store::tests::scratch_home;

    #[test]
    fn applies_changes() {
        let _home = scratch_home("preferences");
        let now = apply_preference(&json!({"kind": "theme", "theme": "midnight"})).unwrap();
        assert_eq!(now["theme"], "midnight");
        assert_eq!(now["pairing"]["mode"], "auto");
        let err = apply_preference(
            &json!({"kind": "guardrail", "setting": "secret-in-url", "value": "allow"}),
        )
        .unwrap_err();
        assert!(err.1.starts_with("\"secret-in-url\" is not a guardrail"));
        assert!(
            apply_preference(
                &json!({"kind": "guardrail", "setting": "raw-html-read", "value": "allow"})
            )
            .is_ok()
        );
        let err = apply_preference(&json!({"kind": "profile", "profile": {"fields": {}, "details": [], "instructions": "my password: hunter22!"}})).unwrap_err();
        assert!(err.1.starts_with("“Instructions” looks like"));
        assert!(apply_preference(&json!({"kind": "profile", "profile": {"fields": {"phone": "4242 4242 4242 4242"}, "details": [], "instructions": ""}})).is_ok());
        assert_eq!(
            apply_preference(&json!({"kind": "x"})).unwrap_err().0,
            "INVALID_INPUT"
        );
        let before = mark_config();
        config::write_active_agent("claude");
        assert_ne!(mark_config().file, before.file);
    }
}
