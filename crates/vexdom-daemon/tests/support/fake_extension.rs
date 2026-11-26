//! Dials `/extension` as a Chromium extension, runs the TypeScript-compatible handshake, and serves
//! a `Page` that answers from canned results.

use std::cell::RefCell;
use std::collections::HashMap;
use std::future::Future;
use std::rc::Rc;

use capnp_rpc::{RpcSystem, rpc_twoparty_capnp::Side};
use serde_json::Value;
use vexdom_core::handshake::new_nonce;
use vexdom_pure::handshake::{self, Transcript};
use vexdom_schema::{AuthKind, extension_session, extension_ui, page, prove_outcome, run_event};

pub const ORIGIN: &str = "chrome-extension://abcdefghijklmnopabcdefghijklmnop";
pub const INSTALL_ID: &str = "install-0123456789ab";

/// Canned answers by action, and every call the page saw.
#[derive(Default)]
pub struct Canned {
    pub answers: HashMap<String, Value>,
    pub calls: Vec<(String, Value)>,
    /// Every push the side panel got, by name; a run's events as `run <id> <kind> <detail>`.
    pub pushes: Vec<String>,
}

pub type Shared = Rc<RefCell<Canned>>;

struct FakePage(Shared);

impl page::Server for FakePage {
    fn describe(
        self: capnp::capability::Rc<Self>,
        _: page::DescribeParams,
        mut results: page::DescribeResults,
    ) -> impl Future<Output = Result<(), capnp::Error>> + 'static {
        results.get().set_tools("[]");
        std::future::ready(Ok(()))
    }

    fn invoke(
        self: capnp::capability::Rc<Self>,
        params: page::InvokeParams,
        mut results: page::InvokeResults,
    ) -> impl Future<Output = Result<(), capnp::Error>> + 'static {
        std::future::ready((|| {
            let params = params.get()?;
            let action = params.get_action()?.to_string()?;
            let input: Value =
                serde_json::from_str(params.get_input()?.to_str()?).unwrap_or(Value::Null);
            self.0.borrow_mut().calls.push((action.clone(), input));
            let answer = self.0.borrow().answers.get(&action).cloned();
            let mut result = results.get().init_result();
            match answer {
                Some(data) => result.set_ok(serde_json::to_string(&data).unwrap()),
                None => {
                    let mut error = result.init_error();
                    error.set_code("UNKNOWN_ACTION");
                    error.set_message(format!("Unknown action \"{action}\"."));
                }
            }
            Ok(())
        })())
    }

    fn screenshot(
        self: capnp::capability::Rc<Self>,
        params: page::ScreenshotParams,
        mut results: page::ScreenshotResults,
    ) -> impl Future<Output = Result<(), capnp::Error>> + 'static {
        std::future::ready((|| {
            let input: Value =
                serde_json::from_str(params.get()?.get_input()?.to_str()?).unwrap_or(Value::Null);
            self.0
                .borrow_mut()
                .calls
                .push(("page.screenshot".into(), input));
            results
                .get()
                .init_result()
                .set_ok(r#"{"format":"png","width":2,"height":2}"#);
            results.get().set_image(b"\x89PNG fake");
            Ok(())
        })())
    }

    fn run_task(
        self: capnp::capability::Rc<Self>,
        _: page::RunTaskParams,
        mut results: page::RunTaskResults,
    ) -> impl Future<Output = Result<(), capnp::Error>> + 'static {
        results.get().init_result().set_ok("null");
        std::future::ready(Ok(()))
    }
}

struct FakeUi(Shared);

macro_rules! push {
    ($name:ident, $params:ident, $results:ident, $label:literal) => {
        fn $name(
            self: capnp::capability::Rc<Self>,
            _: extension_ui::$params,
            _: extension_ui::$results,
        ) -> impl Future<Output = Result<(), capnp::Error>> + 'static {
            self.0.borrow_mut().pushes.push($label.into());
            std::future::ready(Ok(()))
        }
    };
}

/// `run <id> <kind> <detail>`: what a run told the side panel, on one line.
fn run_line(params: extension_ui::RunParams) -> capnp::Result<String> {
    let params = params.get()?;
    let id = params.get_id()?.to_str()?.to_owned();
    let what = match params.get_event()?.which()? {
        run_event::Started(started) => format!("started {}", started.get_skill()?.to_str()?),
        run_event::Text(delta) => format!("text {}", delta?.to_str()?),
        run_event::Tool(tool) => format!("tool {}", tool.get_action()?.to_str()?),
        run_event::Approval(approval) => format!("approval {}", approval.get_action()?.to_str()?),
        run_event::ToolResult(result) => format!("toolResult {}", result.get_summary()?.to_str()?),
        run_event::Session(session) => {
            format!("session {}", session.get_agent_session_id()?.to_str()?)
        }
        run_event::Attachments(_) => "attachments".into(),
        run_event::Usage(_) => "usage".into(),
        run_event::Done(reason) => format!("done {}", reason?.to_str()?),
        run_event::Error(error) => {
            let error = error?;
            format!(
                "error {}: {}",
                error.get_code()?.to_str()?,
                error.get_message()?.to_str()?
            )
        }
    };
    Ok(format!("run {id} {what}"))
}

impl extension_ui::Server for FakeUi {
    fn run(
        self: capnp::capability::Rc<Self>,
        params: extension_ui::RunParams,
        _: extension_ui::RunResults,
    ) -> impl Future<Output = Result<(), capnp::Error>> + 'static {
        let line = run_line(params).unwrap_or_else(|error| format!("run ? {error}"));
        self.0.borrow_mut().pushes.push(line);
        std::future::ready(Ok(()))
    }

    push!(
        site_map_draft,
        SiteMapDraftParams,
        SiteMapDraftResults,
        "siteMapDraft"
    );
    push!(
        open_notice,
        OpenNoticeParams,
        OpenNoticeResults,
        "openNotice"
    );
    push!(agent_info, AgentInfoParams, AgentInfoResults, "agentInfo");
    push!(
        skill_catalog,
        SkillCatalogParams,
        SkillCatalogResults,
        "skillCatalog"
    );
    push!(
        preferences_info,
        PreferencesInfoParams,
        PreferencesInfoResults,
        "preferencesInfo"
    );
    push!(task_list, TaskListParams, TaskListResults, "taskList");
}

pub enum Outcome {
    Welcome {
        session: extension_session::Client,
        session_key: Option<String>,
        in_sync: bool,
    },
    Unauthorized {
        reason: String,
        code: String,
    },
}

pub enum Credential<'a> {
    Code(&'a str),
    Session(&'a str),
}

/// Connects and proves; the RPC system runs on the current `LocalSet` until the socket ends.
pub async fn connect(
    port: u16,
    credential: Credential<'_>,
    canned: Shared,
) -> Result<Outcome, capnp::Error> {
    let (gate, system): (vexdom_schema::extension_gate::Client, RpcSystem<Side>) =
        vexdom_link::connect_extension(port, ORIGIN)
            .await
            .map_err(|e| capnp::Error::failed(e.to_string()))?;
    tokio::task::spawn_local(async move {
        let _ = system.await;
    });
    let nonce = new_nonce();
    let mut hello = gate.hello_request();
    {
        let mut params = hello.get();
        params.set_protocol_version(27);
        params.set_min_daemon_protocol(27);
        params.set_extension_version("0.9.2");
        params.set_manifest_hash(&vexdom_core::tools::chromium().hash);
        params.set_auth(match credential {
            Credential::Code(_) => AuthKind::Pair,
            Credential::Session(_) => AuthKind::Session,
        });
        params.set_nonce(&nonce);
        params.set_install_id(INSTALL_ID);
        params.set_browser("Chrome");
    }
    let reply = hello.send().promise.await?;
    let server_nonce = reply.get()?.get_server_nonce()?.to_string()?;
    let transcript = Transcript {
        protocol_version: 27,
        extension_version: "0.9.2".into(),
        manifest_hash: vexdom_core::tools::chromium().hash.clone(),
        client_nonce: nonce,
        server_nonce,
    };
    let secret = match credential {
        Credential::Code(code) => handshake::pairing_secret(code, &transcript),
        Credential::Session(key) => key.to_owned(),
    };
    let proof = handshake::client_proof(&secret, &transcript).unwrap();
    let mut prove = gate.prove_request();
    prove.get().set_proof(&proof);
    prove
        .get()
        .set_page(capnp_rpc::new_client(FakePage(Rc::clone(&canned))));
    prove.get().set_ui(capnp_rpc::new_client(FakeUi(canned)));
    let reply = prove.send().promise.await?;
    let outcome = reply.get()?.get_outcome()?;
    Ok(match outcome.which()? {
        prove_outcome::Welcome(welcome) => {
            let welcome = welcome?;
            let sealed = welcome.get_sealed_session_key()?.to_string()?;
            let expected = handshake::server_proof(
                &secret,
                &transcript,
                welcome.get_daemon_version()?.to_str()?,
                welcome.get_manifest_hash()?.to_str()?,
                welcome.get_manifest_in_sync(),
                (!sealed.is_empty()).then_some(sealed.as_str()),
            )
            .unwrap();
            assert_eq!(
                welcome.get_proof()?.to_str()?,
                expected,
                "the daemon proves itself"
            );
            Outcome::Welcome {
                session: welcome.get_session()?,
                session_key: (!sealed.is_empty())
                    .then(|| handshake::open_session_key(&secret, &transcript, &sealed).unwrap()),
                in_sync: welcome.get_manifest_in_sync(),
            }
        }
        prove_outcome::Unauthorized(refusal) => {
            let refusal = refusal?;
            Outcome::Unauthorized {
                reason: refusal.get_reason()?.to_string()?,
                code: refusal.get_code()?.to_string()?,
            }
        }
    })
}
