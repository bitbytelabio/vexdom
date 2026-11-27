//! Extension → daemon requests (`daemon.ts` settle's request callback), one capnp method each.

use std::future::Future;
use std::rc::{Rc, Weak};
use std::time::Instant;

use serde_json::{Value, json};
use vexdom_core::approvals;
use vexdom_core::config::{self, read_agent_config};
use vexdom_core::notices::as_device_notice;
use vexdom_core::preferences::{apply_preference, preferences_now};
use vexdom_core::result::{ActionError, ActionResult, failure};
use vexdom_core::{agents, log, skills};
use vexdom_schema::extension_session;

use crate::Daemon;
use crate::agent::{self, AgentRequest, FileRequest};
use crate::link::Link;
use crate::wire::{optional, parse_json, text, write_result};

pub(crate) struct Session {
    daemon: Daemon,
    link: Weak<Link>,
}

impl Session {
    pub(crate) fn new(daemon: Daemon, link: Weak<Link>) -> Self {
        Self { daemon, link }
    }

    /// The link this session belongs to, marked as where the user is (unless `quiet`).
    fn source(&self, quiet: bool) -> Result<Rc<Link>, capnp::Error> {
        let link = self
            .link
            .upgrade()
            .filter(|link| link.is_open())
            .ok_or_else(|| capnp::Error::disconnected("this browser's link is closed".into()))?;
        if !quiet {
            link.last_active.set(self.daemon.touch());
        }
        Ok(link)
    }
}

fn outcome(result: Result<Value, (String, String)>) -> ActionResult {
    result.map_err(ActionError::from)
}

fn unknown_agent(agent: &str) -> ActionResult {
    failure(
        "INVALID_INPUT",
        format!("\"{agent}\" is not an agent Vexdom knows."),
    )
}

macro_rules! reply {
    ($results:ident, $result:expr) => {{
        let result = $result;
        write_result($results.get().init_result(), &result);
        Ok(())
    }};
}

impl extension_session::Server for Session {
    fn ping(
        self: capnp::capability::Rc<Self>,
        _: extension_session::PingParams,
        _: extension_session::PingResults,
    ) -> impl Future<Output = Result<(), capnp::Error>> + 'static {
        if let Some(link) = self.link.upgrade() {
            link.last_ping.set(Instant::now());
        }
        std::future::ready(Ok(()))
    }

    fn focus(
        self: capnp::capability::Rc<Self>,
        _: extension_session::FocusParams,
        _: extension_session::FocusResults,
    ) -> impl Future<Output = Result<(), capnp::Error>> + 'static {
        std::future::ready(self.source(false).map(|_| ()))
    }

    fn instruct(
        self: capnp::capability::Rc<Self>,
        params: extension_session::InstructParams,
        _: extension_session::InstructResults,
    ) -> impl Future<Output = Result<(), capnp::Error>> + 'static {
        std::future::ready((|| {
            let link = self.source(false)?;
            let params = params.get()?;
            let request = AgentRequest::Instruct {
                id: text(params.get_id()),
                text: text(params.get_text()),
                context: parse_json(&text(params.get_context())),
            };
            self.daemon.session(&link).handle(request);
            Ok(())
        })())
    }

    fn cancel(
        self: capnp::capability::Rc<Self>,
        params: extension_session::CancelParams,
        _: extension_session::CancelResults,
    ) -> impl Future<Output = Result<(), capnp::Error>> + 'static {
        std::future::ready((|| {
            let link = self.source(false)?;
            let id = text(params.get()?.get_id());
            self.daemon
                .session(&link)
                .handle(AgentRequest::Cancel { id });
            Ok(())
        })())
    }

    fn decision(
        self: capnp::capability::Rc<Self>,
        params: extension_session::DecisionParams,
        _: extension_session::DecisionResults,
    ) -> impl Future<Output = Result<(), capnp::Error>> + 'static {
        std::future::ready((|| {
            let link = self.source(false)?;
            let params = params.get()?;
            let request = AgentRequest::Decision {
                id: text(params.get_id()),
                tool_id: text(params.get_tool_id()),
                allow: params.get_allow(),
                remember: params.get_remember(),
            };
            self.daemon.session(&link).handle(request);
            Ok(())
        })())
    }

    fn reset(
        self: capnp::capability::Rc<Self>,
        params: extension_session::ResetParams,
        _: extension_session::ResetResults,
    ) -> impl Future<Output = Result<(), capnp::Error>> + 'static {
        std::future::ready((|| {
            let link = self.source(false)?;
            let session_id = optional(text(params.get()?.get_session_id()));
            self.daemon
                .session(&link)
                .handle(AgentRequest::Reset { session_id });
            Ok(())
        })())
    }

    fn analyze_file(
        self: capnp::capability::Rc<Self>,
        params: extension_session::AnalyzeFileParams,
        mut results: extension_session::AnalyzeFileResults,
    ) -> impl Future<Output = Result<(), capnp::Error>> + 'static {
        let started = (|| {
            let link = self.source(false)?;
            let params = params.get()?;
            Ok::<_, capnp::Error>((
                link.serial,
                FileRequest {
                    id: text(params.get_id()),
                    file_id: text(params.get_file_id()),
                    session_id: text(params.get_session_id()),
                    name: text(params.get_name()),
                    mime: text(params.get_mime()),
                    size: params.get_size(),
                    content: params.get_content()?.to_vec(),
                },
            ))
        })();
        let analyses = std::rc::Rc::clone(&self.daemon.0.analyses);
        async move {
            let (owner, request) = started?;
            let result = analyses.start(request, read_agent_config(), owner).await;
            reply!(results, result)
        }
    }

    fn cancel_analysis(
        self: capnp::capability::Rc<Self>,
        params: extension_session::CancelAnalysisParams,
        _: extension_session::CancelAnalysisResults,
    ) -> impl Future<Output = Result<(), capnp::Error>> + 'static {
        std::future::ready((|| {
            self.source(false)?;
            self.daemon
                .0
                .analyses
                .cancel(&text(params.get()?.get_file_id()));
            Ok(())
        })())
    }

    fn save_skill(
        self: capnp::capability::Rc<Self>,
        params: extension_session::SaveSkillParams,
        mut results: extension_session::SaveSkillResults,
    ) -> impl Future<Output = Result<(), capnp::Error>> + 'static {
        std::future::ready((|| {
            self.source(false)?;
            let skill = parse_json(&text(params.get()?.get_skill()));
            let result = outcome(skills::save_skill(&skill));
            self.daemon.share_skill_catalog();
            reply!(results, result)
        })())
    }

    fn delete_skill(
        self: capnp::capability::Rc<Self>,
        params: extension_session::DeleteSkillParams,
        mut results: extension_session::DeleteSkillResults,
    ) -> impl Future<Output = Result<(), capnp::Error>> + 'static {
        std::future::ready((|| {
            self.source(false)?;
            let result = outcome(skills::delete_skill(&text(params.get()?.get_name())));
            self.daemon.share_skill_catalog();
            reply!(results, result)
        })())
    }

    fn delete_site_map(
        self: capnp::capability::Rc<Self>,
        params: extension_session::DeleteSiteMapParams,
        mut results: extension_session::DeleteSiteMapResults,
    ) -> impl Future<Output = Result<(), capnp::Error>> + 'static {
        std::future::ready((|| {
            self.source(false)?;
            let result = outcome(skills::delete_site_map(&text(params.get()?.get_name())));
            self.daemon.share_skill_catalog();
            reply!(results, result)
        })())
    }

    fn activate_site_map(
        self: capnp::capability::Rc<Self>,
        params: extension_session::ActivateSiteMapParams,
        mut results: extension_session::ActivateSiteMapResults,
    ) -> impl Future<Output = Result<(), capnp::Error>> + 'static {
        std::future::ready((|| {
            self.source(false)?;
            let params = params.get()?;
            let result =
                agent::commit_staging(&text(params.get_staging_id()), params.get_exact_host())
                    .map(|data| serde_json::json!({"name": data["name"], "path": data["path"]}));
            self.daemon.share_skill_catalog();
            reply!(results, result)
        })())
    }

    fn discard_site_map(
        self: capnp::capability::Rc<Self>,
        params: extension_session::DiscardSiteMapParams,
        mut results: extension_session::DiscardSiteMapResults,
    ) -> impl Future<Output = Result<(), capnp::Error>> + 'static {
        std::future::ready((|| {
            self.source(false)?;
            let result = agent::discard_staging(&text(params.get()?.get_staging_id()));
            reply!(results, result)
        })())
    }

    fn name_session(
        self: capnp::capability::Rc<Self>,
        params: extension_session::NameSessionParams,
        mut results: extension_session::NameSessionResults,
    ) -> impl Future<Output = Result<(), capnp::Error>> + 'static {
        let started = (|| {
            self.source(false)?;
            let params = params.get()?;
            let messages: Vec<String> = params.get_messages()?.iter().map(text).collect();
            Ok::<_, capnp::Error>((optional(text(params.get_host())), messages))
        })();
        async move {
            let (host, messages) = started?;
            let result = agent::name_session(host, messages, &read_agent_config()).await;
            reply!(results, result)
        }
    }

    fn analyze_recording(
        self: capnp::capability::Rc<Self>,
        params: extension_session::AnalyzeRecordingParams,
        mut results: extension_session::AnalyzeRecordingResults,
    ) -> impl Future<Output = Result<(), capnp::Error>> + 'static {
        let started = (|| {
            self.source(false)?;
            Ok::<_, capnp::Error>(parse_json(&text(params.get()?.get_recording())))
        })();
        async move {
            let recording = started?;
            let result = agent::analyze_recording(recording, &read_agent_config()).await;
            reply!(results, result)
        }
    }

    fn agent_state(
        self: capnp::capability::Rc<Self>,
        params: extension_session::AgentStateParams,
        mut results: extension_session::AgentStateResults,
    ) -> impl Future<Output = Result<(), capnp::Error>> + 'static {
        let refresh = params
            .get()
            .map(|params| params.get_refresh())
            .unwrap_or(false);
        let source = self.source(false);
        async move {
            source?;
            let state = agent::agent_state(&read_agent_config(), refresh).await;
            reply!(results, Ok(state))
        }
    }

    fn set_agent(
        self: capnp::capability::Rc<Self>,
        params: extension_session::SetAgentParams,
        mut results: extension_session::SetAgentResults,
    ) -> impl Future<Output = Result<(), capnp::Error>> + 'static {
        let daemon = self.daemon.clone();
        let started = (|| {
            let link = self.source(false)?;
            Ok::<_, capnp::Error>((link, text(params.get()?.get_agent())))
        })();
        async move {
            let (link, kind) = started?;
            if !agents::is_agent_kind(&kind) {
                return reply!(results, unknown_agent(&kind));
            }
            config::write_active_agent(&kind);
            daemon.reset_conversations();
            log::log(&format!("agent set to {}", agents::agent_label(&kind)));
            let state = agent::agent_state(&read_agent_config(), true).await;
            daemon.push_skill_catalog(&link);
            daemon.config_changed(Some(link.serial));
            reply!(results, Ok(state))
        }
    }

    fn set_agent_model(
        self: capnp::capability::Rc<Self>,
        params: extension_session::SetAgentModelParams,
        mut results: extension_session::SetAgentModelResults,
    ) -> impl Future<Output = Result<(), capnp::Error>> + 'static {
        let daemon = self.daemon.clone();
        let started = (|| {
            let link = self.source(false)?;
            let params = params.get()?;
            Ok::<_, capnp::Error>((
                link,
                text(params.get_agent()),
                optional(text(params.get_model())),
            ))
        })();
        async move {
            let (link, kind, model) = started?;
            if !agents::is_agent_kind(&kind) {
                return reply!(results, unknown_agent(&kind));
            }
            if !config::write_agent_model(&kind, model.as_deref()) {
                return reply!(
                    results,
                    failure(
                        "INVALID_INPUT",
                        format!(
                            "\"{}\" is not a model id: it has to start with a letter or digit and hold no spaces.",
                            model.as_deref().unwrap_or("null")
                        )
                    )
                );
            }
            log::log(&format!(
                "{} model set to {}",
                agents::agent_label(&kind),
                model
                    .as_deref()
                    .map(str::trim)
                    .filter(|m| !m.is_empty())
                    .unwrap_or("the default")
            ));
            let state = agent::agent_state(&read_agent_config(), true).await;
            daemon.config_changed(Some(link.serial));
            reply!(results, Ok(state))
        }
    }

    fn grant_agent(
        self: capnp::capability::Rc<Self>,
        params: extension_session::GrantAgentParams,
        mut results: extension_session::GrantAgentResults,
    ) -> impl Future<Output = Result<(), capnp::Error>> + 'static {
        let daemon = self.daemon.clone();
        let started = (|| {
            let link = self.source(false)?;
            Ok::<_, capnp::Error>((link, text(params.get()?.get_agent())))
        })();
        async move {
            let (link, kind) = started?;
            if !agents::is_agent_kind(&kind) {
                return reply!(results, unknown_agent(&kind));
            }
            let state = agent::agent_state(&read_agent_config(), true).await;
            daemon.announce_agent(Some(link.serial));
            reply!(results, Ok(state))
        }
    }

    fn list_skills(
        self: capnp::capability::Rc<Self>,
        params: extension_session::ListSkillsParams,
        mut results: extension_session::ListSkillsResults,
    ) -> impl Future<Output = Result<(), capnp::Error>> + 'static {
        std::future::ready((|| {
            self.source(false)?;
            let refresh = params.get()?.get_refresh();
            reply!(results, self.daemon.skill_catalog_now(refresh))
        })())
    }

    fn preferences(
        self: capnp::capability::Rc<Self>,
        _: extension_session::PreferencesParams,
        mut results: extension_session::PreferencesResults,
    ) -> impl Future<Output = Result<(), capnp::Error>> + 'static {
        std::future::ready((|| {
            self.source(false)?;
            reply!(results, Ok(preferences_now()))
        })())
    }

    fn set_preference(
        self: capnp::capability::Rc<Self>,
        params: extension_session::SetPreferenceParams,
        mut results: extension_session::SetPreferenceResults,
    ) -> impl Future<Output = Result<(), capnp::Error>> + 'static {
        std::future::ready((|| {
            let link = self.source(false)?;
            let change = parse_json(&text(params.get()?.get_change()));
            let result = outcome(apply_preference(&change));
            self.daemon.config_changed(Some(link.serial));
            reply!(results, result)
        })())
    }

    fn approvals(
        self: capnp::capability::Rc<Self>,
        _: extension_session::ApprovalsParams,
        mut results: extension_session::ApprovalsResults,
    ) -> impl Future<Output = Result<(), capnp::Error>> + 'static {
        std::future::ready((|| {
            self.source(false)?;
            reply!(results, Ok(json!({ "grants": approvals::list_grants() })))
        })())
    }

    fn forget_approvals(
        self: capnp::capability::Rc<Self>,
        params: extension_session::ForgetApprovalsParams,
        mut results: extension_session::ForgetApprovalsResults,
    ) -> impl Future<Output = Result<(), capnp::Error>> + 'static {
        std::future::ready((|| {
            self.source(false)?;
            let host = text(params.get()?.get_host());
            // An empty host forgets every site's.
            let removed = approvals::forget_grants((!host.is_empty()).then_some(host.as_str()));
            reply!(results, Ok(json!({ "removed": removed })))
        })())
    }

    fn tasks(
        self: capnp::capability::Rc<Self>,
        _: extension_session::TasksParams,
        mut results: extension_session::TasksResults,
    ) -> impl Future<Output = Result<(), capnp::Error>> + 'static {
        std::future::ready((|| {
            self.source(false)?;
            reply!(results, Ok(self.daemon.0.scheduler.list()))
        })())
    }

    fn save_task(
        self: capnp::capability::Rc<Self>,
        params: extension_session::SaveTaskParams,
        mut results: extension_session::SaveTaskResults,
    ) -> impl Future<Output = Result<(), capnp::Error>> + 'static {
        std::future::ready((|| {
            self.source(false)?;
            let task = parse_json(&text(params.get()?.get_task()));
            reply!(results, self.daemon.0.scheduler.save(&task))
        })())
    }

    fn delete_task(
        self: capnp::capability::Rc<Self>,
        params: extension_session::DeleteTaskParams,
        mut results: extension_session::DeleteTaskResults,
    ) -> impl Future<Output = Result<(), capnp::Error>> + 'static {
        std::future::ready((|| {
            self.source(false)?;
            let task_id = text(params.get()?.get_task_id());
            reply!(results, self.daemon.0.scheduler.remove(&task_id))
        })())
    }

    fn run_task_now(
        self: capnp::capability::Rc<Self>,
        params: extension_session::RunTaskNowParams,
        mut results: extension_session::RunTaskNowResults,
    ) -> impl Future<Output = Result<(), capnp::Error>> + 'static {
        std::future::ready((|| {
            let link = self.source(false)?;
            let task_id = text(params.get()?.get_task_id());
            reply!(results, self.daemon.0.scheduler.run_now(&task_id, link))
        })())
    }

    fn pause_tasks(
        self: capnp::capability::Rc<Self>,
        params: extension_session::PauseTasksParams,
        mut results: extension_session::PauseTasksResults,
    ) -> impl Future<Output = Result<(), capnp::Error>> + 'static {
        std::future::ready((|| {
            self.source(false)?;
            let paused = params.get()?.get_paused();
            reply!(results, self.daemon.0.scheduler.pause(paused))
        })())
    }

    fn task_done(
        self: capnp::capability::Rc<Self>,
        params: extension_session::TaskDoneParams,
        _: extension_session::TaskDoneResults,
    ) -> impl Future<Output = Result<(), capnp::Error>> + 'static {
        std::future::ready((|| {
            self.source(false)?;
            let params = params.get()?;
            let result = parse_json(&text(params.get_result()));
            self.daemon
                .0
                .scheduler
                .finished(&text(params.get_task_id()), &result);
            Ok(())
        })())
    }

    fn record_frame(
        self: capnp::capability::Rc<Self>,
        params: extension_session::RecordFrameParams,
        _: extension_session::RecordFrameResults,
    ) -> impl Future<Output = Result<(), capnp::Error>> + 'static {
        std::future::ready((|| {
            self.source(false)?;
            let params = params.get()?;
            self.daemon.0.video.borrow_mut().frame(
                &text(params.get_recording_id()),
                params.get_frame()?,
                params.get_at_ms(),
            );
            Ok(())
        })())
    }

    fn notice(
        self: capnp::capability::Rc<Self>,
        params: extension_session::NoticeParams,
        mut results: extension_session::NoticeResults,
    ) -> impl Future<Output = Result<(), capnp::Error>> + 'static {
        std::future::ready((|| {
            // A notice is sent because nobody is looking at this browser: not a sign of where the user is.
            let link = self.source(true)?;
            let raw = parse_json(&text(params.get()?.get_notice()));
            let delivered = match as_device_notice(&raw) {
                Some(notice) => {
                    let id = notice.id.clone();
                    let delivered = self.daemon.0.notices.borrow_mut().offer(
                        notice,
                        &link.install_id,
                        Some(link.label().to_owned()),
                        log::now_ms(),
                    );
                    if delivered {
                        self.daemon
                            .notify(vexdom_schema::Topic::NoticesChanged, &Value::Null);
                    }
                    log::log(&format!(
                        "notice {id} from {}: {}",
                        link.label(),
                        if delivered {
                            "handed to the app"
                        } else {
                            "no app to show it"
                        }
                    ));
                    delivered
                }
                None => false,
            };
            results.get().set_delivered(delivered);
            Ok(())
        })())
    }
}
