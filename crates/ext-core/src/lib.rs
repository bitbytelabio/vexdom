//! The extension's end of the Bridge link (ADR-017, ADR-025): capnp-rpc inside a BoltFFI wasm
//! module, with JavaScript owning the WebSocket.
//!
//! BoltFFI's exported futures must be `Send`, and capnp-rpc's are not (its servers are `Rc`), so
//! nothing async crosses the boundary. The RPC system runs on a thread-local `LocalPool`, and
//! `src/lib/bridge/socket.ts` drives it with synchronous calls: `link_open` once per socket,
//! `link_feed` with every binary frame, `link_pump` to run the pool until it stalls and take the
//! bytes to send, and `link_events` to learn what happened. Calls into the daemon
//! (`link_hello`, `link_prove`, `link_request`) are queued and answered as events; calls from the
//! daemon into the page arrive as `PageCall` events and are answered with `link_reply`.
//!
//! Documents owned by TypeScript cross as JSON text in their existing TS shapes (`ActionResult`,
//! `RunEvent`), so the side panel code is unchanged.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, VecDeque};
use std::io;
use std::pin::Pin;
use std::rc::Rc;
use std::task::{Context, Poll, Waker};

use boltffi::{data, export};
use capnp_rpc::{RpcSystem, rpc_twoparty_capnp, twoparty};
use futures::channel::oneshot;
use futures::executor::LocalPool;
use futures::task::LocalSpawnExt;
use futures::{AsyncRead, AsyncWrite};
use serde_json::{Value, json};
use vexdom_schema::{
    AuthKind, action_result, extension_gate, extension_session, extension_ui, page, prove_outcome,
    run_event,
};

boltffi::scaffolding!();

mod pure;
pub use pure::*;

/// What the extension says in `ExtensionGate.hello`.
#[data]
#[derive(Clone, Debug)]
pub struct Hello {
    pub protocol_version: u32,
    pub min_daemon_protocol: u32,
    pub extension_version: String,
    pub manifest_hash: String,
    /// `pair` or `session`.
    pub auth: String,
    pub nonce: String,
    pub install_id: String,
    pub browser: String,
}

/// One per `ExtensionSession` method. Empty strings mean absent; JSON fields carry TS documents.
#[data]
#[derive(Clone, Debug)]
pub enum ExtensionRequest {
    Ping,
    Focus,
    Instruct {
        id: String,
        text: String,
        context: String,
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
        session_id: String,
    },
    AnalyzeFile {
        id: String,
        file_id: String,
        session_id: String,
        name: String,
        mime: String,
        size: f64,
        content: Vec<u8>,
    },
    CancelAnalysis {
        id: String,
        file_id: String,
    },
    RecordFrame {
        recording_id: String,
        frame: Vec<u8>,
        at_ms: f64,
    },
    SaveSkill {
        skill: String,
    },
    DeleteSkill {
        name: String,
    },
    DeleteSiteMap {
        name: String,
    },
    ActivateSiteMap {
        staging_id: String,
        exact_host: bool,
    },
    DiscardSiteMap {
        staging_id: String,
    },
    NameSession {
        host: String,
        messages: Vec<String>,
    },
    AnalyzeRecording {
        recording: String,
    },
    AgentState {
        refresh: bool,
    },
    SetAgent {
        agent: String,
    },
    SetAgentModel {
        agent: String,
        model: String,
    },
    GrantAgent {
        agent: String,
    },
    ListSkills {
        refresh: bool,
    },
    Preferences,
    SetPreference {
        change: String,
    },
    Tasks,
    SaveTask {
        task: String,
    },
    DeleteTask {
        task_id: String,
    },
    RunTaskNow {
        task_id: String,
    },
    PauseTasks {
        paused: bool,
    },
    TaskDone {
        task_id: String,
        result: String,
    },
    Notice {
        notice: String,
    },
}

/// What happened on the link since the last `link_events`.
#[data]
#[derive(Clone, Debug, PartialEq)]
pub enum LinkEvent {
    /// `hello` answered: compute the proof over the transcript with this nonce.
    ServerNonce {
        nonce: String,
    },
    /// `hello` refused; the reason is the daemon's (a protocol mismatch above all).
    Refused {
        reason: String,
    },
    Welcome {
        daemon_version: String,
        protocol_version: u32,
        manifest_hash: String,
        manifest_in_sync: bool,
        proof: String,
        sealed_session_key: String,
    },
    Unauthorized {
        reason: String,
        retryable: bool,
        code: String,
    },
    /// The answer to `link_request` number `req`: a TS `ActionResult` as JSON. Methods with no
    /// result answer `{"ok":true,"data":null}`; `notice` answers its `delivered` flag as `data`.
    Reply {
        req: u32,
        result: String,
    },
    /// The daemon calls the page: `describe`, `invoke`, `screenshot`, `artifact` or `runTask`.
    /// Answer with `link_reply` (or `link_reply_screenshot`, `link_reply_artifact`). `input` is JSON; `tab_id` is -1 for none.
    PageCall {
        call_id: u32,
        kind: String,
        action: String,
        input: String,
        tab_id: i32,
        run_id: String,
    },
    /// An `ExtensionUi` push: `run` (payload: the TS `RunEvent`), `siteMapDraft` (the draft),
    /// `openNotice` (id: the notice), `agentInfo`, `skillCatalog`, `preferencesInfo`, `taskList`
    /// (payload: a TS `ActionResult`).
    Push {
        kind: String,
        id: String,
        payload: String,
    },
    Closed {
        reason: String,
    },
}

#[derive(Default)]
struct Inbox {
    bytes: VecDeque<u8>,
    waker: Option<Waker>,
    closed: bool,
}

/// The read half: bytes JavaScript fed in, handed to capnp as they are asked for.
struct Reader(Rc<RefCell<Inbox>>);

impl AsyncRead for Reader {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut [u8],
    ) -> Poll<io::Result<usize>> {
        let mut inbox = self.0.borrow_mut();
        if inbox.bytes.is_empty() {
            if inbox.closed {
                return Poll::Ready(Ok(0));
            }
            inbox.waker = Some(cx.waker().clone());
            return Poll::Pending;
        }
        let n = buf.len().min(inbox.bytes.len());
        for (slot, byte) in buf.iter_mut().zip(inbox.bytes.drain(..n)) {
            *slot = byte;
        }
        Poll::Ready(Ok(n))
    }
}

/// The write half: whatever capnp writes waits here until `link_pump` hands it to JavaScript.
struct Writer(Rc<RefCell<Vec<u8>>>);

impl AsyncWrite for Writer {
    fn poll_write(
        self: Pin<&mut Self>,
        _: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        self.0.borrow_mut().extend_from_slice(buf);
        Poll::Ready(Ok(buf.len()))
    }

    fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }

    fn poll_close(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }
}

/// A page call waiting for JavaScript: its TS `ActionResult` JSON and, for screenshots, the image.
type PageAnswer = (String, Vec<u8>);

/// What the futures on the pool share with the exported functions.
#[derive(Default)]
struct Shared {
    events: RefCell<Vec<LinkEvent>>,
    session: RefCell<Option<extension_session::Client>>,
    pending: RefCell<HashMap<u32, oneshot::Sender<PageAnswer>>>,
    next_call: Cell<u32>,
    /// Set by `link_trust` once JavaScript has checked the Bridge's proof. `prove` hands the page
    /// and UI capabilities over before that proof arrives, so until then every call from the peer
    /// waits here and never reaches the browser: an unproven listener on the port gets nothing.
    trusted: Cell<bool>,
    waiting: RefCell<Vec<oneshot::Sender<()>>>,
}

impl Shared {
    fn emit(&self, event: LinkEvent) {
        self.events.borrow_mut().push(event);
    }

    /// Resolves once the Bridge has proved itself; fails if the link goes first.
    fn until_trusted(&self) -> impl Future<Output = Result<(), capnp::Error>> + 'static + use<> {
        let waiting = (!self.trusted.get()).then(|| {
            let (ready, wait) = oneshot::channel();
            self.waiting.borrow_mut().push(ready);
            wait
        });
        async move {
            if let Some(wait) = waiting {
                wait.await.map_err(|_| {
                    capnp::Error::disconnected("the Bridge never proved itself".into())
                })?;
            }
            Ok(())
        }
    }

    fn trust(&self) {
        self.trusted.set(true);
        for ready in self.waiting.borrow_mut().drain(..) {
            let _ = ready.send(());
        }
    }

    /// Hands a page call to JavaScript, once the Bridge is trusted, and waits for its answer.
    fn page_call(
        self: &Rc<Self>,
        kind: &'static str,
        action: String,
        input: String,
        tab_id: i64,
        run_id: String,
    ) -> impl Future<Output = Result<PageAnswer, capnp::Error>> + 'static + use<> {
        let shared = Rc::clone(self);
        let trusted = self.until_trusted();
        async move {
            trusted.await?;
            let call_id = shared.next_call.get().wrapping_add(1);
            shared.next_call.set(call_id);
            let (answer, answered) = oneshot::channel();
            shared.pending.borrow_mut().insert(call_id, answer);
            shared.emit(LinkEvent::PageCall {
                call_id,
                kind: kind.into(),
                action,
                input,
                tab_id: i32::try_from(tab_id).unwrap_or(-1),
                run_id,
            });
            answered.await.map_err(|_| answered_away())
        }
    }
}

struct Link {
    pool: LocalPool,
    inbox: Rc<RefCell<Inbox>>,
    outbox: Rc<RefCell<Vec<u8>>>,
    shared: Rc<Shared>,
    gate: extension_gate::Client,
    next_request: u32,
}

thread_local! {
    static LINK: RefCell<Option<Link>> = const { RefCell::new(None) };
}

fn text(reader: capnp::Result<capnp::text::Reader<'_>>) -> String {
    reader
        .ok()
        .and_then(|text| text.to_str().ok().map(str::to_owned))
        .unwrap_or_default()
}

/// A capnp `ActionResult` as the TS `ActionResult` JSON.
fn result_json(reader: capnp::Result<action_result::Reader<'_>>) -> String {
    let failure = |code: &str, message: String| {
        json!({"ok": false, "error": {"code": code, "message": message}}).to_string()
    };
    let reader = match reader {
        Ok(reader) => reader,
        Err(error) => return failure("BRIDGE_ERROR", error.to_string()),
    };
    match reader.which() {
        Ok(action_result::Ok(data)) => {
            let data = text(data);
            let data: Value = if data.is_empty() { Value::Null } else { serde_json::from_str(&data).unwrap_or(Value::Null) };
            json!({"ok": true, "data": data}).to_string()
        }
        Ok(action_result::Error(Ok(error))) => {
            json!({"ok": false, "error": {"code": text(error.get_code()), "message": text(error.get_message())}}).to_string()
        }
        Ok(action_result::Error(Err(error))) => failure("BRIDGE_ERROR", error.to_string()),
        Err(error) => failure("BRIDGE_ERROR", error.to_string()),
    }
}

/// The TS `ActionResult` JSON JavaScript answered with, into a capnp `ActionResult`.
fn write_result(mut builder: action_result::Builder<'_>, result: &str) {
    let parsed: Value = serde_json::from_str(result).unwrap_or(Value::Null);
    if parsed.get("ok") == Some(&Value::Bool(true)) {
        builder.set_ok(parsed.get("data").unwrap_or(&Value::Null).to_string());
        return;
    }
    let error = parsed.get("error");
    let field = |key: &str, fallback: &str| {
        error
            .and_then(|error| error.get(key))
            .and_then(Value::as_str)
            .unwrap_or(fallback)
            .to_owned()
    };
    let mut out = builder.init_error();
    out.set_code(field("code", "BRIDGE_ERROR"));
    out.set_message(field("message", "The extension answered with no result."));
}

fn json_field(text: &str) -> Value {
    if text.is_empty() {
        Value::Null
    } else {
        serde_json::from_str(text).unwrap_or(Value::Null)
    }
}

/// A capnp `RunEvent` as the TS `RunEvent` JSON (`src/lib/actions/protocol.ts`).
fn run_event_json(event: run_event::Reader<'_>) -> Value {
    let optional = |value: String| (!value.is_empty()).then_some(value);
    match event.which() {
        Ok(run_event::Started(started)) => {
            let mut out = json!({"kind": "started", "skill": text(started.get_skill())});
            if let Some(attached) = optional(text(started.get_attached())) {
                out["attached"] = json_field(&attached);
            }
            if let Some(overlays) = optional(text(started.get_overlays())) {
                out["overlays"] = json_field(&overlays);
            }
            out
        }
        Ok(run_event::Text(delta)) => json!({"kind": "text", "delta": text(delta)}),
        Ok(run_event::Tool(tool)) => {
            let mut out = json!({
                "kind": "tool",
                "toolId": text(tool.get_tool_id()),
                "action": text(tool.get_action()),
                "input": json_field(&text(tool.get_input())),
            });
            if let Some(source) = optional(text(tool.get_source())) {
                out["source"] = json!(source);
            }
            out
        }
        Ok(run_event::Approval(approval)) => {
            let mut out = json!({
                "kind": "approval",
                "toolId": text(approval.get_tool_id()),
                "action": text(approval.get_action()),
                "input": json_field(&text(approval.get_input())),
            });
            if let Some(site) = optional(text(approval.get_site())) {
                out["site"] = json!(site);
            }
            out
        }
        Ok(run_event::ToolResult(result)) => json!({
            "kind": "toolResult",
            "toolId": text(result.get_tool_id()),
            "ok": result.get_ok(),
            "summary": text(result.get_summary()),
        }),
        Ok(run_event::Session(session)) => json!({
            "kind": "session",
            "agent": text(session.get_agent()),
            "agentSessionId": optional(text(session.get_agent_session_id())),
        }),
        Ok(run_event::Attachments(files)) => {
            json!({"kind": "attachments", "files": json_field(&text(files))})
        }
        Ok(run_event::Usage(usage)) => json!({
            "kind": "usage",
            "usage": {"contextTokens": usage.get_context_tokens(), "outputTokens": usage.get_output_tokens()},
        }),
        Ok(run_event::Done(reason)) => json!({"kind": "done", "stopReason": text(reason)}),
        Ok(run_event::Error(Ok(error))) => json!({
            "kind": "error",
            "code": text(error.get_code()),
            "message": text(error.get_message()),
        }),
        Ok(run_event::Error(Err(error))) => {
            json!({"kind": "error", "code": "BRIDGE_ERROR", "message": error.to_string()})
        }
        Err(error) => {
            json!({"kind": "error", "code": "BRIDGE_ERROR", "message": error.to_string()})
        }
    }
}

/// The page, as the daemon calls it: every call is handed to JavaScript as a `PageCall`.
struct PageServer(Rc<Shared>);

fn answered_away() -> capnp::Error {
    capnp::Error::disconnected("the extension dropped the call".into())
}

impl page::Server for PageServer {
    fn describe(
        self: capnp::capability::Rc<Self>,
        _: page::DescribeParams,
        mut results: page::DescribeResults,
    ) -> impl Future<Output = Result<(), capnp::Error>> + 'static {
        let answered =
            self.0
                .page_call("describe", String::new(), String::new(), -1, String::new());
        async move {
            let (result, _) = answered.await?;
            let parsed: Value = serde_json::from_str(&result).unwrap_or(Value::Null);
            results
                .get()
                .set_tools(parsed.get("data").unwrap_or(&json!([])).to_string());
            Ok(())
        }
    }

    fn invoke(
        self: capnp::capability::Rc<Self>,
        params: page::InvokeParams,
        mut results: page::InvokeResults,
    ) -> impl Future<Output = Result<(), capnp::Error>> + 'static {
        let answered = params.get().map(|params| {
            self.0.page_call(
                "invoke",
                text(params.get_action()),
                text(params.get_input()),
                params.get_tab_id(),
                text(params.get_run_id()),
            )
        });
        async move {
            let (result, _) = answered?.await?;
            write_result(results.get().init_result(), &result);
            Ok(())
        }
    }

    fn screenshot(
        self: capnp::capability::Rc<Self>,
        params: page::ScreenshotParams,
        mut results: page::ScreenshotResults,
    ) -> impl Future<Output = Result<(), capnp::Error>> + 'static {
        let answered = params.get().map(|params| {
            self.0.page_call(
                "screenshot",
                "page.screenshot".into(),
                text(params.get_input()),
                params.get_tab_id(),
                String::new(),
            )
        });
        async move {
            let (result, image) = answered?.await?;
            let mut out = results.get();
            write_result(out.reborrow().init_result(), &result);
            out.set_image(&image);
            Ok(())
        }
    }

    fn artifact(
        self: capnp::capability::Rc<Self>,
        params: page::ArtifactParams,
        mut results: page::ArtifactResults,
    ) -> impl Future<Output = Result<(), capnp::Error>> + 'static {
        let answered = params.get().map(|params| {
            self.0.page_call(
                "artifact",
                text(params.get_action()),
                text(params.get_input()),
                params.get_tab_id(),
                String::new(),
            )
        });
        async move {
            let (result, data) = answered?.await?;
            let mut out = results.get();
            write_result(out.reborrow().init_result(), &result);
            out.set_data(&data);
            Ok(())
        }
    }

    fn run_task(
        self: capnp::capability::Rc<Self>,
        params: page::RunTaskParams,
        mut results: page::RunTaskResults,
    ) -> impl Future<Output = Result<(), capnp::Error>> + 'static {
        let answered = params.get().map(|params| {
            self.0.page_call(
                "runTask",
                String::new(),
                text(params.get_order()),
                -1,
                String::new(),
            )
        });
        async move {
            let (result, _) = answered?.await?;
            write_result(results.get().init_result(), &result);
            Ok(())
        }
    }
}

/// The daemon's pushes, queued for JavaScript.
struct UiServer(Rc<Shared>);

impl UiServer {
    fn push(
        &self,
        kind: &str,
        id: String,
        payload: String,
    ) -> impl Future<Output = Result<(), capnp::Error>> + 'static + use<> {
        let shared = Rc::clone(&self.0);
        let trusted = shared.until_trusted();
        let kind = kind.to_owned();
        async move {
            trusted.await?;
            shared.emit(LinkEvent::Push { kind, id, payload });
            Ok(())
        }
    }
}

macro_rules! result_push {
    ($method:ident, $params:ty, $results:ty, $kind:literal) => {
        fn $method(
            self: capnp::capability::Rc<Self>,
            params: $params,
            _: $results,
        ) -> impl Future<Output = Result<(), capnp::Error>> + 'static {
            let payload = match params.get() {
                Ok(params) => result_json(params.get_result()),
                Err(error) => return futures::future::Either::Left(std::future::ready(Err(error))),
            };
            futures::future::Either::Right(self.push($kind, String::new(), payload))
        }
    };
}

impl extension_ui::Server for UiServer {
    fn run(
        self: capnp::capability::Rc<Self>,
        params: extension_ui::RunParams,
        _: extension_ui::RunResults,
    ) -> impl Future<Output = Result<(), capnp::Error>> + 'static {
        let pushed = params.get().and_then(|params| {
            let event = run_event_json(params.get_event()?);
            Ok((text(params.get_id()), event.to_string()))
        });
        match pushed {
            Ok((id, payload)) => futures::future::Either::Right(self.push("run", id, payload)),
            Err(error) => futures::future::Either::Left(std::future::ready(Err(error))),
        }
    }

    fn site_map_draft(
        self: capnp::capability::Rc<Self>,
        params: extension_ui::SiteMapDraftParams,
        _: extension_ui::SiteMapDraftResults,
    ) -> impl Future<Output = Result<(), capnp::Error>> + 'static {
        match params.get() {
            Ok(params) => futures::future::Either::Right(self.push(
                "siteMapDraft",
                text(params.get_id()),
                text(params.get_draft()),
            )),
            Err(error) => futures::future::Either::Left(std::future::ready(Err(error))),
        }
    }

    fn open_notice(
        self: capnp::capability::Rc<Self>,
        params: extension_ui::OpenNoticeParams,
        _: extension_ui::OpenNoticeResults,
    ) -> impl Future<Output = Result<(), capnp::Error>> + 'static {
        match params.get() {
            Ok(params) => futures::future::Either::Right(self.push(
                "openNotice",
                text(params.get_notice_id()),
                String::new(),
            )),
            Err(error) => futures::future::Either::Left(std::future::ready(Err(error))),
        }
    }

    result_push!(
        agent_info,
        extension_ui::AgentInfoParams,
        extension_ui::AgentInfoResults,
        "agentInfo"
    );
    result_push!(
        skill_catalog,
        extension_ui::SkillCatalogParams,
        extension_ui::SkillCatalogResults,
        "skillCatalog"
    );
    result_push!(
        preferences_info,
        extension_ui::PreferencesInfoParams,
        extension_ui::PreferencesInfoResults,
        "preferencesInfo"
    );
    result_push!(
        task_list,
        extension_ui::TaskListParams,
        extension_ui::TaskListResults,
        "taskList"
    );
}

/// Starts the RPC system for a freshly opened socket, replacing any earlier one.
#[export]
pub fn link_open() {
    let inbox = Rc::new(RefCell::new(Inbox::default()));
    let outbox = Rc::new(RefCell::new(Vec::new()));
    let shared = Rc::new(Shared::default());
    let network = twoparty::VatNetwork::new(
        Reader(Rc::clone(&inbox)),
        Writer(Rc::clone(&outbox)),
        rpc_twoparty_capnp::Side::Client,
        vexdom_schema::reader_options(),
    );
    let mut system = RpcSystem::new(Box::new(network), None);
    let gate: extension_gate::Client = system.bootstrap(rpc_twoparty_capnp::Side::Server);
    let pool = LocalPool::new();
    let ended = Rc::clone(&shared);
    let _ = pool.spawner().spawn_local(async move {
        let reason = match system.await {
            Ok(()) => "the Bridge closed the link".to_owned(),
            Err(error) => error.to_string(),
        };
        ended.pending.borrow_mut().clear();
        ended.waiting.borrow_mut().clear();
        ended.session.borrow_mut().take();
        ended.emit(LinkEvent::Closed { reason });
    });
    LINK.with_borrow_mut(|link| {
        *link = Some(Link {
            pool,
            inbox,
            outbox,
            shared,
            gate,
            next_request: 0,
        })
    });
}

/// Hands capnp the bytes of one binary frame.
#[export]
pub fn link_feed(bytes: &[u8]) {
    LINK.with_borrow_mut(|link| {
        if let Some(link) = link {
            let mut inbox = link.inbox.borrow_mut();
            inbox.bytes.extend(bytes);
            if let Some(waker) = inbox.waker.take() {
                waker.wake();
            }
        }
    });
}

/// The socket closed: capnp reads end-of-stream and the RPC system winds down on the next pump.
#[export]
pub fn link_close() {
    LINK.with_borrow_mut(|link| {
        if let Some(link) = link {
            let mut inbox = link.inbox.borrow_mut();
            inbox.closed = true;
            if let Some(waker) = inbox.waker.take() {
                waker.wake();
            }
        }
    });
}

/// Runs everything that can make progress and returns the bytes to send, possibly none.
#[export]
pub fn link_pump() -> Vec<u8> {
    // The pool runs page and UI servers that only touch `Shared`, never `LINK`, so holding the
    // borrow across `run_until_stalled` is sound.
    LINK.with_borrow_mut(|link| {
        let Some(link) = link else { return Vec::new() };
        link.pool.run_until_stalled();
        std::mem::take(&mut *link.outbox.borrow_mut())
    })
}

/// What happened since the last call.
#[export]
pub fn link_events() -> Vec<LinkEvent> {
    LINK.with_borrow(|link| {
        link.as_ref()
            .map(|link| std::mem::take(&mut *link.shared.events.borrow_mut()))
            .unwrap_or_default()
    })
}

fn spawn(task: impl Future<Output = ()> + 'static) {
    LINK.with_borrow(|link| {
        if let Some(link) = link {
            let _ = link.pool.spawner().spawn_local(task);
        }
    });
}

/// Queues `ExtensionGate.hello`; answered by `ServerNonce` or `Refused`.
#[export]
pub fn link_hello(hello: Hello) {
    let Some((gate, shared)) = LINK.with_borrow(|link| {
        link.as_ref()
            .map(|link| (link.gate.clone(), Rc::clone(&link.shared)))
    }) else {
        return;
    };
    spawn(async move {
        let mut request = gate.hello_request();
        {
            let mut params = request.get();
            params.set_protocol_version(hello.protocol_version);
            params.set_min_daemon_protocol(hello.min_daemon_protocol);
            params.set_extension_version(&hello.extension_version);
            params.set_manifest_hash(&hello.manifest_hash);
            params.set_auth(if hello.auth == "pair" {
                AuthKind::Pair
            } else {
                AuthKind::Session
            });
            params.set_nonce(&hello.nonce);
            params.set_install_id(&hello.install_id);
            params.set_browser(&hello.browser);
        }
        let event = match request.send().promise.await {
            Ok(reply) => match reply.get() {
                Ok(reply) => LinkEvent::ServerNonce {
                    nonce: text(reply.get_server_nonce()),
                },
                Err(error) => LinkEvent::Refused {
                    reason: error.extra().to_owned(),
                },
            },
            Err(error) => LinkEvent::Refused {
                reason: refusal_text(&error),
            },
        };
        shared.emit(event);
    });
}

/// A capnp exception's text without the transport's prefix.
fn refusal_text(error: &capnp::Error) -> String {
    error
        .extra()
        .trim_start_matches("remote exception: ")
        .trim()
        .to_owned()
}

/// Queues `ExtensionGate.prove` with this extension's page and UI; answered by `Welcome` or
/// `Unauthorized`.
#[export]
pub fn link_prove(proof: String) {
    let Some((gate, shared)) = LINK.with_borrow(|link| {
        link.as_ref()
            .map(|link| (link.gate.clone(), Rc::clone(&link.shared)))
    }) else {
        return;
    };
    spawn(async move {
        let mut request = gate.prove_request();
        {
            let mut params = request.get();
            params.set_proof(&proof);
            params.set_page(capnp_rpc::new_client(PageServer(Rc::clone(&shared))));
            params.set_ui(capnp_rpc::new_client(UiServer(Rc::clone(&shared))));
        }
        let event = match request.send().promise.await {
            Ok(reply) => match reply
                .get()
                .and_then(|reply| reply.get_outcome())
                .map(|outcome| outcome.which())
            {
                Ok(Ok(prove_outcome::Welcome(Ok(welcome)))) => {
                    if let Ok(session) = welcome.get_session() {
                        shared.session.replace(Some(session));
                    }
                    LinkEvent::Welcome {
                        daemon_version: text(welcome.get_daemon_version()),
                        protocol_version: welcome.get_protocol_version(),
                        manifest_hash: text(welcome.get_manifest_hash()),
                        manifest_in_sync: welcome.get_manifest_in_sync(),
                        proof: text(welcome.get_proof()),
                        sealed_session_key: text(welcome.get_sealed_session_key()),
                    }
                }
                Ok(Ok(prove_outcome::Unauthorized(Ok(refused)))) => LinkEvent::Unauthorized {
                    reason: text(refused.get_reason()),
                    retryable: refused.get_retryable(),
                    code: text(refused.get_code()),
                },
                Ok(Ok(
                    prove_outcome::Welcome(Err(error)) | prove_outcome::Unauthorized(Err(error)),
                ))
                | Err(error) => LinkEvent::Unauthorized {
                    reason: error.to_string(),
                    retryable: true,
                    code: String::new(),
                },
                Ok(Err(error)) => LinkEvent::Unauthorized {
                    reason: error.to_string(),
                    retryable: true,
                    code: String::new(),
                },
            },
            Err(error) => LinkEvent::Unauthorized {
                reason: refusal_text(&error),
                retryable: true,
                code: String::new(),
            },
        };
        shared.emit(event);
    });
}

const DONE: &str = r#"{"ok":true,"data":null}"#;

fn offline() -> String {
    json!({"ok": false, "error": {"code": "EXTENSION_OFFLINE", "message": "No Vexdom daemon is attached — pair the browser first."}})
        .to_string()
}

fn failed(error: &capnp::Error) -> String {
    json!({"ok": false, "error": {"code": "BRIDGE_ERROR", "message": refusal_text(error)}})
        .to_string()
}

/// Sends a request and turns its answer into a TS `ActionResult` JSON.
macro_rules! call {
    ($request:expr, |$reply:ident| $answer:expr) => {{
        match $request.send().promise.await {
            Ok(response) => match response.get() {
                Ok($reply) => $answer,
                Err(error) => failed(&error),
            },
            Err(error) => failed(&error),
        }
    }};
}

async fn request(session: extension_session::Client, request: ExtensionRequest) -> String {
    match request {
        ExtensionRequest::Ping => call!(session.ping_request(), |_reply| DONE.to_owned()),
        ExtensionRequest::Focus => call!(session.focus_request(), |_reply| DONE.to_owned()),
        ExtensionRequest::Instruct { id, text, context } => {
            let mut call = session.instruct_request();
            call.get().set_id(&id);
            call.get().set_text(&text);
            call.get().set_context(&context);
            call!(call, |_reply| DONE.to_owned())
        }
        ExtensionRequest::Cancel { id } => {
            let mut call = session.cancel_request();
            call.get().set_id(&id);
            call!(call, |_reply| DONE.to_owned())
        }
        ExtensionRequest::Decision {
            id,
            tool_id,
            allow,
            remember,
        } => {
            let mut call = session.decision_request();
            call.get().set_id(&id);
            call.get().set_tool_id(&tool_id);
            call.get().set_allow(allow);
            call.get().set_remember(remember);
            call!(call, |_reply| DONE.to_owned())
        }
        ExtensionRequest::Reset { session_id } => {
            let mut call = session.reset_request();
            call.get().set_session_id(&session_id);
            call!(call, |_reply| DONE.to_owned())
        }
        ExtensionRequest::AnalyzeFile {
            id,
            file_id,
            session_id,
            name,
            mime,
            size,
            content,
        } => {
            let mut call = session.analyze_file_request();
            {
                let mut params = call.get();
                params.set_id(&id);
                params.set_file_id(&file_id);
                params.set_session_id(&session_id);
                params.set_name(&name);
                params.set_mime(&mime);
                params.set_size(size.max(0.0) as u64);
                params.set_content(&content);
            }
            call!(call, |reply| result_json(reply.get_result()))
        }
        ExtensionRequest::RecordFrame {
            recording_id,
            frame,
            at_ms,
        } => {
            let mut call = session.record_frame_request();
            {
                let mut params = call.get();
                params.set_recording_id(&recording_id);
                params.set_frame(&frame);
                params.set_at_ms(at_ms);
            }
            call!(call, |_reply| DONE.to_owned())
        }
        ExtensionRequest::CancelAnalysis { id, file_id } => {
            let mut call = session.cancel_analysis_request();
            call.get().set_id(&id);
            call.get().set_file_id(&file_id);
            call!(call, |_reply| DONE.to_owned())
        }
        ExtensionRequest::SaveSkill { skill } => {
            let mut call = session.save_skill_request();
            call.get().set_skill(&skill);
            call!(call, |reply| result_json(reply.get_result()))
        }
        ExtensionRequest::DeleteSkill { name } => {
            let mut call = session.delete_skill_request();
            call.get().set_name(&name);
            call!(call, |reply| result_json(reply.get_result()))
        }
        ExtensionRequest::DeleteSiteMap { name } => {
            let mut call = session.delete_site_map_request();
            call.get().set_name(&name);
            call!(call, |reply| result_json(reply.get_result()))
        }
        ExtensionRequest::ActivateSiteMap {
            staging_id,
            exact_host,
        } => {
            let mut call = session.activate_site_map_request();
            call.get().set_staging_id(&staging_id);
            call.get().set_exact_host(exact_host);
            call!(call, |reply| result_json(reply.get_result()))
        }
        ExtensionRequest::DiscardSiteMap { staging_id } => {
            let mut call = session.discard_site_map_request();
            call.get().set_staging_id(&staging_id);
            call!(call, |reply| result_json(reply.get_result()))
        }
        ExtensionRequest::NameSession { host, messages } => {
            let mut call = session.name_session_request();
            call.get().set_host(&host);
            let count = u32::try_from(messages.len()).unwrap_or(u32::MAX);
            let mut list = call.get().init_messages(count);
            for (at, message) in messages.iter().enumerate().take(count as usize) {
                list.set(at as u32, message);
            }
            call!(call, |reply| result_json(reply.get_result()))
        }
        ExtensionRequest::AnalyzeRecording { recording } => {
            let mut call = session.analyze_recording_request();
            call.get().set_recording(&recording);
            call!(call, |reply| result_json(reply.get_result()))
        }
        ExtensionRequest::AgentState { refresh } => {
            let mut call = session.agent_state_request();
            call.get().set_refresh(refresh);
            call!(call, |reply| result_json(reply.get_result()))
        }
        ExtensionRequest::SetAgent { agent } => {
            let mut call = session.set_agent_request();
            call.get().set_agent(&agent);
            call!(call, |reply| result_json(reply.get_result()))
        }
        ExtensionRequest::SetAgentModel { agent, model } => {
            let mut call = session.set_agent_model_request();
            call.get().set_agent(&agent);
            call.get().set_model(&model);
            call!(call, |reply| result_json(reply.get_result()))
        }
        ExtensionRequest::GrantAgent { agent } => {
            let mut call = session.grant_agent_request();
            call.get().set_agent(&agent);
            call!(call, |reply| result_json(reply.get_result()))
        }
        ExtensionRequest::ListSkills { refresh } => {
            let mut call = session.list_skills_request();
            call.get().set_refresh(refresh);
            call!(call, |reply| result_json(reply.get_result()))
        }
        ExtensionRequest::Preferences => call!(session.preferences_request(), |reply| result_json(
            reply.get_result()
        )),
        ExtensionRequest::SetPreference { change } => {
            let mut call = session.set_preference_request();
            call.get().set_change(&change);
            call!(call, |reply| result_json(reply.get_result()))
        }
        ExtensionRequest::Tasks => call!(session.tasks_request(), |reply| result_json(
            reply.get_result()
        )),
        ExtensionRequest::SaveTask { task } => {
            let mut call = session.save_task_request();
            call.get().set_task(&task);
            call!(call, |reply| result_json(reply.get_result()))
        }
        ExtensionRequest::DeleteTask { task_id } => {
            let mut call = session.delete_task_request();
            call.get().set_task_id(&task_id);
            call!(call, |reply| result_json(reply.get_result()))
        }
        ExtensionRequest::RunTaskNow { task_id } => {
            let mut call = session.run_task_now_request();
            call.get().set_task_id(&task_id);
            call!(call, |reply| result_json(reply.get_result()))
        }
        ExtensionRequest::PauseTasks { paused } => {
            let mut call = session.pause_tasks_request();
            call.get().set_paused(paused);
            call!(call, |reply| result_json(reply.get_result()))
        }
        ExtensionRequest::TaskDone { task_id, result } => {
            let mut call = session.task_done_request();
            call.get().set_task_id(&task_id);
            call.get().set_result(&result);
            call!(call, |_reply| DONE.to_owned())
        }
        ExtensionRequest::Notice { notice } => {
            let mut call = session.notice_request();
            call.get().set_notice(&notice);
            call!(call, |reply| {
                json!({"ok": true, "data": reply.get_delivered()}).to_string()
            })
        }
    }
}

/// Queues one `ExtensionSession` call and returns the request number its `Reply` carries. Before
/// a welcome, or after the link closed, the reply is `EXTENSION_OFFLINE`.
#[export]
pub fn link_request(req: ExtensionRequest) -> u32 {
    let Some((number, shared)) = LINK.with_borrow_mut(|link| {
        link.as_mut().map(|link| {
            link.next_request = link.next_request.wrapping_add(1);
            (link.next_request, Rc::clone(&link.shared))
        })
    }) else {
        return 0;
    };
    let session = shared.session.borrow().clone();
    let Some(session) = session else {
        shared.emit(LinkEvent::Reply {
            req: number,
            result: offline(),
        });
        return number;
    };
    spawn(async move {
        let result = request(session, req).await;
        shared.emit(LinkEvent::Reply {
            req: number,
            result,
        });
    });
    number
}

fn answer(call_id: u32, result: String, image: Vec<u8>) {
    LINK.with_borrow(|link| {
        if let Some(link) = link
            && let Some(answer) = link.shared.pending.borrow_mut().remove(&call_id)
        {
            let _ = answer.send((result, image));
        }
    });
}

/// Answers a `PageCall` with a TS `ActionResult` JSON (`describe`: `data` is the tool list).
#[export]
pub fn link_reply(call_id: u32, result: String) {
    answer(call_id, result, Vec::new());
}

/// Answers a `screenshot` `PageCall`: the result JSON without its `dataUrl`, and the image bytes.
#[export]
pub fn link_reply_screenshot(call_id: u32, result: String, image: Vec<u8>) {
    answer(call_id, result, image);
}

/// Answers an `artifact` `PageCall`: the result JSON without its payload, and the file's bytes.
#[export]
pub fn link_reply_artifact(call_id: u32, result: String, data: Vec<u8>) {
    answer(call_id, result, data);
}

/// JavaScript has checked the Bridge's proof (the `Welcome`): from now on its calls reach the page
/// and its pushes reach the side panel. Before this they wait, and die with the link.
#[export]
pub fn link_trust() {
    LINK.with_borrow(|link| {
        if let Some(link) = link {
            link.shared.trust();
        }
    });
}
