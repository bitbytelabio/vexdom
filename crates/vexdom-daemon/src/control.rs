//! The control socket (`daemon.ts` acceptControl): the CLI, the MCP server, the native host and
//! the desktop app call `Control`. A connection that has spoken for an agent run (a `describe` or
//! `invoke` with a run id) is that run's tool server, and is refused anything that would loosen the
//! guardrails it runs under.

use std::cell::{Cell, RefCell};
use std::future::Future;
use std::rc::Rc;

use futures::{AsyncRead, AsyncWrite};
use serde_json::{Value, json};
use vexdom_core::config::{self, read_agent_config};
use vexdom_core::pairing::{Asker, is_pairing_mode};
use vexdom_core::preferences::{apply_preference, preferences_now};
use vexdom_core::proto::{
    MAX_PAIR_WAIT_MS, NOTICE_PROTOCOL, SOCKET_PROTOCOL_VERSION, is_install_id,
};
use vexdom_core::result::{ActionError, failure};
use vexdom_core::{agents, auth_store, log, tools};
use vexdom_schema::{Topic, control, handle};

use crate::confirmations::Answer;
use crate::link::Reply;
use crate::wire::{json_text, optional, parse_json, text, write_result};
use crate::{Binding, Daemon, Watcher, unknown_action};

pub(crate) fn accept<T>(daemon: &Daemon, stream: T)
where
    T: AsyncRead + AsyncWrite + Unpin + 'static,
{
    let id = daemon.next_serial();
    let conn = Rc::new(ControlConn {
        daemon: daemon.clone(),
        id,
        label: format!("c{id}"),
        binding: RefCell::new(Binding::default()),
        speaks_for_run: Cell::new(false),
    });
    let client: control::Client = capnp_rpc::new_client_from_rc(Rc::clone(&conn));
    let system = vexdom_link::serve(stream, client.client);
    daemon.0.controls.set(daemon.0.controls.get() + 1);
    daemon.activity_changed();
    let daemon = daemon.clone();
    tokio::task::spawn_local(async move {
        let _ = system.await;
        daemon
            .0
            .controls
            .set(daemon.0.controls.get().saturating_sub(1));
        let ids: Vec<u64> = daemon
            .0
            .watchers
            .borrow()
            .iter()
            .filter(|watcher| watcher.conn == id)
            .map(|watcher| watcher.id)
            .collect();
        for watcher in ids {
            daemon.remove_watcher(watcher);
        }
        daemon.activity_changed();
    });
}

pub(crate) struct ControlConn {
    daemon: Daemon,
    id: u64,
    label: String,
    binding: RefCell<Binding>,
    speaks_for_run: Cell<bool>,
}

/// Dropping the handle (or the connection) ends the subscription.
struct WatchHandle {
    daemon: Daemon,
    id: u64,
}

impl handle::Server for WatchHandle {}

impl Drop for WatchHandle {
    fn drop(&mut self) {
        self.daemon.remove_watcher(self.id);
    }
}

impl ControlConn {
    fn run_refusal(&self, what: &str) {
        log::log(&format!(
            "control {} tried to {what} from inside an agent run; refused",
            self.label
        ));
    }
}

impl control::Server for ControlConn {
    fn status(
        self: capnp::capability::Rc<Self>,
        _: control::StatusParams,
        mut results: control::StatusResults,
    ) -> impl Future<Output = Result<(), capnp::Error>> + 'static {
        let daemon = &self.daemon;
        let target = daemon.route_for(&self.binding);
        let mut status = results.get().init_status();
        status.set_connected(target.is_some());
        status.set_daemon_version(crate::VERSION);
        status.set_protocol_version(SOCKET_PROTOCOL_VERSION);
        status.set_port(daemon.port());
        status.set_manifest_in_sync(target.as_ref().is_none_or(|link| daemon.in_sync(link)));
        if let Some(link) = &target {
            status.set_extension_version(&link.extension_version);
            status.set_browser(link.browser.as_deref().unwrap_or(""));
        }
        status.set_connected_browsers(daemon.open_links().len() as u32);
        status.set_paired_browsers(auth_store::list_sessions().len() as u32);
        status.set_pairing_pending(auth_store::has_pending_pairing());
        let mut pairing = status.init_pairing();
        pairing.set_mode(config::pairing_mode(&read_agent_config()));
        pairing.set_approver(daemon.approver_now());
        std::future::ready(Ok(()))
    }

    fn describe(
        self: capnp::capability::Rc<Self>,
        params: control::DescribeParams,
        mut results: control::DescribeResults,
    ) -> impl Future<Output = Result<(), capnp::Error>> + 'static {
        std::future::ready((|| {
            let run_id = optional(text(params.get()?.get_run_id()));
            if run_id.is_some() {
                self.speaks_for_run.set(true);
            }
            let described = self.daemon.describe_for(run_id.as_deref(), &self.binding);
            let mut out = results.get();
            out.set_tools(json_text(&Value::Array(described.tools)));
            fill(
                out.reborrow()
                    .init_reserved(described.reserved.len() as u32),
                &described.reserved,
            );
            let withheld: Vec<String> = described
                .withheld
                .iter()
                .filter_map(|tool| tool.get("name").and_then(Value::as_str).map(str::to_owned))
                .collect();
            fill(
                out.reborrow().init_withheld(withheld.len() as u32),
                &withheld,
            );
            Ok(())
        })())
    }

    fn invoke(
        self: capnp::capability::Rc<Self>,
        params: control::InvokeParams,
        mut results: control::InvokeResults,
    ) -> impl Future<Output = Result<(), capnp::Error>> + 'static {
        let conn = Rc::clone(&self);
        let started = (|| {
            let params = params.get()?;
            Ok::<_, capnp::Error>((
                text(params.get_action()),
                parse_json(&text(params.get_input())),
                optional(text(params.get_run_id())),
            ))
        })();
        async move {
            let (action, mut input, run_id) = started?;
            let delta = !crate::page::take_no_delta(&mut input);
            let reply: Reply = if let Some(run_id) = run_id {
                conn.speaks_for_run.set(true);
                // A run's approvals go to the side panel; an approver tag means nothing here.
                crate::confirmations::take_approver(&mut input);
                match conn.daemon.session_running(&run_id) {
                    Some((_, session)) => session.invoke_for_run(run_id, action, input).await,
                    None => {
                        log::log(&format!(
                            "control {} invoked {action} for inactive run {run_id}",
                            conn.label
                        ));
                        failure("RUN_INACTIVE", "This agent run is no longer active").into()
                    }
                }
            } else if tools::DAEMON_INTERNAL_ACTIONS.contains(&action.as_str()) {
                unknown_action(&action).into()
            } else {
                let target = conn.daemon.route_for(&conn.binding);
                conn.daemon
                    .page_invoke(target, &action, input, &conn.label, delta)
                    .await
            };
            let mut out = results.get();
            write_result(out.reborrow().init_result(), &reply.result);
            if let Some(image) = &reply.image {
                out.set_image(image);
            }
            Ok(())
        }
    }

    fn pair(
        self: capnp::capability::Rc<Self>,
        _: control::PairParams,
        mut results: control::PairResults,
    ) -> impl Future<Output = Result<(), capnp::Error>> + 'static {
        let (code, expires_at) = auth_store::create_pairing();
        log::log("minted a pairing code");
        results.get().set_code(&code);
        results.get().set_expires_at(expires_at);
        std::future::ready(Ok(()))
    }

    fn sessions(
        self: capnp::capability::Rc<Self>,
        _: control::SessionsParams,
        mut results: control::SessionsResults,
    ) -> impl Future<Output = Result<(), capnp::Error>> + 'static {
        let summaries = self.daemon.session_summaries();
        let mut list = results.get().init_sessions(summaries.len() as u32);
        for (at, summary) in summaries.iter().enumerate() {
            let mut out = list.reborrow().get(at as u32);
            let field = |key: &str| {
                summary
                    .get(key)
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_owned()
            };
            out.set_id(field("id"));
            out.set_browser(field("browser"));
            out.set_origin(field("origin"));
            out.set_source(field("source"));
            out.set_extension_version(field("extensionVersion"));
            out.set_paired_at(field("pairedAt"));
            out.set_last_seen_at(field("lastSeenAt"));
            out.set_connected(summary.get("connected") == Some(&Value::Bool(true)));
            out.set_label(field("label"));
            out.set_paired_by(field("pairedBy"));
        }
        std::future::ready(Ok(()))
    }

    fn agent(
        self: capnp::capability::Rc<Self>,
        params: control::AgentParams,
        mut results: control::AgentResults,
    ) -> impl Future<Output = Result<(), capnp::Error>> + 'static {
        let conn = Rc::clone(&self);
        let started = (|| {
            let params = params.get()?;
            Ok::<_, capnp::Error>((
                optional(text(params.get_set())),
                optional(text(params.get_grant())),
                optional(text(params.get_models())),
            ))
        })();
        async move {
            let (set, grant, models) = started?;
            let changed = set.clone().or_else(|| grant.clone());
            let state = if changed
                .as_deref()
                .is_some_and(|kind| !agents::is_agent_kind(kind))
            {
                crate::agent::agent_state(&read_agent_config(), false).await
            } else {
                if let Some(kind) = &set {
                    config::write_active_agent(kind);
                    conn.daemon.reset_conversations();
                    log::log(&format!(
                        "control {} set the agent to {}",
                        conn.label,
                        agents::agent_label(kind)
                    ));
                }
                let listed = models.as_deref().is_some_and(agents::is_agent_kind);
                let state =
                    crate::agent::agent_state(&read_agent_config(), changed.is_some() || listed)
                        .await;
                if set.is_some() {
                    conn.daemon.config_changed(None);
                }
                if grant.is_some() || listed {
                    conn.daemon.announce_agent(None);
                }
                state
            };
            results.get().set_state(json_text(&state));
            Ok(())
        }
    }

    fn preferences(
        self: capnp::capability::Rc<Self>,
        _: control::PreferencesParams,
        mut results: control::PreferencesResults,
    ) -> impl Future<Output = Result<(), capnp::Error>> + 'static {
        write_result(results.get().init_result(), &Ok(preferences_now()));
        std::future::ready(Ok(()))
    }

    fn set_preference(
        self: capnp::capability::Rc<Self>,
        params: control::SetPreferenceParams,
        mut results: control::SetPreferenceResults,
    ) -> impl Future<Output = Result<(), capnp::Error>> + 'static {
        std::future::ready((|| {
            if self.speaks_for_run.get() {
                self.run_refusal("change a setting");
                write_result(
                    results.get().init_result(),
                    &failure(
                        "BLOCKED",
                        "An agent run cannot change the settings it runs under. Change them from the settings page or the desktop app.",
                    ),
                );
                return Ok(());
            }
            let change = parse_json(&text(params.get()?.get_change()));
            let result = apply_preference(&change).map_err(ActionError::from);
            write_result(results.get().init_result(), &result);
            self.daemon.config_changed(None);
            Ok(())
        })())
    }

    fn grant(
        self: capnp::capability::Rc<Self>,
        params: control::GrantParams,
        mut results: control::GrantResults,
    ) -> impl Future<Output = Result<(), capnp::Error>> + 'static {
        let conn = Rc::clone(&self);
        let started = (|| {
            let params = params.get()?;
            Ok::<_, capnp::Error>((
                text(params.get_install_id()),
                text(params.get_caller()),
                text(params.get_browser()),
                text(params.get_extension_version()),
                params.get_gesture(),
                params.get_wait_ms(),
            ))
        })();
        async move {
            let (install_id, caller, browser, extension_version, gesture, wait_ms) = started?;
            let outcome = if conn.speaks_for_run.get() {
                conn.run_refusal("ask for a pairing grant");
                json!({"refused": "off", "error": "An agent run cannot pair a browser."})
            } else if !is_install_id(&install_id) {
                json!({"refused": "off", "error": "That grant request names no browser."})
            } else {
                let printable: String = browser
                    .chars()
                    .filter(|c| (' '..='~').contains(c))
                    .collect();
                let label: String = printable.trim().chars().take(40).collect();
                let asker = Asker {
                    install_id: install_id.clone(),
                    caller,
                    browser: (!label.is_empty()).then_some(label),
                    extension_version: if extension_version.is_empty() {
                        "unknown".into()
                    } else {
                        extension_version.chars().take(32).collect()
                    },
                    gesture,
                };
                let first = conn.daemon.0.desk.borrow_mut().ask(&asker);
                let wait = u64::from(wait_ms.min(MAX_PAIR_WAIT_MS));
                let waiting = matches!(first, vexdom_core::pairing::GrantOutcome::Wait { .. });
                let outcome = if waiting && wait > 0 {
                    conn.daemon.settled(&install_id, wait).await;
                    conn.daemon.0.desk.borrow_mut().ask(&Asker {
                        gesture: false,
                        ..asker
                    })
                } else {
                    first
                };
                let mut value = serde_json::to_value(&outcome).unwrap_or(Value::Null);
                // The port is where the extension connects next.
                if let Some(grant) = value.get_mut("grant") {
                    grant["port"] = json!(conn.daemon.port());
                }
                value
            };
            results.get().set_outcome(json_text(&outcome));
            Ok(())
        }
    }

    fn pair_requests(
        self: capnp::capability::Rc<Self>,
        _: control::PairRequestsParams,
        mut results: control::PairRequestsResults,
    ) -> impl Future<Output = Result<(), capnp::Error>> + 'static {
        let pending = self.daemon.0.desk.borrow_mut().pending();
        results.get().set_requests(json_text(
            &serde_json::to_value(pending).unwrap_or(Value::Null),
        ));
        std::future::ready(Ok(()))
    }

    fn decide(
        self: capnp::capability::Rc<Self>,
        params: control::DecideParams,
        mut results: control::DecideResults,
    ) -> impl Future<Output = Result<(), capnp::Error>> + 'static {
        std::future::ready((|| {
            let mut out = results.get();
            if self.speaks_for_run.get() {
                self.run_refusal("decide a pairing request");
                out.set_ok(false);
                out.set_error("An agent run cannot let a browser in.");
                return Ok(());
            }
            let params = params.get()?;
            let request = text(params.get_request());
            let label = optional(text(params.get_label()));
            let ok = !request.is_empty()
                && self.daemon.0.desk.borrow_mut().decide(
                    &request,
                    params.get_allow(),
                    label.as_deref(),
                );
            out.set_ok(ok);
            if !ok {
                out.set_error("No request or blocked browser has that id.");
            }
            Ok(())
        })())
    }

    fn pairing(
        self: capnp::capability::Rc<Self>,
        params: control::PairingParams,
        mut results: control::PairingResults,
    ) -> impl Future<Output = Result<(), capnp::Error>> + 'static {
        std::future::ready((|| {
            let set = optional(text(params.get()?.get_set()));
            let mut error = None;
            if let Some(mode) = set {
                if self.speaks_for_run.get() {
                    self.run_refusal("change how browsers pair");
                    error = Some("An agent run cannot change how browsers pair.");
                } else if !is_pairing_mode(&mode) {
                    error = Some("Pairing is auto, ask or code.");
                } else {
                    config::write_pairing_mode(&mode);
                    log::log(&format!("control {} set pairing to {mode}", self.label));
                    self.daemon.config_changed(None);
                }
            }
            let mut out = results.get();
            out.set_mode(config::pairing_mode(&read_agent_config()));
            out.set_approver(self.daemon.approver_now());
            out.set_error(error.unwrap_or(""));
            Ok(())
        })())
    }

    fn blocked(
        self: capnp::capability::Rc<Self>,
        _: control::BlockedParams,
        mut results: control::BlockedResults,
    ) -> impl Future<Output = Result<(), capnp::Error>> + 'static {
        let blocked = serde_json::to_value(auth_store::list_blocked()).unwrap_or(Value::Null);
        results.get().set_blocked(json_text(&blocked));
        std::future::ready(Ok(()))
    }

    fn label(
        self: capnp::capability::Rc<Self>,
        params: control::LabelParams,
        mut results: control::LabelResults,
    ) -> impl Future<Output = Result<(), capnp::Error>> + 'static {
        std::future::ready((|| {
            let mut out = results.get();
            if self.speaks_for_run.get() {
                out.set_ok(false);
                out.set_error("An agent run cannot rename a browser.");
                return Ok(());
            }
            let params = params.get()?;
            let ok =
                auth_store::label_session(&text(params.get_session()), &text(params.get_label()));
            out.set_ok(ok);
            Ok(())
        })())
    }

    fn take_notices(
        self: capnp::capability::Rc<Self>,
        _: control::TakeNoticesParams,
        mut results: control::TakeNoticesResults,
    ) -> impl Future<Output = Result<(), capnp::Error>> + 'static {
        let notices = if self.speaks_for_run.get() {
            self.run_refusal("handle notices");
            Vec::new()
        } else {
            let watcher = self
                .daemon
                .0
                .watchers
                .borrow()
                .iter()
                .find(|watcher| watcher.conn == self.id && watcher.topic == Topic::NoticesChanged)
                .map(|watcher| watcher.id);
            watcher.map_or_else(Vec::new, |id| {
                self.daemon
                    .0
                    .notices
                    .borrow_mut()
                    .take(id, vexdom_core::log::now_ms())
            })
        };
        results.get().set_notices(json_text(
            &serde_json::to_value(notices).unwrap_or(Value::Null),
        ));
        std::future::ready(Ok(()))
    }

    fn open_notice(
        self: capnp::capability::Rc<Self>,
        params: control::OpenNoticeParams,
        mut results: control::OpenNoticeResults,
    ) -> impl Future<Output = Result<(), capnp::Error>> + 'static {
        std::future::ready((|| {
            if self.speaks_for_run.get() {
                self.run_refusal("handle notices");
                results.get().set_ok(false);
                return Ok(());
            }
            let notice = text(params.get()?.get_notice());
            let route = self
                .daemon
                .0
                .notices
                .borrow()
                .route_of(&notice)
                .map(str::to_owned);
            let link = route.and_then(|route| self.daemon.link(&route));
            let ok = link
                .as_ref()
                .is_some_and(|link| link.is_open() && link.speaks(NOTICE_PROTOCOL));
            if let Some(link) = link.filter(|_| ok) {
                link.open_notice(&notice);
            }
            results.get().set_ok(ok);
            Ok(())
        })())
    }

    fn revoke(
        self: capnp::capability::Rc<Self>,
        params: control::RevokeParams,
        mut results: control::RevokeResults,
    ) -> impl Future<Output = Result<(), capnp::Error>> + 'static {
        std::future::ready((|| {
            let params = params.get()?;
            let id = optional(text(params.get_session()));
            let origin = optional(text(params.get_origin()));
            let matches = |session_id: &str, session_origin: &str| match (&id, &origin) {
                (Some(id), _) => session_id == id,
                (None, Some(origin)) => session_origin == origin,
                (None, None) => true,
            };
            let revoked =
                auth_store::revoke_sessions(|session| matches(session.id(), &session.origin));
            for link in self.daemon.open_links() {
                if matches(&link.install_id, &link.origin) {
                    link.close("pairing revoked");
                }
            }
            log::log(&format!("revoked {revoked} session(s)"));
            results.get().set_revoked(revoked as u32);
            Ok(())
        })())
    }

    fn watch(
        self: capnp::capability::Rc<Self>,
        params: control::WatchParams,
        mut results: control::WatchResults,
    ) -> impl Future<Output = Result<(), capnp::Error>> + 'static {
        std::future::ready((|| {
            let params = params.get()?;
            let topic = params
                .get_topic()
                .map_err(|_| capnp::Error::failed("unknown topic".into()))?;
            if topic == Topic::NoticesChanged && self.speaks_for_run.get() {
                self.run_refusal("handle notices");
                return Err(capnp::Error::failed(
                    "An agent run cannot handle notices.".into(),
                ));
            }
            if topic == Topic::ConfirmationsChanged && self.speaks_for_run.get() {
                self.run_refusal("watch confirmations");
                return Err(capnp::Error::failed(
                    "An agent run cannot watch approvals.".into(),
                ));
            }
            let approver = optional(text(params.get_approver())).map(|approver| {
                if approver == "terminal" {
                    approver
                } else {
                    "app".to_owned()
                }
            });
            let id = self.daemon.next_serial();
            self.daemon.add_watcher(
                Watcher {
                    id,
                    conn: self.id,
                    topic,
                    approver: if topic == Topic::PairRequestsChanged {
                        Some(approver.unwrap_or_else(|| "app".into()))
                    } else {
                        None
                    },
                    sink: params.get_sink()?,
                },
                params.get_allowed(),
            );
            let handle: handle::Client = capnp_rpc::new_client(WatchHandle {
                daemon: self.daemon.clone(),
                id,
            });
            results.get().set_handle(handle);
            Ok(())
        })())
    }

    fn snapshot(
        self: capnp::capability::Rc<Self>,
        params: control::SnapshotParams,
        mut results: control::SnapshotResults,
    ) -> impl Future<Output = Result<(), capnp::Error>> + 'static {
        let conn = Rc::clone(&self);
        let started = params
            .get()
            .and_then(|params| crate::page::SnapshotRequest::read(params.get_opts()?));
        async move {
            let request = started?;
            if request.run_id.is_some() {
                conn.speaks_for_run.set(true);
            }
            let target = conn.daemon.route_for(&conn.binding);
            let (text, webmcp) = conn
                .daemon
                .snapshot_announced(target, request)
                .await
                .map_err(page_error)?;
            let mut results = results.get();
            results.set_text(&text);
            if let Some(webmcp) = webmcp {
                results.set_webmcp(webmcp.to_string());
            }
            Ok(())
        }
    }

    fn read(
        self: capnp::capability::Rc<Self>,
        params: control::ReadParams,
        mut results: control::ReadResults,
    ) -> impl Future<Output = Result<(), capnp::Error>> + 'static {
        let conn = Rc::clone(&self);
        let started = params
            .get()
            .and_then(|params| crate::page::ReadRequest::read(params.get_opts()?));
        async move {
            let request = started?;
            if request.run_id.is_some() {
                conn.speaks_for_run.set(true);
            }
            let target = conn.daemon.route_for(&conn.binding);
            let text = conn
                .daemon
                .read(target, request)
                .await
                .map_err(page_error)?;
            results.get().set_text(&text);
            Ok(())
        }
    }

    fn stop(
        self: capnp::capability::Rc<Self>,
        _: control::StopParams,
        _: control::StopResults,
    ) -> impl Future<Output = Result<(), capnp::Error>> + 'static {
        if self.speaks_for_run.get() {
            self.run_refusal("stop the Bridge");
            return std::future::ready(Err(capnp::Error::failed(
                "An agent run cannot stop Vexdom Bridge.".into(),
            )));
        }
        log::log(&format!("control {} asked the Bridge to stop", self.label));
        let daemon = self.daemon.clone();
        tokio::task::spawn_local(async move {
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            daemon.stop();
        });
        std::future::ready(Ok(()))
    }

    fn confirmations(
        self: capnp::capability::Rc<Self>,
        _: control::ConfirmationsParams,
        mut results: control::ConfirmationsResults,
    ) -> impl Future<Output = Result<(), capnp::Error>> + 'static {
        let list = if self.speaks_for_run.get() {
            self.run_refusal("read confirmations");
            Value::Array(Vec::new())
        } else {
            self.daemon.confirmations_list()
        };
        results.get().set_list(json_text(&list));
        std::future::ready(Ok(()))
    }

    fn confirm(
        self: capnp::capability::Rc<Self>,
        params: control::ConfirmParams,
        mut results: control::ConfirmResults,
    ) -> impl Future<Output = Result<(), capnp::Error>> + 'static {
        std::future::ready((|| {
            // An agent never answers its own approvals.
            if self.speaks_for_run.get() {
                self.run_refusal("answer a confirmation");
                return Err(capnp::Error::failed(
                    "An agent run cannot answer an approval.".into(),
                ));
            }
            let params = params.get()?;
            let id = text(params.get_id());
            let answer = if params.get_allow() {
                Answer::Allow {
                    remember: params.get_remember(),
                }
            } else {
                Answer::Deny
            };
            results.get().set_found(self.daemon.confirm(&id, answer));
            Ok(())
        })())
    }
}

/// A page failure on `snapshot`/`read` travels as an exception `CODE: message`.
fn page_error(error: ActionError) -> capnp::Error {
    capnp::Error::failed(format!("{}: {}", error.code, error.message))
}

fn fill(mut list: capnp::text_list::Builder<'_>, items: &[String]) {
    for (at, item) in items.iter().enumerate() {
        list.set(at as u32, item);
    }
}
