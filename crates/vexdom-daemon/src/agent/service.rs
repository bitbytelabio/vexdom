//! One browser's side-panel runs (`service.ts`): an instruction admitted and its agent started, and
//! every tool call that agent makes, through the run's scope, guardrails and approvals.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::rc::Rc;
use std::sync::LazyLock;
use std::time::Duration;

use futures::future::LocalBoxFuture;
use regex::Regex;
use serde_json::{Value, json};
use tokio::sync::oneshot;
use vexdom_core::agents::agent_label;
use vexdom_core::approvals::{is_granted, remember_grant};
use vexdom_core::config::{
    AgentConfig, SiteMapSettings, max_concurrent_runs, read_agent_config, site_map_settings,
};
use vexdom_core::downloads::uuid_v4;
use vexdom_core::guardrails::scope::{self, Scope, ScopeSeed, SharedTab};
use vexdom_core::guardrails::secrets::seal_secrets;
use vexdom_core::guardrails::{
    self, CAPTCHA_ACTION, Caller, DECLINED_MESSAGE, Effect, GuardrailRequest, INJECT_ACTION,
    Policy, RUN_CODE_ACTION, UNANSWERED_MESSAGE,
};
use vexdom_core::log;
use vexdom_core::profile::read_profile;
use vexdom_core::result::{ActionResult, failure};
use vexdom_core::skills::{self, Skill};
use vexdom_core::tools::{FOCUS_SHOT_ACTION, READ_SITEMAP_ACTION, SAVE_SITE_MAP_ACTION};
use vexdom_pure::schedules::{TASK_APPROVAL_WAIT_MS, TASK_RUN_MAX_MS};

use super::abort::{Abort, RunError, abort_after, run_error};
use super::agent_skills::{AgentSkill, resolve_agent_skill};
use super::attachments::{AttachedFile, Handover, HandoverDeps, hand_over};
use super::file_report::FileReport;
use super::mapping::{Gate, MapRun, gate_mapping_invoke, note_mapping_result};
use super::prompt::{
    BuiltPrompt, ConversationTab, PromptExtras, PromptSection, TaskContext, build_system_prompt,
    profile_block, prompt_update, scheduled_block, tabs_block, turn_message,
};
use super::runner::{RunRequest, run_instruction};
use super::runners::{agent_state, runner_of};
use super::site_map::{SITE_MAPPER_SKILL, validate_site_map_report};
use super::site_map_store::{
    StageArgs, map_target_for, prepare_staging, stage_site_map, staged_screenshots, sweep_staging,
    write_evidence,
};
use super::sitemap::{SiteIndex, fetch_site_index};
use super::{AgentDeps, AgentRequest, EmitFn, InvokeOpts, Offer, discard_staging};
use crate::link::Reply;
use crate::wire::RunEvent;
use vexdom_pure::text::{
    flatten, js_text, js_trim, safe_integer, str_field, utf16_len, utf16_prefix,
};

/// Each reports the tab the browser shows afterwards, which is where the run's next action lands.
/// The Bridge action that stores a password: the user's alone (`vexdom auth save`).
const AUTH_SAVE_ACTION: &str = "bridge.authSave";
const TAB_ACTIONS: [&str; 3] = ["page.openTab", "page.switchTab", "page.closeTab"];
const OPEN_TAB_ACTION: &str = "page.openTab";

const MAX_FOCUS_CONTENT: usize = 6 * 1024;
const MAX_RECORDINGS_BLOCK: usize = 4 * 1024;
const MAX_ROSTER: usize = 20;
const MAX_SITE_MAP_REPORT: usize = 256 * 1024;
const SUMMARY_LIMIT: usize = 80;

const LIVE_TOOLS_OFF: &str = "Writing and running your own page code is switched off for this message. The user turns it on with the “Live tool” switch beside the message box, and only they can. Do the job with the ordinary page tools, or say that this one needs that switch and why.";

const MAPPING_NOT_REQUESTED: &str = "Mapping a site takes several minutes and drives this tab. Ask for it explicitly with the Map this site button, or by starting your message with @site-mapper.";

static TAB_HOST: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(?:[A-Za-z0-9-]+(?:\.[A-Za-z0-9-]+)*|\[[0-9A-Fa-f:.]+\])(?::[0-9]{1,5})?$")
        .expect("the tab host pattern compiles")
});

/// The user's answer to an approval.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Decision {
    allow: bool,
    remember: bool,
    /// A scheduled run's approval that nobody answered in time.
    unanswered: bool,
}

struct ActiveRun {
    id: String,
    /// The tab-bound conversation this run belongs to; one run at a time per session.
    session_id: Option<String>,
    config: AgentConfig,
    policy: Policy,
    scope: Scope,
    abort: Abort,
    /// Approvals waiting on the user, by tool id.
    pending: RefCell<HashMap<String, oneshot::Sender<Decision>>>,
    /// Tabs this run opened itself, which its own scope may therefore reach.
    owned_tab_ids: RefCell<Vec<i64>>,
    /// The tab a tab action left it on (`Some(None)` when the browser chose one); `None` until it
    /// first moves.
    current_tab_id: Cell<Option<Option<i64>>>,
    /// Host of the tab this run started on: what "this site" means on an approval card.
    site: Option<String>,
    map: RefCell<Option<MapRun>>,
    /// The A-Eye pick photographed when the user attached it to this instruction.
    focus_shot: Option<String>,
    /// The composer's “Live tool” switch, as it stood when this instruction was sent.
    live_tools: bool,
    /// Actions the user allowed once in this run that stay allowed for the rest of it.
    approved: RefCell<HashSet<String>>,
    /// A schedule started it: an approval nobody answers is declined rather than awaited forever.
    scheduled: bool,
}

impl ActiveRun {
    /// How long an approval waits for its answer; an unscheduled run waits as long as the user
    /// takes.
    fn approval_wait(&self) -> Option<Duration> {
        self.scheduled
            .then(|| Duration::from_millis(TASK_APPROVAL_WAIT_MS.unsigned_abs()))
    }

    /// The user's answer to the approval `tool_id` asks for.
    async fn decision(&self, tool_id: &str) -> Decision {
        if self.abort.aborted() {
            return Decision::default();
        }
        let (settle, answer) = oneshot::channel();
        self.pending.borrow_mut().insert(tool_id.to_owned(), settle);
        let expiry = async {
            match self.approval_wait() {
                Some(wait) => tokio::time::sleep(wait).await,
                None => std::future::pending().await,
            }
        };
        let answer = tokio::select! {
            answer = answer => answer.unwrap_or_default(),
            () = self.abort.wait() => Decision::default(),
            () = expiry => Decision { unanswered: true, ..Decision::default() },
        };
        self.pending.borrow_mut().remove(tool_id);
        answer
    }

    /// The scope a call is decided in: the run's, with the tabs it has opened and moved to.
    fn scope_now(&self) -> Scope {
        Scope {
            owned_tab_ids: self.owned_tab_ids.borrow().clone(),
            current_tab_id: self.current_tab_id.get(),
            ..self.scope.clone()
        }
    }
}

/// The conversation an agent is holding open for a panel session, and the prompt that session was
/// last brought up to date with.
#[derive(Debug, Clone)]
struct Held {
    agent: String,
    session_id: String,
    sections: Vec<PromptSection>,
}

/// An instruction past every check that can refuse it at once, its run claimed.
struct Admitted {
    run: Rc<ActiveRun>,
    base: Skill,
    overlays: Vec<Skill>,
    /// The instruction without its `@skill` prefix.
    text: String,
    mapping: bool,
    attached: Option<AgentSkill>,
    roster: Option<Vec<ConversationTab>>,
    context: Value,
}

/// One tool call of a run, from its `tool` event to its `toolResult`. Dropped before it reports,
/// because the agent stopped waiting for it, it withdraws its approval request, so a late answer
/// does nothing, and closes the call on the panel.
struct Call {
    emit: EmitFn,
    run: Rc<ActiveRun>,
    tool_id: String,
    action: String,
    /// Why it waits on the user's approval, while it does.
    waiting: Option<String>,
    reported: bool,
}

impl Call {
    fn open(emit: &EmitFn, run: &Rc<ActiveRun>, action: &str, input: &Value) -> Self {
        let tool_id = uuid_v4();
        emit(
            &run.id,
            RunEvent::Tool {
                tool_id: tool_id.clone(),
                action: action.to_owned(),
                input: input.clone(),
                source: None,
            },
        );
        Self {
            emit: Rc::clone(emit),
            run: Rc::clone(run),
            tool_id,
            action: action.to_owned(),
            waiting: None,
            reported: false,
        }
    }

    fn report(&mut self, ok: bool, summary: String) {
        self.reported = true;
        (self.emit)(
            &self.run.id,
            RunEvent::ToolResult {
                tool_id: self.tool_id.clone(),
                ok,
                summary,
            },
        );
    }
}

impl Drop for Call {
    fn drop(&mut self) {
        if self.reported {
            return;
        }
        let why = match self.waiting.take() {
            Some(why) => {
                if let Ok(mut pending) = self.run.pending.try_borrow_mut() {
                    pending.remove(&self.tool_id);
                }
                format!(": {why}")
            }
            None => String::new(),
        };
        log::log(&format!(
            "agent run {} stopped waiting on {}{why}",
            self.run.id, self.action
        ));
        self.report(false, "the agent stopped waiting".into());
    }
}

/// What a call's input points at, as its summary names it.
enum Target {
    Text(String),
    Selector(String),
    Unnamed,
}

impl Target {
    fn of(input: &Value) -> Self {
        let target = input.get("target");
        let named = |key: &str| {
            target
                .and_then(|target| str_field(target, key))
                .filter(|name| !name.is_empty())
                .map(str::to_owned)
        };
        match (named("text"), named("selector")) {
            (Some(text), _) => Self::Text(text),
            (None, Some(selector)) => Self::Selector(selector),
            (None, None) => Self::Unnamed,
        }
    }
}

/// One browser's side-panel runs.
pub struct AgentSession {
    deps: AgentDeps,
    /// Per panel session, the conversation its agent is holding open. Switching agents leaves
    /// them unused.
    held: RefCell<HashMap<String, Held>>,
    runs: RefCell<Vec<Rc<ActiveRun>>>,
    /// Conversations that have been shown the page-code tools. The tool list leads the prefix an
    /// agent caches, so it may grow once within a conversation and never shrink: a follow-up
    /// lands while the cache is warm, and a changed list re-bills its history.
    code_listed: RefCell<HashSet<String>>,
}

impl AgentSession {
    pub fn new(deps: AgentDeps) -> Rc<Self> {
        Rc::new(Self {
            deps,
            held: RefCell::default(),
            runs: RefCell::default(),
            code_listed: RefCell::default(),
        })
    }

    /// An instruction is admitted, or refused, before this returns; its run then goes on in the
    /// background.
    pub fn handle(self: &Rc<Self>, request: AgentRequest) {
        match request {
            AgentRequest::Instruct { id, text, context } => {
                if let Some(admitted) = self.admit(&id, &text, context) {
                    tokio::task::spawn_local(Rc::clone(self).start(admitted));
                }
            }
            AgentRequest::Cancel { id } => self.cancel(&id),
            AgentRequest::Decision {
                id,
                tool_id,
                allow,
                remember,
            } => {
                let settle = self
                    .run(&id)
                    .and_then(|run| run.pending.borrow_mut().remove(&tool_id));
                if let Some(settle) = settle {
                    let _ = settle.send(Decision {
                        allow,
                        remember,
                        unanswered: false,
                    });
                }
            }
            AgentRequest::Reset { session_id } => {
                match session_id.as_deref().filter(|id| !id.is_empty()) {
                    Some(id) => {
                        self.held.borrow_mut().remove(id);
                        self.code_listed.borrow_mut().remove(id);
                        log::log(&format!("agent conversation reset for session {id}"));
                    }
                    None => {
                        self.held.borrow_mut().clear();
                        self.code_listed.borrow_mut().clear();
                        log::log("agent conversations reset");
                    }
                }
            }
        }
    }

    pub fn running(&self) -> usize {
        self.runs.borrow().len()
    }

    pub fn owns(&self, run_id: &str) -> bool {
        self.runs.borrow().iter().any(|run| run.id == run_id)
    }

    /// What a run's tool list should hold. A tool the run would only be refused is left out: its
    /// schema is re-sent on every turn, and listing it invites the call.
    pub fn offer_for(&self, run_id: &str) -> Option<Offer> {
        let run = self.run(run_id)?;
        let code_listed = run.live_tools
            || run
                .session_id
                .as_ref()
                .is_some_and(|id| self.code_listed.borrow().contains(id));
        let mut withheld: Vec<String> = if code_listed {
            Vec::new()
        } else {
            vec![INJECT_ACTION.into(), RUN_CODE_ACTION.into()]
        };
        // A password reaches the vault from the user, never through an agent run.
        withheld.push(AUTH_SAVE_ACTION.into());
        // A mapping run never resumes a conversation, so its tool can come and go. A pick comes
        // and goes between the messages of one, so the tool that shows it stays.
        let mut reserved = Vec::new();
        if run.map.borrow().is_some() {
            reserved.push(SAVE_SITE_MAP_ACTION.to_owned());
        }
        reserved.push(FOCUS_SHOT_ACTION.to_owned());
        Some(Offer { withheld, reserved })
    }

    /// The agent's own tool call (MCP `invoke` with its run id). Dropping the future withdraws
    /// the call, so nothing it asked for outlives it.
    pub async fn invoke_for_run(
        self: Rc<Self>,
        run_id: String,
        action: String,
        input: Value,
    ) -> Reply {
        let Some(run) = self.run(&run_id) else {
            return failure("RUN_INACTIVE", "This agent run is no longer active").into();
        };
        // A run's flow writes wait for the user like its other writes.
        let mut input = input;
        if action == "bridge.flowRun"
            && let Some(object) = input.as_object_mut()
        {
            object.shift_remove("allowWrite");
        }
        if action == AUTH_SAVE_ACTION {
            return failure(
                "BLOCKED",
                "Saving a login is for the user (vexdom auth save); an agent run cannot store passwords.",
            )
            .into();
        }
        let target = Target::of(&input);
        let mut call = Call::open(&self.deps.emit, &run, &action, &input);

        if action == SAVE_SITE_MAP_ACTION {
            let Some(result) = self.submit_site_map(&run, &input) else {
                call.report(false, "not a mapping run".into());
                return failure("UNKNOWN_ACTION", format!("Unknown action \"{action}\".")).into();
            };
            let reply = Reply::from(result);
            call.report(reply.result.is_ok(), summarize(&target, &reply));
            return reply;
        }

        if action == FOCUS_SHOT_ACTION {
            let (reply, summary) = match &run.focus_shot {
                Some(shot) => (
                    Reply::from(Ok(json!({"dataUrl": shot}))),
                    "the picked element, as photographed",
                ),
                None => (
                    Reply::from(failure(
                        "NO_FOCUS_SHOT",
                        "No A-Eye pick came with this instruction, so nothing was photographed.",
                    )),
                    "no pick attached",
                ),
            };
            call.report(reply.result.is_ok(), summary.into());
            return reply;
        }

        if !run.live_tools && (action == INJECT_ACTION || action == RUN_CODE_ACTION) {
            call.report(false, "live tools are switched off".into());
            return failure("LIVE_TOOLS_OFF", LIVE_TOOLS_OFF).into();
        }

        if run.map.borrow().is_some() {
            return self.invoke_mapping(&run, call, &action, input).await;
        }

        let decision = {
            let scope = run.scope_now();
            guardrails::decide(
                &GuardrailRequest {
                    action: &action,
                    input: &input,
                    caller: Caller::Agent,
                    scope: &scope,
                },
                &run.policy,
            )
        };
        if decision.effect == Effect::Deny {
            log::log(&format!(
                "agent run {run_id} blocked {action}: {}",
                guardrails::describe(&decision)
            ));
            call.report(
                false,
                format!("blocked: {}", guardrails::summary(&decision)),
            );
            return failure("BLOCKED", guardrails::blocked_message(&decision)).into();
        }
        let granted = run
            .site
            .as_deref()
            .is_some_and(|site| is_granted(&action, site));
        if decision.effect == Effect::Confirm
            && !granted
            && !run.approved.borrow().contains(&action)
        {
            (self.deps.emit)(
                &run_id,
                RunEvent::Approval {
                    tool_id: call.tool_id.clone(),
                    action: action.clone(),
                    input: input.clone(),
                    site: run.site.clone(),
                },
            );
            call.waiting = Some(guardrails::describe(&decision));
            return self.answered(&run, call, &action, input).await;
        }
        self.perform(&run, call, &action, input).await
    }

    /// Acts on the user's answer to the approval `call` waits on.
    async fn answered(&self, run: &ActiveRun, mut call: Call, action: &str, input: Value) -> Reply {
        let answer = run.decision(&call.tool_id).await;
        let why = call.waiting.take().unwrap_or_default();
        if answer.unanswered {
            log::log(&format!(
                "agent run {} got no answer on {action}: {why}",
                run.id
            ));
            call.report(false, "nobody answered in time".into());
            return failure("APPROVAL_TIMEOUT", UNANSWERED_MESSAGE).into();
        }
        if !answer.allow {
            log::log(&format!("agent run {} declined {action}: {why}", run.id));
            call.report(false, "declined by the user".into());
            return failure("DECLINED", DECLINED_MESSAGE).into();
        }
        if answer.remember
            && let Some(site) = &run.site
        {
            remember_grant(action, site, &log::iso_now());
        }
        // Getting past a captcha takes a call per round; one yes covers every round of that run.
        if action == CAPTCHA_ACTION {
            run.approved.borrow_mut().insert(action.to_owned());
        }
        self.perform(run, call, action, input).await
    }

    async fn perform(&self, run: &ActiveRun, mut call: Call, action: &str, input: Value) -> Reply {
        let target = Target::of(&input);
        let opts = InvokeOpts {
            run_id: Some(run.id.clone()),
            hosts: Some(run.scope.hosts.clone()),
            ..InvokeOpts::default()
        };
        let reply = (self.deps.invoke)(action.to_owned(), input, opts).await;
        if let Ok(data) = &reply.result {
            if action == OPEN_TAB_ACTION
                && let Some(opened) = data.get("tabId").and_then(safe_integer)
            {
                run.owned_tab_ids.borrow_mut().push(opened);
            }
            if TAB_ACTIONS.contains(&action) {
                let landed = data.get("activeTabId").and_then(safe_integer);
                run.current_tab_id.set(Some(landed));
            }
        }
        log::log(&format!("agent → {action} {}", outcome_of(&reply.result)));
        call.report(reply.result.is_ok(), summarize(&target, &reply));
        reply
    }

    /// A mapping run's call: only what its gate lets through, on the tab it maps.
    async fn invoke_mapping(
        &self,
        run: &ActiveRun,
        mut call: Call,
        action: &str,
        input: Value,
    ) -> Reply {
        let target = Target::of(&input);
        let (gate, tab_id) = {
            let map = run.map.borrow();
            let map = map.as_ref().expect("only a mapping run's calls are gated");
            (gate_mapping_invoke(map, action, &input), map.tab_id)
        };
        let save_to = match gate {
            Gate::Allow { save_to } => save_to,
            Gate::Deny(error) => {
                log::log(&format!(
                    "mapping run {} refused {action}: {}",
                    run.id, error.code
                ));
                let reply = Reply::from(Err(error));
                call.report(false, summarize(&target, &reply));
                return reply;
            }
        };
        let opts = InvokeOpts {
            save_to,
            tab_id,
            ..InvokeOpts::default()
        };
        let reply = (self.deps.invoke)(action.to_owned(), input, opts).await;
        {
            let mut map = run.map.borrow_mut();
            if let Some(map) = map.as_mut() {
                note_mapping_result(map, action, &reply.result);
            }
        }
        log::log(&format!("agent → {action} {}", outcome_of(&reply.result)));
        call.report(reply.result.is_ok(), summarize(&target, &reply));
        reply
    }

    pub fn dispose(&self) {
        let ids: Vec<String> = self
            .runs
            .borrow()
            .iter()
            .map(|run| run.id.clone())
            .collect();
        for id in ids {
            self.cancel(&id);
        }
        self.held.borrow_mut().clear();
        self.code_listed.borrow_mut().clear();
    }

    fn run(&self, run_id: &str) -> Option<Rc<ActiveRun>> {
        self.runs
            .borrow()
            .iter()
            .find(|run| run.id == run_id)
            .cloned()
    }

    fn emit_error(&self, run_id: &str, code: &str, message: impl Into<String>) {
        (self.deps.emit)(run_id, RunEvent::Error(run_error(code, message)));
    }

    /// `start` up to its first wait: every refusal that needs no agent, then the run claimed, so a
    /// second instruction meets `RUN_IN_PROGRESS` and the run's offer can be read at once.
    fn admit(&self, run_id: &str, instruction: &str, context: Value) -> Option<Admitted> {
        let text = js_trim(instruction);
        if text.is_empty() {
            self.emit_error(run_id, "INVALID_INPUT", "Say what you want done.");
            return None;
        }

        let session_id = str_field(&context, "sessionId")
            .filter(|id| !id.is_empty())
            .map(str::to_owned);
        let clashes = {
            let runs = self.runs.borrow();
            match &session_id {
                Some(id) => runs.iter().any(|run| run.session_id.as_ref() == Some(id)),
                None => !runs.is_empty(),
            }
        };
        if clashes {
            self.emit_error(
                run_id,
                "RUN_IN_PROGRESS",
                "This conversation is still running — cancel it first.",
            );
            return None;
        }

        let url = str_field(&context, "url").filter(|url| !url.is_empty());
        let live_tools = context.get("liveTools") == Some(&Value::Bool(true));
        let Some(routed) = skills::route_skill(&skills::load_skills(), text, url, live_tools)
        else {
            self.emit_error(
                run_id,
                "NO_SKILL",
                format!(
                    "No skills found. Looked in {} — reinstall the package, or add a skill of your own.",
                    skills::skill_dir_names().join(" and ")
                ),
            );
            return None;
        };

        let config = read_agent_config();
        let limit = max_concurrent_runs(&config);
        let running = u64::try_from((self.deps.running)()).unwrap_or(u64::MAX);
        if running >= limit {
            self.emit_error(
                run_id,
                "RUN_LIMIT",
                format!(
                    "{limit} runs are already going — let one finish, or stop it, before starting another."
                ),
            );
            return None;
        }
        let mapping = routed.base.name == SITE_MAPPER_SKILL && routed.base.source == "bundled";
        if mapping && !explicitly_asked(text) {
            self.emit_error(run_id, "MAPPING_NOT_REQUESTED", MAPPING_NOT_REQUESTED);
            return None;
        }

        let mut attached = None;
        if let Some(id) = str_field(&context, "agentSkillId").filter(|id| !id.is_empty()) {
            if mapping {
                log::log(&format!(
                    "agent run {run_id}: ignoring the attached agent skill — mapping runs build their own prompt"
                ));
            } else {
                match resolve_agent_skill(id, &config) {
                    Ok(skill) => attached = Some(skill),
                    Err(error) => {
                        (self.deps.emit)(run_id, RunEvent::Error(error));
                        return None;
                    }
                }
            }
        }

        let policy = guardrails::policy_from(&config.guardrails, &config.require_approval);
        let roster = if mapping { None } else { roster_from(&context) };
        let scope = scope::scope_for(&ScopeSeed {
            url: url.map(str::to_owned),
            tab_id: context.get("tabId").and_then(safe_integer),
            instruction: Some(text.to_owned()),
            extra_hosts: config.guardrails.hosts.clone().unwrap_or_default(),
            pin_tab: true,
            tabs: roster
                .iter()
                .flatten()
                .map(|tab| SharedTab {
                    tab_id: tab.tab_id,
                    host: tab.host.clone(),
                    chosen: tab.chosen,
                })
                .collect(),
        });
        let focus_shot = context
            .get("focus")
            .and_then(|focus| str_field(focus, "shot"))
            .filter(|shot| !shot.is_empty())
            .map(str::to_owned);
        let run = Rc::new(ActiveRun {
            id: run_id.to_owned(),
            session_id,
            policy,
            scope,
            site: url.and_then(site_of),
            abort: Abort::new(),
            pending: RefCell::default(),
            owned_tab_ids: RefCell::default(),
            current_tab_id: Cell::new(None),
            map: RefCell::new(None),
            focus_shot,
            live_tools,
            approved: RefCell::default(),
            scheduled: context.get("task").is_some_and(|task| !task.is_null()),
            config,
        });
        {
            let mut runs = self.runs.borrow_mut();
            runs.retain(|held| held.id != run.id);
            runs.push(Rc::clone(&run));
        }
        if run.live_tools
            && let Some(id) = &run.session_id
        {
            self.code_listed.borrow_mut().insert(id.clone());
        }
        Some(Admitted {
            run,
            base: routed.base,
            overlays: routed.overlays,
            text: routed.text,
            mapping,
            attached,
            roster,
            context,
        })
    }

    /// The rest of `start`: the agent probed, the prompt built, the turn run to its end.
    async fn start(self: Rc<Self>, admitted: Admitted) {
        let Admitted {
            run,
            base,
            overlays,
            text,
            mapping,
            attached,
            roster,
            context,
        } = admitted;
        let emit = |event: RunEvent| (self.deps.emit)(&run.id, event);
        let agent = run.config.agent.clone();

        // The slot is claimed before probing, so a second instruction still meets
        // RUN_IN_PROGRESS.
        let state = agent_state(&run.config, false).await;
        if let Err(error) = readiness(&state, &agent) {
            self.release(&run);
            log::log(&format!(
                "agent run {} refused: {} is not ready",
                run.id,
                agent_label(&agent)
            ));
            emit(RunEvent::Error(error));
            return;
        }

        let holding = run
            .session_id
            .as_ref()
            .and_then(|id| self.held.borrow().get(id).cloned());
        let held = holding
            .as_ref()
            .filter(|holding| holding.agent == agent)
            .map(|holding| holding.session_id.clone());
        let resuming = if mapping {
            None
        } else {
            session_from(&context, &agent).or(held)
        };

        let attached_name = attached.as_ref().map(|skill| skill.name.clone());
        let mut handover = Handover::default();
        let (built, instruction) = if mapping {
            match self.prepare_mapping(&run, &base, &context).await {
                Ok(Some(prepared)) => prepared,
                Ok(None) => return,
                Err(message) => {
                    self.release(&run);
                    emit(RunEvent::Error(run_error("AGENT_FAILED", message)));
                    return;
                }
            }
        } else {
            // A session that is not being resumed has been handed nothing, whatever the browser
            // recorded.
            let files = attached_files(&context, resuming.is_some());
            let analyses = Rc::clone(&self.deps.analyses);
            let wait = move |file_id: String,
                             signal: Abort|
                  -> LocalBoxFuture<'static, Option<FileReport>> {
                let analyses = Rc::clone(&analyses);
                Box::pin(async move { analyses.wait(&file_id, &signal).await })
            };
            handover = hand_over(
                &files,
                &HandoverDeps {
                    agent: &agent,
                    signal: &run.abort,
                    emit: &emit,
                    wait: &wait,
                },
            )
            .await;
            let profile = read_profile();
            let task = task_of(&context);
            let built = build_system_prompt(
                &base,
                &overlays,
                &PromptExtras {
                    instructions: Some(profile.instructions.clone()),
                    profile: profile_block(&profile),
                    attached,
                    focus: focus_block(context.get("focus")),
                    tabs: tabs_block(roster.as_deref()),
                    attachments: handover.known.clone(),
                    recordings: recordings_block(context.get("recordings")),
                    scheduled: scheduled_block(task.as_ref()),
                    fetched: None,
                },
            );
            // The CLI sends the prompt its session began with, so what has changed since goes in
            // the message.
            let update = resuming
                .as_ref()
                .filter(|_| runner_of(&agent).keeps_first_prompt())
                .and_then(|resuming| {
                    let known = holding
                        .as_ref()
                        .filter(|holding| holding.agent == agent && holding.session_id == *resuming)
                        .map(|holding| holding.sections.as_slice());
                    prompt_update(known, &built)
                });
            if let Some(update) = &update {
                log::log(&format!(
                    "agent run {} brings its session's prompt up to date: {}",
                    run.id,
                    update.changed.join(", ")
                ));
            }
            let instruction = turn_message(
                &text,
                update.as_ref().map(|update| update.text.as_str()),
                handover.reports.as_deref(),
            );
            (built, instruction)
        };

        if run.abort.aborted() {
            let staged = run.map.borrow().as_ref().map(|map| map.staging.id.clone());
            if let Some(id) = staged {
                let _ = discard_staging(&id);
            }
            self.release(&run);
            log::log(&format!(
                "agent run {} was cancelled before it started",
                run.id
            ));
            emit(RunEvent::Error(run_error("CANCELLED", "Run cancelled.")));
            return;
        }

        let BuiltPrompt {
            prompt,
            dropped,
            sections,
        } = built;
        let applied = attached_name.filter(|name| !dropped.contains(name));
        let overlay_names: Vec<String> = overlays
            .iter()
            .map(|overlay| overlay.name.clone())
            .collect();
        let mapped = run.map.borrow().as_ref().map(|map| {
            (
                map.target.host.clone(),
                map.settings.timeout_ms,
                map.settings.research,
            )
        });
        log::log(&format!(
            "agent run {} started with skill \"{}\"{}{}{}",
            run.id,
            base.name,
            applied
                .as_ref()
                .map(|name| format!(" + attached \"{name}\""))
                .unwrap_or_default(),
            if overlay_names.is_empty() {
                String::new()
            } else {
                format!(" + site notes [{}]", overlay_names.join(", "))
            },
            mapped
                .as_ref()
                .map(|(host, ..)| format!(" mapping {host}"))
                .unwrap_or_default()
        ));
        let mut shown = overlay_names;
        shown.extend(
            dropped
                .iter()
                .map(|name| format!("{name} (too large — not applied)")),
        );
        emit(RunEvent::Started {
            skill: base.name.clone(),
            attached: applied.map(Value::String),
            overlays: Some(json!(shown)),
        });
        if !handover.handed.is_empty() {
            log::log(&format!(
                "agent run {} carries {} file report(s)",
                run.id,
                handover.handed.len()
            ));
            emit(RunEvent::Attachments(json!(handover.handed)));
        }

        let (allowance, research) = match mapped {
            Some((_, timeout_ms, research)) => (Some(timeout_ms), research),
            None => (
                run.scheduled.then_some(TASK_RUN_MAX_MS.unsigned_abs()),
                false,
            ),
        };
        let request = RunRequest {
            run_id: run.id.clone(),
            instruction,
            system_prompt: prompt,
            config: &run.config,
            research,
            session_id: resuming.clone(),
            signal: &run.abort,
            emit: &emit,
        };
        let outcome = match allowance {
            Some(ms) => abort_after(run_instruction(request), &run.abort, ms).await,
            None => run_instruction(request).await,
        };
        match outcome {
            Ok(outcome) => {
                if !mapping {
                    if let Some(conversation) = &run.session_id {
                        let mut held = self.held.borrow_mut();
                        match &outcome.session_id {
                            Some(session_id) => {
                                held.insert(
                                    conversation.clone(),
                                    Held {
                                        agent: agent.clone(),
                                        session_id: session_id.clone(),
                                        sections,
                                    },
                                );
                            }
                            None => {
                                held.remove(conversation);
                            }
                        }
                    }
                    // A turn that names no session tells the panel so too.
                    let same = matches!(
                        (&outcome.session_id, &resuming),
                        (Some(now), Some(before)) if now == before
                    );
                    if !same {
                        emit(RunEvent::Session {
                            agent: agent.clone(),
                            agent_session_id: outcome.session_id.clone(),
                        });
                    }
                }
                log::log(&format!(
                    "agent run {} finished ({})",
                    run.id, outcome.stop_reason
                ));
                emit(RunEvent::Done(outcome.stop_reason));
            }
            Err(error) => {
                log::log(&format!(
                    "agent run {} failed: {}: {}",
                    run.id, error.code, error.message
                ));
                emit(RunEvent::Error(error));
            }
        }
        let unsubmitted = run
            .map
            .borrow()
            .as_ref()
            .filter(|map| !map.submitted)
            .map(|map| map.staging.id.clone());
        if let Some(id) = unsubmitted {
            let _ = discard_staging(&id);
        }
        self.release(&run);
    }

    fn release(&self, run: &ActiveRun) {
        self.runs
            .borrow_mut()
            .retain(|held| !std::ptr::eq(Rc::as_ptr(held), run));
    }

    /// A mapping run's prompt and brief, after the site's own index is read; `Ok(None)` when it
    /// was refused, which it has already said.
    async fn prepare_mapping(
        &self,
        run: &ActiveRun,
        skill: &Skill,
        context: &Value,
    ) -> Result<Option<(BuiltPrompt, String)>, String> {
        let Some(url) = str_field(context, "url").filter(|url| !url.is_empty()) else {
            self.release(run);
            self.emit_error(
                &run.id,
                "INVALID_INPUT",
                "Open the site you want mapped in this tab first.",
            );
            return Ok(None);
        };
        let target = match map_target_for(url) {
            Ok(target) => target,
            Err(message) => {
                self.release(run);
                self.emit_error(&run.id, "MAPPING_REFUSED", message);
                return Ok(None);
            }
        };

        sweep_staging();
        let settings = site_map_settings(&run.config);
        let staging = prepare_staging().map_err(|error| error.to_string())?;

        let tool_id = uuid_v4();
        (self.deps.emit)(
            &run.id,
            RunEvent::Tool {
                tool_id: tool_id.clone(),
                action: READ_SITEMAP_ACTION.into(),
                input: json!({"origin": target.origin}),
                source: None,
            },
        );
        let index = fetch_site_index(&target.origin, &run.abort).await;
        (self.deps.emit)(
            &run.id,
            RunEvent::ToolResult {
                tool_id,
                ok: true,
                summary: if index.url_count > 0 {
                    format!("{} URLs from {}", index.url_count, index.source)
                } else {
                    "no sitemap published".into()
                },
            },
        );
        write_evidence(&staging, "sitemap.txt", &index.raw).map_err(|error| error.to_string())?;

        let built = build_system_prompt(
            skill,
            &[],
            &PromptExtras {
                fetched: Some(fetched_block(&target.origin, &index)),
                ..PromptExtras::default()
            },
        );
        let brief = mapping_brief(&target.host, &settings);
        *run.map.borrow_mut() = Some(MapRun {
            target,
            staging,
            index,
            settings,
            tab_id: context.get("tabId").and_then(safe_integer),
            shots: 0,
            pages_visited: Vec::new(),
            off_site: None,
            submitted: false,
        });
        Ok(Some((built, brief)))
    }

    /// `vexdom.saveSiteMap`: the run's report checked and staged for the user's review; `None`
    /// when this is not a mapping run.
    fn submit_site_map(&self, run: &ActiveRun, input: &Value) -> Option<ActionResult> {
        let draft = {
            let mut slot = run.map.borrow_mut();
            let map = slot.as_mut()?;
            if map.submitted {
                return Some(failure(
                    "ALREADY_SUBMITTED",
                    "This run has already written its map.",
                ));
            }
            let raw = input.get("report");
            let size = serde_json::to_string(raw.unwrap_or(&Value::Null))
                .map_or(0, |text| utf16_len(&text));
            if size > MAX_SITE_MAP_REPORT {
                return Some(failure(
                    "MAPPING_REPORT_TOO_LARGE",
                    "That report is far too large. Summarize each page in a sentence.",
                ));
            }
            let checked = match validate_site_map_report(
                raw,
                &map.target.origin,
                &staged_screenshots(&map.staging),
            ) {
                Ok(checked) => checked,
                Err(message) => return Some(failure("INVALID_INPUT", message)),
            };
            let staged = stage_site_map(&StageArgs {
                staging: &map.staging,
                target: &map.target,
                report: &checked.report,
                index: &map.index,
                background: None,
                warnings: &checked.warnings,
                run_id: &run.id,
            });
            match staged {
                Ok(draft) => {
                    map.submitted = true;
                    draft
                }
                Err(error) => return Some(failure("AGENT_FAILED", error.to_string())),
            }
        };
        (self.deps.draft)(&run.id, &draft);
        let field = |key: &str| draft.get(key).cloned().unwrap_or(Value::Null);
        log::log(&format!(
            "mapping run {} staged {} ({} pages, {} shots)",
            run.id,
            js_text(&field("name")),
            js_text(&field("pages")),
            js_text(&field("screenshots"))
        ));
        Some(Ok(json!({
            "staged": true,
            "name": field("name"),
            "pages": field("pages"),
            "message": "The map is written and waiting for the user to review it. Tell them what you found, briefly.",
        })))
    }

    fn cancel(&self, run_id: &str) {
        let cancelled = {
            let mut runs = self.runs.borrow_mut();
            runs.iter()
                .position(|run| run.id == run_id)
                .map(|at| runs.remove(at))
        };
        let Some(run) = cancelled else {
            log::log(&format!("cancel for run {run_id}, which is not running"));
            (self.deps.emit)(run_id, RunEvent::Done("cancelled".into()));
            return;
        };
        let pending: Vec<oneshot::Sender<Decision>> = run
            .pending
            .borrow_mut()
            .drain()
            .map(|(_, settle)| settle)
            .collect();
        for settle in pending {
            let _ = settle.send(Decision::default());
        }
        run.abort.abort();
        log::log(&format!("agent run {run_id} cancelled"));
    }
}

/// The active agent ready to run, or why it cannot (`activeRunner(state)`).
fn readiness(state: &Value, agent: &str) -> Result<(), RunError> {
    let active = state.get("active").and_then(Value::as_str);
    let runner = state
        .get("runners")
        .and_then(Value::as_array)
        .and_then(|runners| {
            runners
                .iter()
                .find(|runner| runner.get("kind").and_then(Value::as_str) == active)
        });
    if runner.and_then(|runner| runner.get("ready")) == Some(&Value::Bool(true)) {
        return Ok(());
    }
    let problem = runner.and_then(|runner| runner.get("problem"));
    let field = |key: &str| problem.and_then(|problem| str_field(problem, key));
    let fix = field("fix")
        .filter(|fix| !fix.is_empty())
        .map(|fix| format!(" {fix}"))
        .unwrap_or_default();
    Err(run_error(
        field("code").unwrap_or("AGENT_MISSING"),
        format!(
            "{} cannot run. {}{fix}",
            agent_label(agent),
            field("message").unwrap_or("It is not available.")
        ),
    ))
}

fn outcome_of(result: &ActionResult) -> &str {
    match result {
        Ok(_) => "ok",
        Err(error) => &error.code,
    }
}

fn site_of(url: &str) -> Option<String> {
    let url = url::Url::parse(url).ok()?;
    scope::normalize_host(&scope::hostname(&url))
}

fn explicitly_asked(text: &str) -> bool {
    const MENTION: &str = "@site-mapper";
    let text = js_trim(text);
    text.get(..MENTION.len())
        .is_some_and(|head| head.eq_ignore_ascii_case(MENTION))
        && !text[MENTION.len()..]
            .chars()
            .next()
            .is_some_and(|next| next.is_ascii_alphanumeric() || next == '_')
}

/// `!!value` for a JSON value.
fn truthy(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) => false,
        Some(Value::Bool(flag)) => *flag,
        Some(Value::Number(number)) => number.as_f64().is_some_and(|n| n != 0.0 && !n.is_nan()),
        Some(Value::String(text)) => !text.is_empty(),
        Some(Value::Array(_) | Value::Object(_)) => true,
    }
}

fn focus_block(focus: Option<&Value>) -> Option<String> {
    let focus = focus.filter(|focus| focus.is_object())?;
    let field = |key: &str| str_field(focus, key).unwrap_or_default();
    let named = |key: &str| str_field(focus, key).filter(|value| !value.is_empty());
    let role = named("role")
        .map(|role| format!(", role `{}`", flatten(role)))
        .unwrap_or_default();
    let mut lines = vec![
        format!("- Selector: `{}`", flatten(field("selector"))),
        format!("- Element: `<{}>`{role}", flatten(field("tag"))),
    ];
    if let Some(label) = named("label") {
        lines.push(format!("- Label: {}", flatten(label)));
    }
    lines.push(format!(
        "- On: {} — {}",
        flatten(field("title")),
        flatten(field("url"))
    ));
    if named("shot").is_some() {
        lines.push(
            "- A screenshot of it, taken at the instant they picked it, is attached — call `act {\"do\":\"focusShot\"}` once to see it.".to_owned(),
        );
    }

    let full = field("content");
    let content = utf16_prefix(full, MAX_FOCUS_CONTENT);
    let cut = truthy(focus.get("truncated")) || content.len() < full.len();
    let text = js_trim(content);
    lines.extend([
        String::new(),
        if cut {
            "Its text when they pointed at it (cut short — re-read it for the rest):"
        } else {
            "Its text when they pointed at it:"
        }
        .to_owned(),
        String::new(),
        if text.is_empty() {
            "(no text — it is an image, an icon or an empty container)".to_owned()
        } else {
            text.to_owned()
        },
    ]);
    Some(lines.join("\n"))
}

fn recordings_block(recordings: Option<&Value>) -> Option<String> {
    let recordings = recordings?
        .as_array()
        .filter(|recordings| !recordings.is_empty())?;
    let mut lines: Vec<String> = Vec::new();
    let mut used = 0;
    let mut dropped = 0;

    for recording in recordings {
        let field = |key: &str| str_field(recording, key).unwrap_or_default();
        let duration = recording
            .get("durationMs")
            .and_then(Value::as_f64)
            .unwrap_or(0.0);
        // `Math.max(1, Math.round(ms / 60_000))`.
        let minutes = (duration / 60_000.0 + 0.5).floor().max(1.0);
        let steps = recording
            .get("steps")
            .filter(|steps| !steps.is_null())
            .map_or_else(|| "0".to_owned(), js_text);
        let mut parts = vec![
            format!(
                "- {} (id: {})",
                flatten(field("name")),
                flatten(field("id"))
            ),
            format!(
                "  on {} · {steps} steps · about {minutes} min",
                flatten(field("host"))
            ),
        ];
        if let Some(goal) = str_field(recording, "goal").filter(|goal| !goal.is_empty()) {
            parts.push(format!("  Goal: {}", flatten(goal)));
        }
        if !truthy(recording.get("capturedValues")) {
            parts
                .push("  Typed values were not captured; every value is a {{placeholder}}.".into());
        }

        let section = parts.join("\n");
        let length = utf16_len(&section);
        if used + length > MAX_RECORDINGS_BLOCK {
            dropped += 1;
            continue;
        }
        used += length;
        lines.push(section);
    }

    if lines.is_empty() {
        return None;
    }
    if dropped > 0 {
        log::log(&format!(
            "recording context is full; left out {dropped} recording(s)"
        ));
        lines.push(format!(
            "({dropped} more recording(s) did not fit here — listRecordings still lists them.)"
        ));
    }
    Some(lines.join("\n"))
}

/// Agents name their own sessions: UUIDs, thread ids, conversation ids. Any tame token will do.
fn is_session_id(id: &str) -> bool {
    (6..=200).contains(&id.len())
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b':' | b'-'))
}

/// The session the browser says this agent holds for the conversation.
fn session_from(context: &Value, agent: &str) -> Option<String> {
    if str_field(context, "agent") != Some(agent) {
        return None;
    }
    str_field(context, "agentSessionId")
        .filter(|id| is_session_id(id))
        .map(str::to_owned)
}

/// The conversation's tabs as the browser sent them, kept only when well-formed and more than
/// one.
fn roster_from(context: &Value) -> Option<Vec<ConversationTab>> {
    let entries = context.get("tabs")?.as_array()?;
    let mut seen = HashSet::new();
    let mut roster = Vec::new();
    for entry in entries.iter().take(MAX_ROSTER) {
        let (Some(tab_id), Some(title)) = (
            entry.get("tabId").and_then(safe_integer),
            str_field(entry, "title"),
        ) else {
            continue;
        };
        if !seen.insert(tab_id) {
            continue;
        }
        roster.push(ConversationTab {
            tab_id,
            title: title.to_owned(),
            host: str_field(entry, "host")
                .filter(|host| TAB_HOST.is_match(host))
                .map(str::to_owned),
            current: entry.get("current") == Some(&Value::Bool(true)),
            chosen: entry.get("chosen") == Some(&Value::Bool(true)),
        });
    }
    (roster.len() > 1).then_some(roster)
}

fn task_of(context: &Value) -> Option<TaskContext> {
    let task = context.get("task").filter(|task| task.is_object())?;
    Some(TaskContext {
        name: str_field(task, "name").unwrap_or_default().to_owned(),
        previous: str_field(task, "previous").map(str::to_owned),
    })
}

/// The conversation's files; a turn that starts a session has been handed none of them.
fn attached_files(context: &Value, resuming: bool) -> Vec<AttachedFile> {
    let Some(files) = context.get("files").and_then(Value::as_array) else {
        return Vec::new();
    };
    files
        .iter()
        .filter_map(AttachedFile::from_json)
        .map(|mut file| {
            file.delivered &= resuming;
            file
        })
        .collect()
}

fn fetched_block(origin: &str, index: &SiteIndex) -> String {
    if index.url_count == 0 {
        return format!(
            "No sitemap is published at {origin}. Discover the site by walking its own links."
        );
    }
    let mut lines = vec![
        format!(
            "## {origin} sitemap ({}, {} URLs across {} documents)",
            index.source, index.url_count, index.documents
        ),
        String::new(),
        "### Shapes".to_owned(),
    ];
    lines.extend(index.patterns.iter().map(|pattern| {
        format!(
            "- {} — {} pages, e.g. {}",
            pattern.pattern, pattern.count, pattern.example
        )
    }));
    lines.extend([String::new(), "### Paths".to_owned()]);
    lines.extend(index.paths.iter().map(|path| format!("- {path}")));
    if index.truncated {
        lines.push("- (truncated: the sitemap lists more than was read)".into());
    }
    lines.join("\n")
}

fn mapping_brief(host: &str, settings: &SiteMapSettings) -> String {
    format!(
        "Map {host}. Visit up to {} pages and take up to {} screenshots, then call act {{\"do\":\"saveSiteMap\"}} once with everything you found. Prefer breadth over depth: the landing page, the main navigation destinations, and one example of each repeated page shape. Record what you observed, not advice — a future assistant will read this to find its way around, and anything that reads like an instruction will be shown to the user as suspicious.",
        settings.max_pages, settings.max_screenshots
    )
}

/// One line for the panel's timeline. A screenshot's image travels in `Reply.image`, so that, or
/// a `dataUrl` an action still returns, is what marks one.
fn summarize(target: &Target, reply: &Reply) -> String {
    let data = match &reply.result {
        Err(error) => return clip(&format!("{}: {}", error.code, error.message)),
        Ok(data) => data,
    };

    if reply.image.is_some() || data.get("dataUrl").is_some_and(Value::is_string) {
        let saved = str_field(data, "savedTo")
            .map(|path| {
                let name = Path::new(path)
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_default();
                format!(" → {name}")
            })
            .unwrap_or_default();
        let size = |key: &str| {
            data.get(key)
                .filter(|value| !value.is_null())
                .map_or_else(|| "?".to_owned(), js_text)
        };
        return clip(&format!(
            "image {}×{}{saved}",
            size("width"),
            size("height")
        ));
    }

    if let (Some(_), Some(label), Some(state)) = (
        str_field(data, "vendor"),
        str_field(data, "label"),
        str_field(data, "state"),
    ) {
        return clip(&format!("{label}: {state}"));
    }

    let submits = if data.get("submits") == Some(&Value::Bool(true)) {
        " — submits a form"
    } else {
        ""
    };
    match target {
        Target::Text(text) => return clip(&format!("“{text}”{submits}")),
        Target::Selector(selector) => return clip(&format!("{selector}{submits}")),
        Target::Unnamed => {}
    }

    ["navigatedTo", "navigatingTo", "url", "performed", "value"]
        .into_iter()
        .find_map(|key| str_field(data, key).filter(|value| !value.is_empty()))
        .map_or_else(|| "done".to_owned(), clip)
}

fn clip(text: &str) -> String {
    let flat = flatten(&seal_secrets(text));
    if utf16_len(&flat) > SUMMARY_LIMIT {
        format!("{}…", utf16_prefix(&flat, SUMMARY_LIMIT - 1))
    } else {
        flat
    }
}

#[cfg(test)]
mod tests {
    use std::assert_matches;
    use std::cell::RefCell;
    use std::future::Future;
    #[cfg(unix)]
    use std::path::PathBuf;
    use std::time::Instant;

    use vexdom_core::config::config_path;

    use super::*;
    use crate::agent::test_support::{local, scratch};

    const CODE_TOOLS: [&str; 2] = [INJECT_ACTION, RUN_CODE_ACTION];

    /// Only Claude Code is configured, at `bin`, so the readiness probe starts nothing else.
    fn only_claude(bin: &str) {
        let path = config_path();
        std::fs::create_dir_all(path.parent().expect("config.json sits in the state folder"))
            .expect("the scratch state folder can be made");
        std::fs::write(
            &path,
            json!({"agent": "claude", "agents": {"claude": {"bin": bin}}}).to_string(),
        )
        .expect("config.json can be written");
    }

    /// A stand-in Claude Code in `dir`: it answers the version probe, then runs `body`.
    #[cfg(unix)]
    fn stand_in(dir: &Path, body: &str) -> String {
        use std::os::unix::fs::PermissionsExt;

        let bin = dir.join("claude");
        std::fs::write(
            &bin,
            format!(
                "#!/bin/sh\nif [ \"$1\" = --version ]; then echo '2.1.283 (Claude Code)'; exit 0; fi\n{body}\n"
            ),
        )
        .expect("the stand-in can be written");
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755))
            .expect("the stand-in can be made executable");
        bin.to_string_lossy().into_owned()
    }

    /// Reads its turn, then thinks it over until it is stopped.
    #[cfg(unix)]
    const WAITS: &str = "cat > /dev/null\nexec sleep 30";

    /// Records what it hears into `dir/turn-N`, and its system prompt into `dir/prompt-N`, then
    /// ends the turn in session `sess-1`.
    #[cfg(unix)]
    fn recording(dir: &Path) -> String {
        format!(
            r#"dir='{}'
n=$(ls "$dir" | grep -c '^turn-')
prompt=''
next=''
for arg in "$@"; do
  if [ -n "$next" ]; then prompt="$arg"; next=''; fi
  if [ "$arg" = --append-system-prompt-file ]; then next=1; fi
done
cp "$prompt" "$dir/prompt-$n"
cat > "$dir/turn-$n"
printf '%s\n' '{{"type":"system","subtype":"init","session_id":"sess-1","mcp_servers":[{{"name":"vexdom","status":"connected"}}]}}'
printf '%s\n' '{{"type":"result","subtype":"success","is_error":false,"stop_reason":"end_turn","result":"ok"}}'"#,
            dir.display()
        )
    }

    fn offline(_action: &str) -> ActionResult {
        failure("EXTENSION_OFFLINE", "no browser")
    }

    /// A session whose page answers every call with `answer`, and what it said and was asked.
    struct Rig {
        session: Rc<AgentSession>,
        events: Rc<RefCell<Vec<(String, RunEvent)>>>,
        invoked: Rc<RefCell<Vec<(String, Value)>>>,
        /// Runs going in other browsers.
        elsewhere: Rc<Cell<usize>>,
    }

    fn rig(answer: impl Fn(&str) -> ActionResult + 'static) -> Rig {
        let events: Rc<RefCell<Vec<(String, RunEvent)>>> = Rc::default();
        let invoked: Rc<RefCell<Vec<(String, Value)>>> = Rc::default();
        let elsewhere: Rc<Cell<usize>> = Rc::default();
        let (seen, said, load) = (
            Rc::clone(&invoked),
            Rc::clone(&events),
            Rc::clone(&elsewhere),
        );
        let deps = AgentDeps {
            invoke: Rc::new(move |action: String, input, _opts| {
                let reply = Reply::from(answer(&action));
                seen.borrow_mut().push((action, input));
                Box::pin(async move { reply })
            }),
            emit: Rc::new(move |id, event| said.borrow_mut().push((id.to_owned(), event))),
            draft: Rc::new(|_, _| {}),
            running: Rc::new(move || load.get()),
            analyses: Rc::default(),
        };
        Rig {
            session: AgentSession::new(deps),
            events,
            invoked,
            elsewhere,
        }
    }

    impl Rig {
        fn instruct(&self, id: &str, text: &str, context: Value) {
            self.session.handle(AgentRequest::Instruct {
                id: id.into(),
                text: text.into(),
                context,
            });
        }

        fn call(
            &self,
            id: &str,
            action: &str,
            input: Value,
        ) -> impl Future<Output = Reply> + use<> {
            Rc::clone(&self.session).invoke_for_run(id.into(), action.into(), input)
        }

        fn answer(&self, id: &str, tool_id: &str, allow: bool) {
            self.session.handle(AgentRequest::Decision {
                id: id.into(),
                tool_id: tool_id.into(),
                allow,
                remember: false,
            });
        }

        /// How `id` ended, once it has.
        fn ending(&self, id: &str) -> Option<RunEvent> {
            self.events
                .borrow()
                .iter()
                .find(|(run, event)| {
                    run == id && matches!(event, RunEvent::Error(_) | RunEvent::Done(_))
                })
                .map(|(_, event)| event.clone())
        }

        /// Every approval asked for: its tool id and action.
        fn asked(&self) -> Vec<(String, String)> {
            self.events
                .borrow()
                .iter()
                .filter_map(|(_, event)| match event {
                    RunEvent::Approval {
                        tool_id, action, ..
                    } => Some((tool_id.clone(), action.clone())),
                    _ => None,
                })
                .collect()
        }

        /// The kinds of the tool events, in order.
        fn kinds(&self) -> Vec<&'static str> {
            self.events
                .borrow()
                .iter()
                .filter_map(|(_, event)| match event {
                    RunEvent::Tool { .. } => Some("tool"),
                    RunEvent::Approval { .. } => Some("approval"),
                    RunEvent::ToolResult { .. } => Some("toolResult"),
                    _ => None,
                })
                .collect()
        }

        /// Every tool result, marked by whether it answers the approval asked for.
        fn results(&self) -> Vec<String> {
            let asked: Vec<String> = self
                .asked()
                .into_iter()
                .map(|(tool_id, _)| tool_id)
                .collect();
            self.events
                .borrow()
                .iter()
                .filter_map(|(_, event)| match event {
                    RunEvent::ToolResult {
                        tool_id, summary, ..
                    } => Some(format!(
                        "{}: {summary}",
                        if asked.contains(tool_id) {
                            "asked"
                        } else {
                            "other"
                        }
                    )),
                    _ => None,
                })
                .collect()
        }

        fn invoked_actions(&self) -> Vec<String> {
            self.invoked
                .borrow()
                .iter()
                .map(|(action, _)| action.clone())
                .collect()
        }

        /// Admits `id`, reads its offer, then waits for it to end: refused, when no agent can be
        /// found.
        async fn offer_to(&self, id: &str, context: Value) -> Option<Offer> {
            self.instruct(id, "click Sign in", context);
            let offer = self.session.offer_for(id);
            until(id, || self.ending(id).is_some()).await;
            offer
        }

        /// Cancels every run and waits for `ids` to end.
        async fn finish(&self, ids: &[&str]) {
            self.session.dispose();
            for id in ids {
                until(id, || self.ending(id).is_some()).await;
            }
        }
    }

    async fn until(what: &str, done: impl Fn() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !done() {
            assert!(Instant::now() < deadline, "timed out waiting for {what}");
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }

    fn offer(withheld: &[&str]) -> Offer {
        Offer {
            withheld: withheld
                .iter()
                .chain(&[AUTH_SAVE_ACTION])
                .map(|name| (*name).to_owned())
                .collect(),
            reserved: vec![FOCUS_SHOT_ACTION.into()],
        }
    }

    #[test]
    fn a_run_that_is_not_active_is_offered_nothing() {
        assert_eq!(rig(offline).session.offer_for("gone"), None);
    }

    #[test]
    fn the_page_code_tools_are_offered_only_with_live_tools_on() {
        let _home = scratch("service-live-tools");
        only_claude("/nonexistent/claude");
        local(async {
            let rig = rig(offline);
            let shut = rig.offer_to("a", json!({"sessionId": "s1"})).await;
            let open = rig
                .offer_to("b", json!({"sessionId": "s2", "liveTools": true}))
                .await;
            assert_eq!(shut, Some(offer(&CODE_TOOLS)));
            assert_eq!(open, Some(offer(&[])));
            assert_matches!(
                rig.ending("a"),
                Some(RunEvent::Error(error)) if error.code == "AGENT_MISSING"
            );
        });
    }

    #[test]
    fn a_conversation_shown_the_page_code_tools_keeps_them_until_it_is_reset() {
        let _home = scratch("service-code-listed");
        only_claude("/nonexistent/claude");
        local(async {
            let rig = rig(offline);
            rig.offer_to("a", json!({"sessionId": "s1", "liveTools": true}))
                .await;
            let switched_off = rig.offer_to("b", json!({"sessionId": "s1"})).await;
            rig.session.handle(AgentRequest::Reset {
                session_id: Some("s1".into()),
            });
            let after_reset = rig.offer_to("c", json!({"sessionId": "s1"})).await;
            assert_eq!(
                switched_off.map(|offer| offer.withheld),
                Some(vec![AUTH_SAVE_ACTION.to_owned()])
            );
            let mut withheld = CODE_TOOLS.map(str::to_owned).to_vec();
            withheld.push(AUTH_SAVE_ACTION.to_owned());
            assert_eq!(after_reset.map(|offer| offer.withheld), Some(withheld));
        });
    }

    #[test]
    fn a_second_instruction_meets_the_run_in_progress_and_the_limit() {
        let _home = scratch("service-admission");
        only_claude("/nonexistent/claude");
        local(async {
            let rig = rig(offline);
            rig.instruct("a", "click Sign in", json!({"sessionId": "s1"}));
            rig.instruct("b", "click Sign up", json!({"sessionId": "s1"}));
            rig.instruct("c", "   ", json!({"sessionId": "s2"}));
            rig.elsewhere.set(3);
            rig.instruct("d", "click Sign in", json!({"sessionId": "s3"}));
            let code = |id: &str| match rig.ending(id) {
                Some(RunEvent::Error(error)) => error.code,
                other => panic!("{id} ended with {other:?}"),
            };
            assert_eq!(
                [code("b"), code("c"), code("d")],
                ["RUN_IN_PROGRESS", "INVALID_INPUT", "RUN_LIMIT"]
            );
            assert!(
                rig.session.owns("a"),
                "the first instruction holds its slot"
            );
            until("a", || rig.ending("a").is_some()).await;
            assert!(!rig.session.owns("a"), "a refused run gives its slot back");

            rig.session.handle(AgentRequest::Cancel { id: "a".into() });
            assert_eq!(
                rig.events.borrow().last(),
                Some(&("a".to_owned(), RunEvent::Done("cancelled".into())))
            );
        });
    }

    #[cfg(unix)]
    #[test]
    fn a_captcha_is_asked_about_once_and_its_later_rounds_go_through() {
        let home = scratch("service-captcha");
        only_claude(&stand_in(home.path(), WAITS));
        local(async {
            let rig = rig(|_| failure("CAPTCHA_NOT_FOUND", "none"));
            rig.instruct("r1", "sign me up", json!({"sessionId": "s1"}));
            assert!(
                rig.session.offer_for("r1").is_some(),
                "the run is admitted before handle returns"
            );

            let first = tokio::task::spawn_local(rig.call("r1", CAPTCHA_ACTION, json!({})));
            until("the approval", || !rig.asked().is_empty()).await;
            let (tool_id, _) = rig.asked()[0].clone();
            rig.answer("r1", &tool_id, true);
            first.await.expect("the first round finishes");
            rig.call("r1", CAPTCHA_ACTION, json!({"tiles": [1, 4]}))
                .await;
            rig.finish(&["r1"]).await;

            assert_eq!(
                rig.asked().len(),
                1,
                "one yes covers every round of the run"
            );
            assert_eq!(rig.invoked_actions(), [CAPTCHA_ACTION, CAPTCHA_ACTION]);
        });
    }

    #[cfg(unix)]
    #[test]
    fn a_run_switches_among_its_own_tabs_and_asks_to_close_one() {
        let home = scratch("service-tabs");
        only_claude(&stand_in(home.path(), WAITS));
        local(async {
            let rig = rig(|_| Ok(json!({"done": true})));
            let tabs = json!([
                {"tabId": 3, "title": "Board", "host": "jira.example.com", "current": true, "chosen": true},
                {"tabId": 9, "title": "Ticket", "host": "zendesk.example.com", "chosen": true},
                {"tabId": 9, "title": "the same tab again"},
                {"tabId": "x", "title": "not a tab"},
                null,
            ]);
            rig.instruct(
                "r1",
                "match the ticket to its issue",
                json!({"sessionId": "s1", "url": "https://jira.example.com/b", "tabId": 3, "tabs": tabs}),
            );
            rig.call("r1", "page.switchTab", json!({"tabId": 9})).await;
            rig.call(
                "r1",
                "page.navigate",
                json!({"url": "https://zendesk.example.com/t/4411"}),
            )
            .await;
            let close =
                tokio::task::spawn_local(rig.call("r1", "page.closeTab", json!({"tabId": 9})));
            until("the approval", || !rig.asked().is_empty()).await;
            rig.finish(&["r1"]).await;
            let closed = close.await.expect("the close call settles");

            assert_eq!(
                *rig.invoked.borrow(),
                [
                    ("page.switchTab".to_owned(), json!({"tabId": 9})),
                    (
                        "page.navigate".to_owned(),
                        json!({"url": "https://zendesk.example.com/t/4411"})
                    ),
                ]
            );
            let asked: Vec<String> = rig.asked().into_iter().map(|(_, action)| action).collect();
            assert_eq!(asked, ["page.closeTab"]);
            assert_eq!(
                closed.result.map_err(|error| error.code),
                Err("DECLINED".to_owned()),
                "a cancelled run declines what it was waiting on"
            );
        });
    }

    #[cfg(unix)]
    #[test]
    fn a_run_asks_before_closing_the_tab_it_switched_to() {
        let home = scratch("service-bare-close");
        only_claude(&stand_in(home.path(), WAITS));
        local(async {
            let rig = rig(|action| {
                Ok(if action == "page.switchTab" {
                    json!({"activeTabId": 9})
                } else {
                    json!({"done": true})
                })
            });
            let tabs = json!([
                {"tabId": 3, "title": "Board", "host": "jira.example.com", "current": true, "chosen": true},
                {"tabId": 9, "title": "Ticket", "host": "zendesk.example.com", "chosen": true},
            ]);
            rig.instruct(
                "r1",
                "tidy up",
                json!({"sessionId": "s1", "url": "https://jira.example.com/b", "tabId": 3, "tabs": tabs}),
            );
            rig.call("r1", "page.switchTab", json!({"tabId": 9})).await;
            let close = tokio::task::spawn_local(rig.call("r1", "page.closeTab", json!({})));
            until("the approval", || !rig.asked().is_empty()).await;
            rig.finish(&["r1"]).await;
            close.await.expect("the close call settles");

            assert_eq!(rig.invoked_actions(), ["page.switchTab"]);
            let asked: Vec<String> = rig.asked().into_iter().map(|(_, action)| action).collect();
            assert_eq!(asked, ["page.closeTab"]);
        });
    }

    #[cfg(unix)]
    #[test]
    fn an_approval_is_withdrawn_when_the_agent_stops_waiting() {
        let home = scratch("service-withdrawn");
        only_claude(&stand_in(home.path(), WAITS));
        local(async {
            let rig = rig(|_| Ok(json!({"done": true})));
            rig.instruct("r1", "sign me up", json!({"sessionId": "s1"}));
            let mut call = Box::pin(rig.call("r1", CAPTCHA_ACTION, json!({})));
            tokio::select! {
                _ = &mut call => panic!("the call was answered before the user did"),
                () = until("the approval", || !rig.asked().is_empty()) => {}
            }
            drop(call);
            let (tool_id, _) = rig.asked()[0].clone();
            rig.answer("r1", &tool_id, true);
            tokio::task::yield_now().await;

            assert_eq!(rig.results(), ["asked: the agent stopped waiting"]);
            assert!(rig.invoked.borrow().is_empty(), "a late yes does nothing");
            let run = rig.session.run("r1").expect("the run goes on");
            assert!(
                run.pending.borrow().is_empty(),
                "the approval was withdrawn"
            );
            drop(run);
            rig.finish(&["r1"]).await;
        });
    }

    #[cfg(unix)]
    #[test]
    fn an_approval_waits_as_long_as_the_user_takes_unless_a_schedule_started_the_run() {
        let home = scratch("service-patience");
        only_claude(&stand_in(home.path(), WAITS));
        local(async {
            let rig = rig(|_| Ok(json!({"done": true})));
            rig.instruct("r1", "sign me up", json!({"sessionId": "s1"}));
            rig.instruct(
                "r2",
                "check the PRs",
                json!({"sessionId": "s2", "task": {"id": "t1", "name": "PR digest"}}),
            );
            let wait = |id: &str| rig.session.run(id).map(|run| run.approval_wait());
            assert_eq!(wait("r1"), Some(None));
            assert_eq!(wait("r2"), Some(Some(Duration::from_secs(600))));

            let call = tokio::task::spawn_local(rig.call("r1", CAPTCHA_ACTION, json!({})));
            until("the approval", || !rig.asked().is_empty()).await;
            tokio::time::sleep(Duration::from_millis(200)).await;
            let before = rig.kinds();
            let (tool_id, _) = rig.asked()[0].clone();
            rig.answer("r1", &tool_id, true);
            let reply = call.await.expect("the call finishes");
            rig.finish(&["r1", "r2"]).await;

            assert_eq!(before, ["tool", "approval"]);
            assert_eq!(reply.result, Ok(json!({"done": true})));
        });
    }

    #[cfg(unix)]
    #[test]
    fn a_wait_inside_a_call_is_left_as_asked() {
        let home = scratch("service-await");
        only_claude(&stand_in(home.path(), WAITS));
        local(async {
            let rig = rig(|_| Ok(json!({})));
            rig.instruct("r1", "wait for it", json!({"sessionId": "s1"}));
            rig.call("r1", "page.awaitMonitor", json!({"monitorId": "m1"}))
                .await;
            rig.finish(&["r1"]).await;
            assert_eq!(
                *rig.invoked.borrow(),
                [("page.awaitMonitor".to_owned(), json!({"monitorId": "m1"}))]
            );
        });
    }

    #[cfg(unix)]
    fn turns(home: &Path) -> (PathBuf, String) {
        let dir = home.join("turns");
        std::fs::create_dir_all(&dir).expect("the turns folder can be made");
        let bin = stand_in(home, &recording(&dir));
        (dir, bin)
    }

    #[cfg(unix)]
    fn read(dir: &Path, name: &str) -> String {
        std::fs::read_to_string(dir.join(name)).unwrap_or_else(|error| panic!("{name}: {error}"))
    }

    // Claude Code goes on sending the system prompt a session began with, so a turn that changes
    // it has to say what changed in its own message, and a turn that changes nothing says nothing
    // more.
    #[cfg(unix)]
    #[test]
    fn a_follow_up_carries_what_changed_since_the_session_began_once() {
        let home = scratch("service-prompt-turns");
        let (dir, bin) = turns(home.path());
        only_claude(&bin);
        local(async {
            let rig = rig(offline);
            let focus = json!({"tag": "button", "selector": "#buy", "content": "Buy now", "truncated": false, "url": "https://shop.example/", "title": "Shop"});
            for (id, context) in [
                ("r1", json!({"sessionId": "s1"})),
                ("r2", json!({"sessionId": "s1", "focus": focus})),
                ("r3", json!({"sessionId": "s1", "focus": focus})),
            ] {
                rig.instruct(id, "what does this cost?", context);
                until(id, || rig.ending(id).is_some()).await;
                assert_eq!(rig.ending(id), Some(RunEvent::Done("end_turn".into())));
            }
            rig.session.dispose();

            let said: Vec<String> = (0..3).map(|n| read(&dir, &format!("turn-{n}"))).collect();
            let marks: Vec<(bool, bool)> = said
                .iter()
                .map(|message| {
                    (
                        message.contains("# Vexdom's instructions for this message"),
                        message.contains("`#buy`"),
                    )
                })
                .collect();
            assert_eq!(marks, [(false, false), (true, true), (false, false)]);
            assert!(
                said.iter()
                    .all(|message| message.ends_with("what does this cost?")),
                "the user's words close every message: {said:?}"
            );
            let sessions: Vec<RunEvent> = rig
                .events
                .borrow()
                .iter()
                .filter(|(_, event)| matches!(event, RunEvent::Session { .. }))
                .map(|(_, event)| event.clone())
                .collect();
            assert_eq!(
                sessions,
                [RunEvent::Session {
                    agent: "claude".into(),
                    agent_session_id: Some("sess-1".into()),
                }],
                "the panel hears of the session once, when it begins"
            );
        });
    }

    #[cfg(unix)]
    #[test]
    fn the_profile_reaches_the_prompt_and_a_later_edit_reaches_a_resumed_turn() {
        let home = scratch("service-profile");
        let (dir, bin) = turns(home.path());
        only_claude(&bin);
        let save_profile = |email: &str| {
            std::fs::write(
                vexdom_core::profile::profile_path(),
                json!({"fields": {"email": email}, "details": [], "instructions": "Always choose the cheapest shipping."})
                    .to_string(),
            )
            .expect("profile.json can be written");
        };
        local(async {
            let rig = rig(offline);
            for (id, email) in [("r1", "ada@example.com"), ("r2", "ada@lovelace.dev")] {
                save_profile(email);
                rig.instruct(
                    id,
                    "sign me up for the newsletter",
                    json!({"sessionId": "s1"}),
                );
                until(id, || rig.ending(id).is_some()).await;
                assert_eq!(rig.ending(id), Some(RunEvent::Done("end_turn".into())));
            }
            rig.session.dispose();
        });

        let prompt = read(&dir, "prompt-0");
        assert!(prompt.contains("# About the user"), "{prompt}");
        assert!(prompt.contains("Email: ada@example.com"), "{prompt}");
        assert!(
            prompt.contains("Always choose the cheapest shipping."),
            "{prompt}"
        );
        assert_eq!(read(&dir, "turn-0"), "sign me up for the newsletter");
        let heard = read(&dir, "turn-1");
        assert!(
            heard.starts_with("# Vexdom's instructions for this message"),
            "{heard}"
        );
        assert!(heard.contains("Email: ada@lovelace.dev"), "{heard}");
        assert!(
            !heard.contains("Always choose the cheapest shipping."),
            "{heard}"
        );
    }

    #[test]
    fn a_summary_is_one_short_line_naming_what_the_call_touched() {
        let reply = |result: ActionResult| Reply::from(result);
        let unnamed = Target::of(&json!({}));
        let shot = Reply {
            result: Ok(
                json!({"format": "png", "width": 1280, "height": 720, "savedTo": "/shots/a.png"}),
            ),
            image: Some(b"png".to_vec()),
        };
        assert_eq!(summarize(&unnamed, &shot), "image 1280×720 → a.png");
        assert_eq!(
            summarize(
                &Target::of(&json!({"target": {"text": "Buy"}})),
                &reply(Ok(json!({"submits": true})))
            ),
            "“Buy” — submits a form"
        );
        assert_eq!(
            summarize(
                &unnamed,
                &reply(Ok(
                    json!({"vendor": "recaptcha", "label": "reCAPTCHA", "state": "solved"})
                ))
            ),
            "reCAPTCHA: solved"
        );
        assert_eq!(
            summarize(
                &unnamed,
                &reply(Ok(json!({"navigatedTo": "https://a.test/"})))
            ),
            "https://a.test/"
        );
        assert_eq!(summarize(&unnamed, &reply(Ok(Value::Null))), "done");
        let long = summarize(&unnamed, &reply(failure("TIMEOUT", "x ".repeat(100))));
        assert_eq!(utf16_len(&long), SUMMARY_LIMIT);
        assert!(long.ends_with('…'), "{long}");
    }

    #[test]
    fn the_context_is_read_as_the_browser_sent_it() {
        assert!(explicitly_asked("  @Site-Mapper map it"));
        assert!(!explicitly_asked("@site-mappers"));
        assert_eq!(
            session_from(
                &json!({"agent": "claude", "agentSessionId": "2e281281-10ef"}),
                "claude"
            ),
            Some("2e281281-10ef".into())
        );
        assert_eq!(
            session_from(
                &json!({"agent": "claude", "agentSessionId": "a b"}),
                "claude"
            ),
            None
        );
        assert_eq!(
            site_of("https://www.Example.com/x").as_deref(),
            Some("example.com")
        );
        let roster = roster_from(&json!({"tabs": [
            {"tabId": 1, "title": "One", "host": "a.test:8080"},
            {"tabId": 2, "title": "Two", "host": "not a host"},
        ]}))
        .expect("two well-formed tabs make a roster");
        assert_eq!(
            roster
                .iter()
                .map(|tab| tab.host.clone())
                .collect::<Vec<_>>(),
            [Some("a.test:8080".to_owned()), None]
        );
        let block = recordings_block(Some(&json!([
            {"id": "r1", "name": "Check out", "host": "shop.test", "durationMs": 150_000, "steps": 6, "goal": "Buy", "capturedValues": true},
            {"id": "r2", "name": "Sign up", "host": "shop.test", "durationMs": 1_000},
        ])))
        .expect("two recordings make a block");
        assert_eq!(
            block,
            "- Check out (id: r1)\n  on shop.test · 6 steps · about 3 min\n  Goal: Buy\n- Sign up (id: r2)\n  on shop.test · 0 steps · about 1 min\n  Typed values were not captured; every value is a {{placeholder}}."
        );
    }
}
