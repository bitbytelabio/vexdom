//! A control-socket client for the CLI, the MCP server, the native host and the desktop app: finds
//! (or starts) the Bridge, then speaks `Control` over capnp. Every call here runs on the current
//! `LocalSet`.

use std::time::Duration;

use serde_json::Value;
use vexdom_core::result::{ActionError, ActionResult};
use vexdom_schema::{Topic, action_result, control, event_sink};

pub struct Bridge {
    pub control: control::Client,
    run_id: Option<String>,
    _connection: tokio::task::JoinHandle<()>,
}

/// What `Control.invoke` answers.
pub struct Invoked {
    pub result: ActionResult,
    pub image: Option<Vec<u8>>,
}

pub fn read_result(reader: capnp::Result<action_result::Reader<'_>>) -> ActionResult {
    vexdom_daemon::wire::read_result(reader)
}

fn text(reader: capnp::Result<capnp::text::Reader<'_>>) -> String {
    vexdom_daemon::wire::text(reader)
}

pub fn unreachable() -> ActionError {
    ActionError {
        code: "DAEMON_UNREACHABLE".into(),
        message: "The Vexdom daemon did not respond".into(),
    }
}

/// How long a caller outside any run waits for an action (`remote-bridge.ts`).
pub fn invoke_timeout(action: &str, input: &Value) -> Duration {
    let declared = input
        .get("timeoutMs")
        .and_then(Value::as_f64)
        .filter(|ms| *ms > 0.0)
        .map(|ms| ms as u64);
    let ms = match action {
        "page.startMonitor" => 60_000,
        "page.awaitMonitor" => declared.unwrap_or(120_000) + 10_000,
        "page.pickElement" => declared.unwrap_or(60_000) + 10_000,
        "page.solveCaptcha" => declared.unwrap_or(120_000) + 10_000,
        "page.screenshot" => 130_000,
        // A login waits for its fields, then for the page after submitting.
        "bridge.authLogin" => declared.unwrap_or(25_000) * 4 + 10_000,
        "bridge.flowRun" => declared.unwrap_or(60_000) * 2 + 10_000,
        _ => declared.map_or(60_000, |ms| ms + 10_000),
    };
    // A tagged caller can wait on the user's approval first (ADR-032).
    let approval = if input.get("_approver").is_some() {
        vexdom_core::proto::CONFIRM_WAIT_MS
    } else {
        0
    };
    Duration::from_millis(ms + approval)
}

impl Bridge {
    /// Ensures a Bridge is running, then connects. `run_id` makes this connection an agent run's.
    pub async fn connect(run_id: Option<String>) -> Result<Self, String> {
        let lock = vexdom_daemon::ensure::ensure_daemon().await?;
        Self::connect_to(lock.port, &lock.token, run_id).await
    }

    /// Connects to a Bridge already known to run, without starting one.
    pub async fn existing(run_id: Option<String>) -> Result<Option<Self>, String> {
        match vexdom_daemon::ensure::probe_existing().await {
            Some(lock) => Self::connect_to(lock.port, &lock.token, run_id)
                .await
                .map(Some),
            None => Ok(None),
        }
    }

    pub async fn connect_to(
        port: u16,
        token: &str,
        run_id: Option<String>,
    ) -> Result<Self, String> {
        let (control, connection) = vexdom_link::connect_control(port, token)
            .await
            .map_err(|error| error.to_string())?;
        Ok(Self {
            control,
            run_id: run_id.filter(|id| !id.is_empty()),
            _connection: connection,
        })
    }

    pub fn run_id(&self) -> Option<&str> {
        self.run_id.as_deref()
    }

    /// `Control.invoke`. A run's calls wait as long as the daemon does (approvals); anything else
    /// gives up after the action's own timeout.
    pub async fn invoke(&self, action: &str, input: &Value) -> Invoked {
        let mut request = self.control.invoke_request();
        request.get().set_action(action);
        request.get().set_input(input.to_string());
        request
            .get()
            .set_run_id(self.run_id.as_deref().unwrap_or(""));
        let call = request.send().promise;
        let reply = if self.run_id.is_some() {
            call.await.ok()
        } else {
            tokio::time::timeout(invoke_timeout(action, input), call)
                .await
                .ok()
                .and_then(Result::ok)
        };
        let Some(reply) = reply else {
            return Invoked {
                result: Err(unreachable()),
                image: None,
            };
        };
        match reply.get() {
            Ok(reply) => Invoked {
                result: read_result(reply.get_result()),
                image: reply
                    .get_image()
                    .ok()
                    .filter(|image| !image.is_empty())
                    .map(<[u8]>::to_vec),
            },
            Err(_) => Invoked {
                result: Err(unreachable()),
                image: None,
            },
        }
    }

    /// `Control.describe`: the tools, the reserved names and the on-demand groups. `known` is the
    /// stamp of the caller's last answer (0 for none); when the offer still has it, `tools` is
    /// `None`.
    pub async fn describe(&self, known: u64) -> Result<Described, capnp::Error> {
        let mut request = self.control.describe_request();
        let mut params = request.get();
        params.set_run_id(self.run_id.as_deref().unwrap_or(""));
        params.set_known(known);
        let reply = request.send().promise.await?;
        let reply = reply.get()?;
        let list = |reader: capnp::Result<capnp::text_list::Reader<'_>>| -> Vec<String> {
            reader
                .map(|list| list.iter().map(text).collect())
                .unwrap_or_default()
        };
        let stamp = reply.get_stamp();
        let tools = (known == 0 || stamp != known).then(|| {
            serde_json::from_str::<Vec<Value>>(&text(reply.get_tools())).unwrap_or_default()
        });
        Ok(Described {
            stamp,
            tools,
            reserved: list(reply.get_reserved()),
            withheld: list(reply.get_withheld()),
        })
    }

    pub async fn status(&self) -> Result<Value, capnp::Error> {
        let reply = self.control.status_request().send().promise.await?;
        let status = reply.get()?.get_status()?;
        let mut out = serde_json::json!({
            "connected": status.get_connected(),
            "daemonVersion": text(status.get_daemon_version()),
            "protocolVersion": status.get_protocol_version(),
            "port": status.get_port(),
            "manifestInSync": status.get_manifest_in_sync(),
        });
        let extension = text(status.get_extension_version());
        if !extension.is_empty() {
            out["extensionVersion"] = Value::String(extension);
        }
        let browser = text(status.get_browser());
        if !browser.is_empty() {
            out["browser"] = Value::String(browser);
        }
        out["connectedBrowsers"] = status.get_connected_browsers().into();
        out["pairedBrowsers"] = status.get_paired_browsers().into();
        out["pairingPending"] = status.get_pairing_pending().into();
        let pairing = status.get_pairing();
        out["pairing"] = serde_json::json!({"mode": text(pairing.get_mode()), "approver": text(pairing.get_approver())});
        Ok(out)
    }

    /// `Control.confirmations`: the external actions waiting for the user's approval.
    pub async fn confirmations(&self) -> Vec<Value> {
        let Ok(reply) = self.control.confirmations_request().send().promise.await else {
            return Vec::new();
        };
        reply
            .get()
            .ok()
            .map(|reply| text(reply.get_list()))
            .and_then(|list| serde_json::from_str::<Value>(&list).ok())
            .and_then(|list| list.as_array().cloned())
            .unwrap_or_default()
    }

    /// `Control.confirm`: answers confirmation `id`; `false` when nothing waits under it.
    pub async fn confirm(&self, id: &str, allow: bool, remember: bool) -> bool {
        let mut request = self.control.confirm_request();
        request.get().set_id(id);
        request.get().set_allow(allow);
        request.get().set_remember(remember);
        match request.send().promise.await {
            Ok(reply) => reply.get().is_ok_and(|reply| reply.get_found()),
            Err(_) => false,
        }
    }

    /// Subscribes `on_event` to a topic until the returned handle is dropped.
    pub async fn watch(
        &self,
        topic: Topic,
        approver: Option<&str>,
        allowed: bool,
        on_event: impl Fn(Topic, Value) + 'static,
    ) -> Result<vexdom_schema::handle::Client, capnp::Error> {
        let mut request = self.control.watch_request();
        request.get().set_topic(topic);
        request.get().set_approver(approver.unwrap_or(""));
        request.get().set_allowed(allowed);
        request
            .get()
            .set_sink(capnp_rpc::new_client::<event_sink::Client, _>(Sink(
                Box::new(on_event),
            )));
        let reply = request.send().promise.await?;
        reply.get()?.get_handle()
    }
}

pub struct Described {
    /// The offer's stamp, 0 from a Bridge that does not stamp it.
    pub stamp: u64,
    /// `None` when the offer still has the stamp the caller sent.
    pub tools: Option<Vec<Value>>,
    pub reserved: Vec<String>,
    pub withheld: Vec<String>,
}

struct Sink(Box<dyn Fn(Topic, Value)>);

impl event_sink::Server for Sink {
    fn event(
        self: capnp::capability::Rc<Self>,
        params: event_sink::EventParams,
        _: event_sink::EventResults,
    ) -> impl std::future::Future<Output = Result<(), capnp::Error>> + 'static {
        std::future::ready((|| {
            let params = params.get()?;
            let topic = params
                .get_topic()
                .map_err(|_| capnp::Error::failed("unknown topic".into()))?;
            let payload = serde_json::from_str(&text(params.get_payload())).unwrap_or(Value::Null);
            (self.0)(topic, payload);
            Ok(())
        })())
    }
}

/// Runs `body` on a fresh current-thread runtime inside a `LocalSet`.
pub fn block_on<F: std::future::Future>(body: F) -> F::Output {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("a tokio runtime starts");
    tokio::task::LocalSet::new().block_on(&runtime, body)
}
