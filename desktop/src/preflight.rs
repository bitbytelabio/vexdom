//! The first-run checks: what has to be in place before the window is of any use (the `vexdom`
//! command, and browsers able to wake the Bridge), and what only helps (a browser, Claude Code).
//! `bridge.rs` gathers the [`Findings`]; this decides what they mean. Kept apart from GPUI so it is
//! tested without a window.

use std::path::PathBuf;

use serde_json::Value;

use crate::model::agent_card;

/// The doctor check that says browsers can start the Bridge.
const NATIVE_HOST_CHECK: &str = "native_host.registered";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckId {
    /// A `vexdom` binary: beside the app, else on the login PATH.
    Bridge,
    /// The browsers' native-host registration (`vexdom doctor`).
    BridgeSetup,
    /// Any supported browser installed.
    Browser,
    /// Claude Code installed.
    Agent,
}

impl CheckId {
    pub const ORDER: [Self; 4] = [Self::Bridge, Self::BridgeSetup, Self::Browser, Self::Agent];

    /// The window waits for these; the others only advise.
    pub fn required(self) -> bool {
        matches!(self, Self::Bridge | Self::BridgeSetup)
    }

    pub fn title(self) -> &'static str {
        match self {
            Self::Bridge => "The vexdom command",
            Self::BridgeSetup => "Browsers can wake the Bridge",
            Self::Browser => "A browser",
            Self::Agent => "Claude Code",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Checking,
    Pass(String),
    Fail(String),
    /// Not looked at, and why.
    Skipped(String),
}

/// What a check's button does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fix {
    /// `vexdom setup --no-pair --no-open --no-wait --json`.
    SetUp,
    /// Opens Chrome's download page.
    GetChrome,
    /// `npm i -g @anthropic-ai/claude-code`; `npm` says whether npm is on the login PATH.
    InstallClaude { npm: bool },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Check {
    pub id: CheckId,
    pub outcome: Outcome,
    pub fix: Option<Fix>,
}

/// Every check, in [`CheckId::ORDER`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Preflight {
    pub checks: Vec<Check>,
}

impl Preflight {
    pub fn checking() -> Self {
        Self {
            checks: CheckId::ORDER
                .into_iter()
                .map(|id| Check {
                    id,
                    outcome: Outcome::Checking,
                    fix: None,
                })
                .collect(),
        }
    }

    pub fn running(&self) -> bool {
        self.checks
            .iter()
            .any(|check| check.outcome == Outcome::Checking)
    }

    /// Every required check passed, so the window can show its tabs.
    pub fn ready(&self) -> bool {
        self.checks
            .iter()
            .filter(|check| check.id.required())
            .all(|check| matches!(check.outcome, Outcome::Pass(_)))
    }

    /// What "Set up everything" runs, in order: the fixes of the required checks that failed.
    pub fn required_fixes(&self) -> Vec<Fix> {
        self.checks
            .iter()
            .filter(|check| check.id.required() && matches!(check.outcome, Outcome::Fail(_)))
            .filter_map(|check| check.fix)
            .collect()
    }
}

/// What `bridge.rs` found out.
#[derive(Debug, Default)]
pub struct Findings {
    /// The `vexdom` the app runs.
    pub vexdom: Option<PathBuf>,
    /// `vexdom doctor --json --offline`.
    pub doctor: Option<Result<Value, String>>,
    /// `vexdom browsers --json`.
    pub browsers: Option<Result<Value, String>>,
    /// `vexdom agent --json`; `None` while the Bridge is off, since asking would start it.
    pub agent: Option<Result<Value, String>>,
    pub npm: bool,
}

const NEEDS_VEXDOM: &str = "Needs the vexdom command first.";

pub fn evaluate(found: &Findings) -> Preflight {
    let check = |id, outcome, fix| Check { id, outcome, fix };
    let bridge = match &found.vexdom {
        Some(path) => Outcome::Pass(path.display().to_string()),
        None => Outcome::Fail(
            "There is no vexdom beside this app or on your PATH. Reinstall Vexdom.".to_owned(),
        ),
    };
    let (setup, setup_fix) = match &found.doctor {
        None => (Outcome::Fail(NEEDS_VEXDOM.to_owned()), None),
        Some(Err(error)) => (Outcome::Fail(error.clone()), Some(Fix::SetUp)),
        Some(Ok(report)) => wake_up(report),
    };
    let (browser, browser_fix) = match &found.browsers {
        None => (Outcome::Skipped(NEEDS_VEXDOM.to_owned()), None),
        Some(Err(error)) => (Outcome::Fail(error.clone()), Some(Fix::GetChrome)),
        Some(Ok(list)) => installed_browsers(list),
    };
    let install = Fix::InstallClaude { npm: found.npm };
    let (agent, agent_fix) = match &found.agent {
        None => (
            Outcome::Skipped("Start Vexdom to look for Claude Code.".to_owned()),
            None,
        ),
        Some(Err(error)) => (Outcome::Fail(error.clone()), None),
        Some(Ok(state)) => match agent_card(state) {
            Some(card) if card.problem != "AGENT_MISSING" => (Outcome::Pass(card.detail), None),
            Some(card) => (Outcome::Fail(card.detail), Some(install)),
            None => (
                Outcome::Fail("Claude Code is not installed.".to_owned()),
                Some(install),
            ),
        },
    };
    Preflight {
        checks: vec![
            check(CheckId::Bridge, bridge, None),
            check(CheckId::BridgeSetup, setup, setup_fix),
            check(CheckId::Browser, browser, browser_fix),
            check(CheckId::Agent, agent, agent_fix),
        ],
    }
}

/// The doctor's `native_host.registered` check.
fn wake_up(report: &Value) -> (Outcome, Option<Fix>) {
    let found = report
        .get("checks")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .find(|check| check.get("id").and_then(Value::as_str) == Some(NATIVE_HOST_CHECK));
    let Some(found) = found else {
        return (
            Outcome::Fail("vexdom doctor did not report the browsers' wake-up.".to_owned()),
            Some(Fix::SetUp),
        );
    };
    let message = found
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned();
    if found.get("status").and_then(Value::as_str) == Some("pass") {
        (Outcome::Pass(message), None)
    } else {
        (Outcome::Fail(message), Some(Fix::SetUp))
    }
}

fn installed_browsers(list: &Value) -> (Outcome, Option<Fix>) {
    let installed: Vec<&str> = list
        .get("browsers")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|row| row.get("installed") == Some(&Value::Bool(true)))
        .filter_map(|row| row.get("label").and_then(Value::as_str))
        .collect();
    if installed.is_empty() {
        (
            Outcome::Fail(
                "No supported browser found. Chrome, Edge, Brave, Arc, Vivaldi, Opera and Firefox all work."
                    .to_owned(),
            ),
            Some(Fix::GetChrome),
        )
    } else {
        (Outcome::Pass(installed.join(", ")), None)
    }
}

#[cfg(test)]
mod tests {
    use std::assert_matches;

    use serde_json::json;

    use super::*;

    fn doctor(status: &str) -> Value {
        json!({"success": true, "checks": [
            {"id": "env.version", "status": "pass", "message": "vexdom 0.9.2"},
            {"id": "native_host.registered", "status": status, "message": "Browsers can start the Bridge"},
        ]})
    }

    fn found(native_host: &str, browsers: Value, agent: Option<Value>) -> Findings {
        Findings {
            vexdom: Some(PathBuf::from(
                "/Applications/Vexdom.app/Contents/MacOS/vexdom",
            )),
            doctor: Some(Ok(doctor(native_host))),
            browsers: Some(Ok(browsers)),
            agent: agent.map(Ok),
            npm: false,
        }
    }

    fn ids(preflight: &Preflight) -> Vec<CheckId> {
        preflight.checks.iter().map(|check| check.id).collect()
    }

    #[test]
    fn checks_come_in_order_and_two_are_required() {
        let checking = Preflight::checking();
        assert_eq!(ids(&checking), CheckId::ORDER);
        assert!(checking.running() && !checking.ready());
        let required: Vec<CheckId> = CheckId::ORDER
            .into_iter()
            .filter(|id| id.required())
            .collect();
        assert_eq!(required, [CheckId::Bridge, CheckId::BridgeSetup]);
        let evaluated = evaluate(&Findings::default());
        assert_eq!(ids(&evaluated), CheckId::ORDER);
        assert!(!evaluated.running());
    }

    #[test]
    fn advisory_checks_never_hold_the_window() {
        let ready = evaluate(&found(
            "pass",
            json!({"browsers": [{"id": "edge", "label": "Edge", "installed": false}]}),
            Some(
                json!({"runners": [{"kind": "claude", "bin": "claude", "ready": false,
                "problem": {"code": "AGENT_MISSING", "message": "Claude Code is not installed"}}]}),
            ),
        ));
        assert!(ready.ready());
        assert!(ready.required_fixes().is_empty());
        assert_eq!(ready.checks[2].fix, Some(Fix::GetChrome));
        assert_eq!(
            ready.checks[3],
            Check {
                id: CheckId::Agent,
                outcome: Outcome::Fail("Claude Code is not installed".to_owned()),
                fix: Some(Fix::InstallClaude { npm: false }),
            }
        );
        let off = evaluate(&found("pass", json!({"browsers": []}), None));
        assert_matches!(off.checks[3].outcome, Outcome::Skipped(_));
        assert!(off.ready());
    }

    #[test]
    fn a_missing_wake_up_holds_the_window_and_set_up_fixes_it() {
        let broken = evaluate(&found(
            "warn",
            json!({"browsers": [{"id": "chrome", "label": "Chrome", "installed": true}]}),
            Some(
                json!({"runners": [{"kind": "claude", "ready": true, "version": "2.1.3 (Claude Code)"}]}),
            ),
        ));
        assert!(!broken.ready());
        assert_eq!(broken.required_fixes(), [Fix::SetUp]);
        assert_eq!(broken.checks[2].outcome, Outcome::Pass("Chrome".to_owned()));
        assert_eq!(
            broken.checks[3].outcome,
            Outcome::Pass("2.1.3 (Claude Code)".to_owned())
        );
    }

    #[test]
    fn without_vexdom_nothing_can_be_set_up() {
        let none = evaluate(&Findings::default());
        assert!(!none.ready());
        assert_matches!(none.checks[0].outcome, Outcome::Fail(_));
        assert_eq!(none.checks[1].fix, None);
        assert!(none.required_fixes().is_empty());
    }
}
