//! A recording turned into replayable steps, checked before it reaches the panel
//! (`src/lib/recordings/workflow.ts`).

use serde::Serialize;
use serde_json::Value;
use url::Url;

use super::scrub::{clip, looks_like_instruction, looks_like_selector, scrub};
use super::text::js_trim;

const MAX_STEPS: usize = 80;
const MAX_VARIABLES: usize = 20;
const MAX_CAVEATS: usize = 8;
const MAX_WORKFLOW_BYTES: usize = 12 * 1024;

struct StepLimits {
    goal: usize,
    summary: usize,
    intent: usize,
    note: usize,
    value: usize,
    text: usize,
    variable: usize,
    field: usize,
    kind: usize,
    caveat: usize,
}

const STEP_LIMITS: StepLimits = StepLimits {
    goal: 160,
    summary: 400,
    intent: 120,
    note: 160,
    value: 200,
    text: 80,
    variable: 40,
    field: 80,
    kind: 24,
    caveat: 200,
};

pub const REPLAYABLE_ACTIONS: [&str; 13] = [
    "page.navigate",
    "page.clickElement",
    "page.fillInput",
    "page.selectOption",
    "page.pressKey",
    "page.submitForm",
    "page.scrollTo",
    "page.hoverElement",
    "page.focusInput",
    "page.selectText",
    "page.waitForElement",
    "page.extractText",
    "page.getPageInfo",
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StepTarget {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub selector: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RecordingStep {
    pub ordinal: usize,
    pub intent: String,
    pub action: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<StepTarget>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RecordingVariable {
    pub name: String,
    pub field: String,
    pub kind: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RecordingWorkflow {
    pub goal: String,
    pub summary: String,
    pub steps: Vec<RecordingStep>,
    pub variables: Vec<RecordingVariable>,
    pub caveats: Vec<String>,
}

pub struct CheckedWorkflow {
    pub workflow: RecordingWorkflow,
    pub warnings: Vec<String>,
}

fn items(value: Option<&Value>) -> &[Value] {
    value.and_then(Value::as_array).map_or(&[], Vec::as_slice)
}

fn optional(text: String) -> Option<String> {
    (!text.is_empty()).then_some(text)
}

/// `validateRecordingWorkflow`: `origin` is where the recording started.
pub fn validate_recording_workflow(input: &Value, origin: &str) -> Result<CheckedWorkflow, String> {
    if !input.is_object() {
        return Err("The workflow must be an object.".into());
    }
    let mut warnings = Vec::new();

    let goal = scrub(input.get("goal"), STEP_LIMITS.goal);
    if goal.is_empty() {
        return Err("The workflow needs a one-line goal.".into());
    }
    let summary = scrub(input.get("summary"), STEP_LIMITS.summary);

    let mut steps: Vec<RecordingStep> = Vec::new();
    for candidate in items(input.get("steps")).iter().take(MAX_STEPS) {
        if let Some(mut step) = normalize_step(candidate, origin, &mut warnings) {
            step.ordinal = steps.len() + 1;
            steps.push(step);
        }
    }
    if steps.is_empty() {
        return Err("The workflow lists no replayable steps.".into());
    }

    let mut variables: Vec<RecordingVariable> = Vec::new();
    for candidate in items(input.get("variables")).iter().take(MAX_VARIABLES) {
        let name: String = scrub(candidate.get("name"), STEP_LIMITS.variable)
            .chars()
            .filter(|c| !matches!(c, '{' | '}'))
            .collect();
        if name.is_empty() || variables.iter().any(|known| known.name == name) {
            continue;
        }
        let field = optional(scrub(candidate.get("field"), STEP_LIMITS.field))
            .unwrap_or_else(|| name.clone());
        let kind = optional(scrub(candidate.get("kind"), STEP_LIMITS.kind))
            .unwrap_or_else(|| "text".into());
        variables.push(RecordingVariable { name, field, kind });
    }

    let caveats = items(input.get("caveats"))
        .iter()
        .take(MAX_CAVEATS)
        .map(|caveat| scrub(Some(caveat), STEP_LIMITS.caveat))
        .filter(|caveat| !caveat.is_empty())
        .collect();

    let mut workflow = RecordingWorkflow {
        goal,
        summary,
        steps,
        variables,
        caveats,
    };
    trim_to_budget(&mut workflow, &mut warnings);

    for text in prompt_strings(&workflow) {
        if looks_like_instruction(text) {
            warnings.push(format!(
                "Reads like an instruction rather than an observation: “{}”",
                clip(text, 80)
            ));
        }
    }
    Ok(CheckedWorkflow { workflow, warnings })
}

fn normalize_step(
    candidate: &Value,
    origin: &str,
    warnings: &mut Vec<String>,
) -> Option<RecordingStep> {
    let action = candidate
        .get("action")
        .and_then(Value::as_str)
        .map(js_trim)
        .unwrap_or("");
    if !REPLAYABLE_ACTIONS.contains(&action) {
        if !action.is_empty() {
            warnings.push(format!(
                "Dropped a step naming an action that cannot be replayed: {}",
                clip(action, 40)
            ));
        }
        return None;
    }
    let intent = scrub(candidate.get("intent"), STEP_LIMITS.intent);
    if intent.is_empty() {
        return None;
    }
    let target = normalize_target(candidate.get("target"), warnings);
    let url = same_origin(candidate.get("url"), origin);
    if action == "page.navigate" && url.is_empty() {
        return None;
    }
    if action != "page.navigate" && action != "page.getPageInfo" && target.is_none() {
        return None;
    }
    Some(RecordingStep {
        ordinal: 0,
        intent,
        action: action.to_owned(),
        target,
        value: optional(scrub(candidate.get("value"), STEP_LIMITS.value)),
        url: optional(url),
        note: optional(scrub(candidate.get("note"), STEP_LIMITS.note)),
    })
}

fn normalize_target(target: Option<&Value>, warnings: &mut Vec<String>) -> Option<StepTarget> {
    let target = target.filter(|target| target.is_object())?;
    let mut selector = target
        .get("selector")
        .and_then(Value::as_str)
        .map(|selector| js_trim(selector).to_owned())
        .unwrap_or_default();
    if !selector.is_empty() && !looks_like_selector(&selector) {
        warnings.push(format!(
            "Dropped a step selector that did not look like a selector: {}",
            clip(&selector, 40)
        ));
        selector.clear();
    }
    let text = scrub(target.get("text"), STEP_LIMITS.text);
    let role = scrub(target.get("role"), STEP_LIMITS.kind);
    if selector.is_empty() && text.is_empty() {
        return None;
    }
    Some(StepTarget {
        selector: optional(selector),
        text: optional(text),
        role: optional(role),
    })
}

fn same_origin(value: Option<&Value>, origin: &str) -> String {
    let Some(text) = value.and_then(Value::as_str).map(js_trim) else {
        return String::new();
    };
    if text.is_empty() {
        return String::new();
    }
    let Ok(base) = Url::parse(origin) else {
        return String::new();
    };
    let Ok(url) = base.join(text) else {
        return String::new();
    };
    if url.origin() != base.origin() || !matches!(url.scheme(), "http" | "https") {
        return String::new();
    }
    clip(url.as_str(), 300)
}

fn prompt_strings(workflow: &RecordingWorkflow) -> Vec<&str> {
    let mut out: Vec<&str> = vec![&workflow.goal, &workflow.summary];
    for step in &workflow.steps {
        out.push(&step.intent);
        out.push(step.note.as_deref().unwrap_or(""));
        out.push(
            step.target
                .as_ref()
                .and_then(|target| target.text.as_deref())
                .unwrap_or(""),
        );
    }
    out.extend(
        workflow
            .variables
            .iter()
            .map(|variable| variable.field.as_str()),
    );
    out.extend(workflow.caveats.iter().map(String::as_str));
    out.retain(|text| !text.is_empty());
    out
}

fn prompt_bytes(workflow: &RecordingWorkflow) -> usize {
    prompt_strings(workflow).iter().map(|text| text.len()).sum()
}

fn trim_to_budget(workflow: &mut RecordingWorkflow, warnings: &mut Vec<String>) {
    if prompt_bytes(workflow) <= MAX_WORKFLOW_BYTES {
        return;
    }
    let shed = |workflow: &mut RecordingWorkflow,
                warnings: &mut Vec<String>,
                label: &str,
                cut: &dyn Fn(&mut RecordingWorkflow)| {
        if prompt_bytes(workflow) <= MAX_WORKFLOW_BYTES {
            return;
        }
        cut(workflow);
        warnings.push(format!(
            "The workflow was over its size budget, so {label} was left out."
        ));
    };
    shed(workflow, warnings, "the per-step notes", &|workflow| {
        for step in &mut workflow.steps {
            step.note = None;
        }
    });
    shed(workflow, warnings, "some of the caveats", &|workflow| {
        workflow.caveats.truncate(3);
    });
    shed(
        workflow,
        warnings,
        "the longer step descriptions",
        &|workflow| {
            for step in &mut workflow.steps {
                step.intent = clip(&step.intent, 60);
            }
        },
    );
    while prompt_bytes(workflow) > MAX_WORKFLOW_BYTES && workflow.steps.len() > 1 {
        workflow.steps.pop();
    }
    if workflow.steps.len() == 1 {
        warnings.push("Only the first step fitted in the workflow.".into());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn steps_that_cannot_be_replayed_on_the_site_are_dropped() {
        let checked = validate_recording_workflow(
            &json!({
                "goal": "Check out", "summary": "To the payment page.",
                "steps": [
                    {"intent": "Start", "action": "page.clickElement", "target": {"text": "Checkout", "selector": "button#checkout"}},
                    {"intent": "Email", "action": "page.fillInput", "target": {"selector": "input#email"}, "value": "{{email}}"},
                    {"intent": "Leave", "action": "page.navigate", "url": "https://tracker.example/pixel"},
                    {"intent": "Go", "action": "page.navigate", "url": "/cart"},
                    {"intent": "Run", "action": "page.runScript", "target": {"selector": "body"}},
                    {"intent": "Read", "action": "page.getPageInfo"}
                ],
                "variables": [{"name": "{{email}}", "field": "Email address", "kind": "email"}, {"name": "email"}],
                "caveats": ["The basket changes."]
            }),
            "https://shop.example",
        )
        .unwrap();
        let steps: Vec<(usize, &str)> = checked
            .workflow
            .steps
            .iter()
            .map(|step| (step.ordinal, step.action.as_str()))
            .collect();
        assert_eq!(
            steps,
            [
                (1, "page.clickElement"),
                (2, "page.fillInput"),
                (3, "page.navigate"),
                (4, "page.getPageInfo")
            ]
        );
        assert_eq!(
            checked.workflow.steps[2].url.as_deref(),
            Some("https://shop.example/cart")
        );
        assert_eq!(
            serde_json::to_value(&checked.workflow.variables).unwrap(),
            json!([{"name": "email", "field": "Email address", "kind": "email"}])
        );
        assert_eq!(
            checked.warnings,
            ["Dropped a step naming an action that cannot be replayed: page.runScript"]
        );
    }

    #[test]
    fn a_workflow_needs_a_goal_and_a_step() {
        assert_eq!(
            validate_recording_workflow(&json!({"goal": ""}), "https://a.example").err(),
            Some("The workflow needs a one-line goal.".into())
        );
        assert_eq!(
            validate_recording_workflow(&json!({"goal": "x", "steps": []}), "https://a.example")
                .err(),
            Some("The workflow lists no replayable steps.".into())
        );
    }
}
