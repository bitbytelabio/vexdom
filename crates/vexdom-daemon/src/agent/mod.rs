//! Side-panel agent runs (`src/daemon/agent/`). The daemon core calls exactly the items in this
//! file.

mod abort;
mod agent_skills;
mod attachments;
mod captcha_analyst;
mod captcha_solver;
mod file_analyst;
mod file_report;
mod file_screen;
mod mapping;
mod prompt;
mod recording;
mod runner;
mod runners;
mod service;
mod site_map;
mod site_map_store;
mod sitemap;
#[cfg(test)]
mod test_support;
mod title;

use std::rc::Rc;

use futures::future::LocalBoxFuture;
use serde_json::Value;
use vexdom_core::config::AgentConfig;

pub use captcha_solver::solve_captcha_with_analyst;
pub use file_analyst::{FileAnalyses, FileRequest};
pub use recording::analyze_recording;
pub use service::AgentSession;
pub use site_map_store::{commit_staging, discard_staging};
pub use title::name_session;

use crate::link::Reply;
use crate::wire::RunEvent;

/// Where a mapping run's screenshot is written instead of the screenshot folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SaveTo {
    pub dir: String,
    pub filename: String,
}

/// How a run's page call is routed and post-processed by the daemon (`invokeOn`'s options).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InvokeOpts {
    pub save_to: Option<SaveTo>,
    pub tab_id: Option<i64>,
    pub run_id: Option<String>,
    /// The run's scope hosts, for downloads.
    pub hosts: Option<Vec<String>>,
    /// An external caller's approver tag (ADR-032), for approvals a Bridge action asks for.
    pub approver: Option<String>,
}

/// A page call through the daemon (`invokeOn` bound to one browser).
pub type InvokeFn = Rc<dyn Fn(String, Value, InvokeOpts) -> LocalBoxFuture<'static, Reply>>;
/// A `run` push to one browser's side panel.
pub type EmitFn = Rc<dyn Fn(&str, RunEvent)>;
/// A `siteMapDraft` push.
pub type DraftFn = Rc<dyn Fn(&str, &Value)>;

/// What an `AgentSession` needs from the daemon, for the one browser it belongs to.
#[derive(Clone)]
pub struct AgentDeps {
    /// `invokeOn(link, …)`: a page action in this browser, with the daemon's post-processing.
    pub invoke: InvokeFn,
    /// `run` push to this browser's side panel.
    pub emit: EmitFn,
    /// `siteMapDraft` push.
    pub draft: DraftFn,
    /// Runs going across every browser, for `maxConcurrentRuns`.
    pub running: Rc<dyn Fn() -> usize>,
    pub analyses: Rc<FileAnalyses>,
}

/// The extension → daemon frames an agent session handles.
#[derive(Debug, Clone, PartialEq)]
pub enum AgentRequest {
    Instruct {
        id: String,
        text: String,
        context: Value,
    },
    Cancel {
        id: String,
    },
    Decision {
        id: String,
        tool_id: String,
        allow: bool,
        remember: bool,
    },
    Reset {
        session_id: Option<String>,
    },
}

/// What a run is offered beyond its browser's tool list (`describeFor` with a run id).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Offer {
    pub withheld: Vec<String>,
    pub reserved: Vec<String>,
}

/// `agentState(config, {refresh})`: `AgentState` JSON (`src/lib/agents/catalog.ts`), each agent
/// probed with its version flag; a probe is reused for half a minute unless `refresh`.
pub async fn agent_state(config: &AgentConfig, refresh: bool) -> Value {
    runners::agent_state(config, refresh).await
}

/// The agent CLI's own skills (`agentSkills`), as the catalog lists them: `AgentSkillMeta[]`.
pub fn agent_skills(config: &AgentConfig, refresh: bool) -> Value {
    serde_json::to_value(agent_skills::agent_skills(config, refresh))
        .unwrap_or_else(|_| Value::Array(Vec::new()))
}
