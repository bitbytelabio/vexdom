//! The policy described for the settings panel (`guardrails/settings.ts`, `src/lib/settings/guardrails.ts`).

use serde::Serialize;
use serde_json::Value;

use super::{DEFAULT_RULES, Effect, GuardrailConfig, PermissionMode, policy_from};

pub const FENCE_SETTING: &str = "fence";
pub const UNATTENDED_SETTING: &str = "unattended";
/// The side panel's permission mode: `rules`, `ask`, `plan` or `auto`.
pub const MODE_SETTING: &str = "mode";

/// Not preferences: shown, explained, not changeable from the panel.
const LOCKED: [&str; 4] = [
    "reserved-action",
    "non-http-navigation",
    "unreadable-navigation",
    "secret-in-url",
];

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct GuardrailToggle {
    pub id: &'static str,
    pub title: &'static str,
    pub reason: &'static str,
    pub fallback: Effect,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub r#override: Option<Effect>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub locked: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct GuardrailSwitch {
    pub enabled: bool,
    pub overridden: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Unattended {
    pub effect: Effect,
    pub overridden: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GuardrailSettings {
    pub rules: Vec<GuardrailToggle>,
    pub fence: GuardrailSwitch,
    pub unattended: Unattended,
    pub hosts: Vec<String>,
    pub config_path: String,
    pub mode: PermissionMode,
}

pub fn guardrail_settings(
    config: &GuardrailConfig,
    require_approval: &[String],
    config_path: &str,
) -> GuardrailSettings {
    let baseline = policy_from(
        &GuardrailConfig {
            rules: None,
            ..config.clone()
        },
        require_approval,
    );
    let rules = baseline
        .rules
        .iter()
        .map(|rule| GuardrailToggle {
            id: rule.id,
            title: rule.title,
            reason: rule.reason,
            fallback: rule.effect,
            r#override: config
                .rules
                .as_ref()
                .and_then(|rules| rules.get(rule.id))
                .and_then(Value::as_str)
                .and_then(Effect::parse),
            locked: LOCKED.contains(&rule.id).then_some(true),
        })
        .collect();
    GuardrailSettings {
        rules,
        fence: GuardrailSwitch {
            enabled: config.fence != Some(Value::Bool(false)),
            overridden: config.fence.is_some(),
        },
        unattended: Unattended {
            effect: if config.unattended.as_ref().and_then(Value::as_str) == Some("allow") {
                Effect::Allow
            } else {
                Effect::Deny
            },
            overridden: config.unattended.is_some(),
        },
        hosts: config.hosts.clone().unwrap_or_default(),
        config_path: config_path.to_owned(),
        mode: baseline.mode,
    }
}

/// Whether the panel may write this setting at all, and whether the value fits it.
pub fn setting_writable(setting: &str, value: &Value) -> bool {
    match setting {
        FENCE_SETTING => value.is_null() || value.is_boolean(),
        UNATTENDED_SETTING => value.is_null() || matches!(value.as_str(), Some("allow" | "deny")),
        MODE_SETTING => value.is_null() || value.as_str().and_then(PermissionMode::parse).is_some(),
        _ => {
            DEFAULT_RULES.iter().any(|rule| rule.id == setting)
                && !LOCKED.contains(&setting)
                && (value.is_null() || value.as_str().and_then(Effect::parse).is_some())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn settings_shape() {
        let config: GuardrailConfig =
            serde_json::from_value(json!({"rules": {"raw-html-read": "allow"}, "fence": false}))
                .unwrap();
        let settings = guardrail_settings(&config, &[], "/x/config.json");
        let raw = settings
            .rules
            .iter()
            .find(|rule| rule.id == "raw-html-read")
            .unwrap();
        assert_eq!(
            (raw.fallback, raw.r#override),
            (Effect::Deny, Some(Effect::Allow))
        );
        let form = settings
            .rules
            .iter()
            .find(|rule| rule.id == "form-submission")
            .unwrap();
        assert_eq!(form.fallback, Effect::Allow);
        assert!(!settings.fence.enabled && settings.fence.overridden);
        let text = serde_json::to_string(&settings.rules[0]).unwrap();
        assert!(text.contains("\"locked\":true"));
        assert!(setting_writable("fence", &json!(null)));
        assert!(setting_writable("raw-html-read", &json!("allow")));
        assert!(!setting_writable("secret-in-url", &json!("allow")));
        assert!(!setting_writable("nope", &json!("allow")));
    }
}
