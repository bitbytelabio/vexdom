//! Guardrails (`src/daemon/guardrails/`): what an agent may do in a page (rules as data over a
//! closed set of conditions), how page text reaches the model (fence), what a credential looks like
//! on its way out (secrets), the policy as the settings panel shows it, and what a spawned agent
//! CLI may touch (spawn).

pub mod fence;
pub mod scope;
pub mod secrets;
pub mod settings;
pub mod spawn;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::proto::RESERVED_PREFIX;
use scope::{
    Scope, TargetKind, host_allowed, host_bearing_url, target_url, targets_another_tab,
    url_payload_bytes,
};
use secrets::{handles_in, sealed_handles};

pub const SUBMIT_ACTION: &str = "page.submitForm";
pub const UPLOAD_ACTION: &str = "page.attachFile";
pub const DOWNLOAD_ACTION: &str = "page.captureDownload";
pub const EXTRACT_ACTION: &str = "page.extractText";
pub const CAPTCHA_ACTION: &str = "page.solveCaptcha";
pub const NETWORK_ACTION: &str = "page.readNetwork";
pub const INJECT_ACTION: &str = "page.injectCode";
pub const RUN_CODE_ACTION: &str = "page.runCode";
pub const SITE_TOOL_ACTION: &str = "page.callSiteTool";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Effect {
    Allow,
    Confirm,
    Deny,
}

impl Effect {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "allow" => Some(Self::Allow),
            "confirm" => Some(Self::Confirm),
            "deny" => Some(Self::Deny),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Allow => "allow",
            Self::Confirm => "confirm",
            Self::Deny => "deny",
        }
    }
}

/// Who is asking: a side-panel run (`Agent`, a person watching) or any MCP client (`External`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Caller {
    Agent,
    External,
}

pub struct GuardrailRequest<'a> {
    pub action: &'a str,
    pub input: &'a Value,
    pub caller: Caller,
    pub scope: &'a Scope,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Condition {
    ReservedAction,
    NonHttpNavigation,
    UnreadableNavigation,
    NavigatesOffScope,
    CarriesUrlPayload,
    SubmitsForm,
    UploadsFile,
    DownloadsFile,
    LeavesPinnedTab,
    AnswersCaptcha,
    CallsSiteTool,
    InjectsCode,
    InjectsCodeOutsideThePanel,
    RunsCodeOutsideThePanel,
    ReleasesSecret,
    ReleasesSecretOffScope,
    CarriesSecretInUrl,
    ReadsRawHtml,
    ReadsResponseBodies,
    ListedInConfig,
}

impl Condition {
    fn holds(self, request: &GuardrailRequest<'_>, policy: &Policy) -> bool {
        let action = request.action;
        let input = request.input;
        match self {
            Self::ReservedAction => action.starts_with(RESERVED_PREFIX),
            Self::NonHttpNavigation => host_bearing_url(action, input)
                .is_some_and(|url| url.scheme() != "http" && url.scheme() != "https"),
            Self::UnreadableNavigation => {
                target_url(action, input).is_some_and(|target| target.kind == TargetKind::Opaque)
            }
            Self::NavigatesOffScope => host_bearing_url(action, input)
                .is_some_and(|url| !host_allowed(&scope::hostname(&url), &request.scope.hosts)),
            Self::CarriesUrlPayload => {
                action != DOWNLOAD_ACTION
                    && target_url(action, input)
                        .and_then(|target| target.url)
                        .is_some_and(|url| {
                            url_payload_bytes(&url) as f64 > policy.url_payload_bytes
                        })
            }
            Self::SubmitsForm => submits_form(action, input),
            Self::UploadsFile => action == UPLOAD_ACTION,
            Self::DownloadsFile => action == DOWNLOAD_ACTION,
            Self::LeavesPinnedTab => targets_another_tab(action, input, request.scope),
            Self::AnswersCaptcha => action == CAPTCHA_ACTION,
            Self::CallsSiteTool => action == SITE_TOOL_ACTION,
            Self::InjectsCode => action == INJECT_ACTION,
            Self::InjectsCodeOutsideThePanel => {
                action == INJECT_ACTION && request.caller == Caller::External
            }
            Self::RunsCodeOutsideThePanel => {
                action == RUN_CODE_ACTION && request.caller == Caller::External
            }
            Self::ReleasesSecret => !sealed_handles(input).is_empty(),
            Self::ReleasesSecretOffScope => sealed_handles(input).iter().any(|handle| {
                handle
                    .origin
                    .as_deref()
                    .is_some_and(|origin| !host_allowed(origin, &request.scope.hosts))
            }),
            Self::CarriesSecretInUrl => {
                target_url(action, input).is_some()
                    && input
                        .get("url")
                        .and_then(Value::as_str)
                        .is_some_and(|url| !handles_in(url).is_empty())
            }
            Self::ReadsRawHtml => {
                action == EXTRACT_ACTION
                    && input.get("format").and_then(Value::as_str) == Some("html")
            }
            Self::ReadsResponseBodies => {
                action == NETWORK_ACTION && input.get("includeBodies") == Some(&Value::Bool(true))
            }
            Self::ListedInConfig => {
                action != SUBMIT_ACTION
                    && policy
                        .require_approval
                        .iter()
                        .any(|listed| listed == action)
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rule {
    pub id: &'static str,
    pub when: Condition,
    pub effect: Effect,
    pub title: &'static str,
    pub reason: &'static str,
}

const fn rule(
    id: &'static str,
    when: Condition,
    effect: Effect,
    title: &'static str,
    reason: &'static str,
) -> Rule {
    Rule {
        id,
        when,
        effect,
        title,
        reason,
    }
}

pub const DEFAULT_RULES: &[Rule] = &[
    rule(
        "reserved-action",
        Condition::ReservedAction,
        Effect::Deny,
        "Reserved action",
        "That action is internal to Vexdom and cannot be called.",
    ),
    rule(
        "non-http-navigation",
        Condition::NonHttpNavigation,
        Effect::Deny,
        "Non-http navigation",
        "Only http(s) URLs can be opened.",
    ),
    rule(
        "unreadable-navigation",
        Condition::UnreadableNavigation,
        Effect::Deny,
        "URL with no readable destination",
        "That URL does not resolve to a destination Vexdom can check. Pass an absolute https:// URL.",
    ),
    rule(
        "off-scope-navigation",
        Condition::NavigatesOffScope,
        Effect::Confirm,
        "Leaves the sites this run is about",
        "That URL is not on a site this run was asked about.",
    ),
    rule(
        "url-payload",
        Condition::CarriesUrlPayload,
        Effect::Confirm,
        "Carries a large payload in the URL",
        "That URL carries an unusually large query string, which is how page content gets smuggled out.",
    ),
    rule(
        "form-submission",
        Condition::SubmitsForm,
        Effect::Confirm,
        "Submits a form",
        "Submitting a form is a consequential action.",
    ),
    rule(
        "site-tool-call",
        Condition::CallsSiteTool,
        Effect::Confirm,
        "Calls a tool the site provides",
        "A WebMCP site tool runs the site’s own code and can act on the user’s account in one call.",
    ),
    rule(
        "file-upload",
        Condition::UploadsFile,
        Effect::Confirm,
        "Uploads one of the user’s files",
        "Putting a file into a page hands it to whoever runs that site.",
    ),
    rule(
        "file-download",
        Condition::DownloadsFile,
        Effect::Confirm,
        "Saves a file from the page to disk",
        "That writes a file the page chose into the user’s download folder.",
    ),
    rule(
        "code-injection",
        Condition::InjectsCode,
        Effect::Confirm,
        "Runs code it wrote in the page",
        "That installs JavaScript the agent wrote into the page, with your logged-in session.",
    ),
    rule(
        "external-code-injection",
        Condition::InjectsCodeOutsideThePanel,
        Effect::Deny,
        "Installs code from outside the panel",
        "Installing page code needs a person to read it first, and an MCP client has nobody to show it to. Ask from the Vexdom side panel instead.",
    ),
    rule(
        "external-code-execution",
        Condition::RunsCodeOutsideThePanel,
        Effect::Deny,
        "Calls injected code from outside the panel",
        "Code installed by page.injectCode was reviewed and approved for the side-panel conversation that asked for it. It is not available to an MCP client.",
    ),
    rule(
        "leaves-pinned-tab",
        Condition::LeavesPinnedTab,
        Effect::Confirm,
        "Moves to another tab",
        "That tab is not the one this run was pointed at, and may hold a different logged-in session.",
    ),
    rule(
        "captcha-solve",
        Condition::AnswersCaptcha,
        Effect::Confirm,
        "Answers a captcha",
        "That ticks a site’s “I am a human” check, and answers any image challenge it sets, on your behalf.",
    ),
    rule(
        "secret-release",
        Condition::ReleasesSecret,
        Effect::Confirm,
        "Types a saved secret into the page",
        "That field holds a credential Vexdom sealed earlier.",
    ),
    rule(
        "secret-off-scope",
        Condition::ReleasesSecretOffScope,
        Effect::Confirm,
        "Uses a secret from another site",
        "That credential was read on a different site to the one this run is about.",
    ),
    rule(
        "secret-in-url",
        Condition::CarriesSecretInUrl,
        Effect::Deny,
        "Puts a secret in a URL",
        "A sealed secret cannot travel in a URL. Type it into the field it belongs in and Vexdom will release it there.",
    ),
    rule(
        "config-require-approval",
        Condition::ListedInConfig,
        Effect::Confirm,
        "Listed in requireApproval",
        "The user asked to approve this action every time.",
    ),
    rule(
        "network-body-read",
        Condition::ReadsResponseBodies,
        Effect::Deny,
        "Reads response bodies",
        "Reading response bodies is disabled by policy — they carry session tokens and personal data wholesale. Status, timing and headers are available without it.",
    ),
    rule(
        "raw-html-read",
        Condition::ReadsRawHtml,
        Effect::Deny,
        "Reads raw HTML",
        "Reading raw HTML is disabled by policy. Use the default text format instead.",
    ),
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FencePolicy {
    pub enabled: bool,
    pub except: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Policy {
    pub rules: Vec<Rule>,
    pub require_approval: Vec<String>,
    /// What `confirm` becomes for a caller with no way to ask.
    pub unattended: Effect,
    pub url_payload_bytes: f64,
    pub fence: FencePolicy,
    /// How often a side-panel run stops to ask; scheduled runs and outside callers keep to `Rules`.
    pub mode: PermissionMode,
}

/// How a side-panel run asks before acting. `decide` is the same in every mode; the agent service
/// adds or skips the asking around it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PermissionMode {
    /// Acts freely and asks only where the rules say `confirm`.
    #[default]
    Rules,
    /// Asks the first time it acts on each site in a conversation.
    Ask,
    /// Shows its plan and the sites it needs first, then asks before going anywhere else.
    Plan,
    /// Never stops to ask; what the rules block stays blocked.
    Auto,
}

impl PermissionMode {
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "rules" => Some(Self::Rules),
            "ask" => Some(Self::Ask),
            "plan" => Some(Self::Plan),
            "auto" => Some(Self::Auto),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Rules => "rules",
            Self::Ask => "ask",
            Self::Plan => "plan",
            Self::Auto => "auto",
        }
    }
}

/// Page actions that touch no page and so never need a site's permission.
pub const HOSTLESS_ACTIONS: &[&str] = &[
    "page.listFiles",
    "page.listDownloads",
    "page.listRecordings",
    "page.readRecording",
    "page.monitorStatus",
    "page.timerStatus",
    "page.siteToolResult",
    "page.cancelSiteTool",
    "page.startTimer",
    "page.stopTimer",
    "page.stopMonitor",
    "page.awaitMonitor",
    "page.readConsole",
    "page.readNetwork",
    "page.stopDiagnostics",
    "page.checkSites",
    "page.switchTab",
    "page.closeTab",
    "page.resizeWindow",
];

/// Whether an action works on a site, and so needs that site's permission in Ask and Plan modes.
pub fn needs_site(action: &str) -> bool {
    action.starts_with("page.") && !HOSTLESS_ACTIONS.contains(&action)
}

/// `guardrails` in config.json. Kept as loose JSON fields so unknown values read as absent.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GuardrailConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rules: Option<serde_json::Map<String, Value>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unattended: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url_payload_bytes: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hosts: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fence: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<Value>,
}

const DEFAULT_URL_PAYLOAD_BYTES: f64 = 512.0;

fn default_fence(enabled: bool) -> FencePolicy {
    FencePolicy {
        enabled,
        except: ["page.closeTab", "page.stopMonitor", "page.screenshot"]
            .map(str::to_owned)
            .to_vec(),
    }
}

/// `policyFrom`.
pub fn policy_from(config: &GuardrailConfig, require_approval: &[String]) -> Policy {
    let rules = DEFAULT_RULES
        .iter()
        .map(|rule| {
            let legacy = if rule.id == "form-submission"
                && !require_approval
                    .iter()
                    .any(|action| action == SUBMIT_ACTION)
            {
                Effect::Allow
            } else {
                rule.effect
            };
            let effect = config
                .rules
                .as_ref()
                .and_then(|rules| rules.get(rule.id))
                .and_then(Value::as_str)
                .and_then(Effect::parse)
                .unwrap_or(legacy);
            Rule {
                effect,
                ..rule.clone()
            }
        })
        .collect();
    Policy {
        rules,
        require_approval: require_approval.to_vec(),
        unattended: if config.unattended.as_ref().and_then(Value::as_str) == Some("allow") {
            Effect::Allow
        } else {
            Effect::Deny
        },
        url_payload_bytes: config
            .url_payload_bytes
            .as_ref()
            .and_then(Value::as_f64)
            .filter(|bytes| *bytes >= 0.0)
            .unwrap_or(DEFAULT_URL_PAYLOAD_BYTES),
        fence: default_fence(config.fence != Some(Value::Bool(false))),
        mode: config
            .mode
            .as_ref()
            .and_then(Value::as_str)
            .and_then(PermissionMode::parse)
            .unwrap_or_default(),
    }
}

/// Whether an action commits a form, however it is spelled.
pub fn submits_form(action: &str, input: &Value) -> bool {
    match action {
        SUBMIT_ACTION => true,
        "page.fillInput" | "page.typeText" => input.get("pressEnter") == Some(&Value::Bool(true)),
        "page.pressKey" => input.get("key").and_then(Value::as_str) == Some("Enter"),
        _ => false,
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Decision {
    /// Fully resolved: an external caller's `Confirm` waits for the user (ADR-032).
    pub effect: Effect,
    pub matched: Vec<Rule>,
    /// What to tell the agent; empty when nothing is refused.
    pub reason: String,
}

/// Every matching rule is collected and the most severe effect wins.
pub fn decide(request: &GuardrailRequest<'_>, policy: &Policy) -> Decision {
    let matched: Vec<&Rule> = policy
        .rules
        .iter()
        .filter(|rule| rule.effect != Effect::Allow && rule.when.holds(request, policy))
        .collect();
    let Some(worst) = matched.iter().map(|rule| rule.effect).max() else {
        return Decision {
            effect: Effect::Allow,
            matched: Vec::new(),
            reason: String::new(),
        };
    };
    let decisive: Vec<Rule> = matched
        .into_iter()
        .filter(|rule| rule.effect == worst)
        .cloned()
        .collect();
    let reason = decisive
        .iter()
        .map(|rule| rule.reason)
        .collect::<Vec<_>>()
        .join(" ");
    // An external caller's confirm waits for the user (ADR-032), unless the policy lets
    // unattended callers through.
    if worst == Effect::Confirm
        && request.caller == Caller::External
        && policy.unattended == Effect::Allow
    {
        return Decision {
            effect: Effect::Allow,
            matched: decisive,
            reason: String::new(),
        };
    }
    Decision {
        effect: worst,
        matched: decisive,
        reason,
    }
}

/// Rule ids behind a decision, for the log.
pub fn describe(decision: &Decision) -> String {
    decision
        .matched
        .iter()
        .map(|rule| rule.id)
        .collect::<Vec<_>>()
        .join(", ")
}

/// One line for the approval prompt.
pub fn summary(decision: &Decision) -> String {
    decision
        .matched
        .iter()
        .map(|rule| rule.title)
        .collect::<Vec<_>>()
        .join(" · ")
}

pub const DECLINED_MESSAGE: &str = "The user declined this action. Do not retry it and do not try to achieve the same effect another way. Tell the user it was declined and stop.";
pub const UNANSWERED_MESSAGE: &str = "Nobody answered the approval in time, so it was not done. Do not retry it and do not try another way. Say what you were about to do and that it is waiting on the user, then stop.";

/// `blocked(decision)`'s message (code `BLOCKED`).
pub fn blocked_message(decision: &Decision) -> String {
    format!(
        "{} Tell the user what you were about to do and why it was blocked, then stop.",
        decision.reason
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn request<'a>(
        action: &'a str,
        input: &'a Value,
        caller: Caller,
        scope: &'a Scope,
    ) -> GuardrailRequest<'a> {
        GuardrailRequest {
            action,
            input,
            caller,
            scope,
        }
    }

    #[test]
    fn decisions() {
        let policy = policy_from(&GuardrailConfig::default(), &[SUBMIT_ACTION.into()]);
        let anywhere = scope::anywhere();
        let input = json!({});
        let d = decide(
            &request("vexdom.x", &input, Caller::Agent, &anywhere),
            &policy,
        );
        assert_eq!(
            (d.effect, describe(&d).as_str()),
            (Effect::Deny, "reserved-action")
        );
        let d = decide(
            &request(SUBMIT_ACTION, &input, Caller::Agent, &anywhere),
            &policy,
        );
        assert_eq!(
            (d.effect, summary(&d).as_str()),
            (Effect::Confirm, "Submits a form")
        );
        let d = decide(
            &request(SUBMIT_ACTION, &input, Caller::External, &anywhere),
            &policy,
        );
        assert_eq!(d.effect, Effect::Confirm);
        let js = json!({"url": "javascript:alert(1)"});
        assert_eq!(
            decide(
                &request("page.navigate", &js, Caller::Agent, &anywhere),
                &policy
            )
            .effect,
            Effect::Deny
        );
        let scoped = Scope {
            hosts: vec!["example.com".into()],
            ..Scope::default()
        };
        let off = json!({"url": "https://evil.com/"});
        assert_eq!(
            decide(
                &request("page.navigate", &off, Caller::Agent, &scoped),
                &policy
            )
            .effect,
            Effect::Confirm
        );
        let html = json!({"format": "html"});
        assert_eq!(
            decide(
                &request(EXTRACT_ACTION, &html, Caller::Agent, &anywhere),
                &policy
            )
            .effect,
            Effect::Deny
        );
        let unattended = policy_from(
            &serde_json::from_value(
                json!({"unattended": "allow", "rules": {"raw-html-read": "allow"}}),
            )
            .unwrap(),
            &[],
        );
        assert_eq!(
            decide(
                &request(SUBMIT_ACTION, &input, Caller::External, &anywhere),
                &unattended
            )
            .effect,
            Effect::Allow
        );
        assert_eq!(
            decide(
                &request(EXTRACT_ACTION, &html, Caller::Agent, &anywhere),
                &unattended
            )
            .effect,
            Effect::Allow
        );
    }

    #[test]
    fn submits() {
        assert!(submits_form("page.fillInput", &json!({"pressEnter": true})));
        assert!(submits_form("page.pressKey", &json!({"key": "Enter"})));
        assert!(!submits_form("page.pressKey", &json!({"key": "a"})));
    }
}
