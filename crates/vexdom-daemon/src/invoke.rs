//! Page calls with the daemon's half of the work (`invokeOn`, `invokeExternal`): reserved actions
//! refused, downloads listed and attachments filled from disk, screenshots and downloads kept,
//! guardrails for callers outside any run.

use std::rc::Rc;

use serde_json::{Value, json};
use vexdom_core::config::read_agent_config;
use vexdom_core::downloads::{self, CapturedItem};
use vexdom_core::guardrails::scope::anywhere;
use vexdom_core::guardrails::secrets::seal_secrets;
use vexdom_core::guardrails::{self, Caller, Effect, GuardrailRequest, decide, policy_from};
use vexdom_core::log;
use vexdom_core::proto::{EXTERNAL_RUN_ID, RESERVED_PREFIX};
use vexdom_core::result::{ActionError, ActionResult, failure};

use crate::agent::{self, InvokeOpts};
use crate::link::{Link, Reply};
use crate::wire::RunEvent;
use crate::{Daemon, unknown_action};

fn mime_of(data: &Value) -> &'static str {
    match data.get("format").and_then(Value::as_str) {
        Some("jpeg" | "jpg") => "image/jpeg",
        Some("webp") => "image/webp",
        _ => "image/png",
    }
}

/// Keeps a screenshot on disk when the caller asked to (`input.save`) or a run says where.
fn persist_screenshot(action: &str, input: &Value, mut reply: Reply, opts: &InvokeOpts) -> Reply {
    if action != "page.screenshot" {
        return reply;
    }
    let wants = input.get("save") == Some(&Value::Bool(true)) || opts.save_to.is_some();
    let (Ok(data), Some(image)) = (&mut reply.result, &reply.image) else {
        return reply;
    };
    if !wants {
        return reply;
    }
    let filename = opts
        .save_to
        .as_ref()
        .map(|save| save.filename.clone())
        .or_else(|| {
            input
                .get("filename")
                .and_then(Value::as_str)
                .map(str::to_owned)
        });
    let dir = opts.save_to.as_ref().map(|save| save.dir.clone());
    match downloads::save_screenshot(mime_of(data), image, filename.as_deref(), dir.as_deref()) {
        Ok(path) => {
            log::log(&format!("screenshot saved to {}", path.display()));
            data["savedTo"] = json!(path.to_string_lossy());
        }
        Err(error) => {
            log::log(&format!("screenshot save failed: {error}"));
            data["saveError"] = json!(error);
        }
    }
    reply
}

/// The capture's other half: whether a download is kept is decided here.
fn persist_download(action: &str, reply: Reply, hosts: Option<&[String]>) -> Reply {
    if action != "page.captureDownload" {
        return reply;
    }
    let Ok(data) = &reply.result else {
        return reply;
    };
    let Some(item) = data.get("item").filter(|item| item.is_object()) else {
        return reply;
    };
    let field = |key: &str| {
        item.get(key)
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned()
    };
    let captured = CapturedItem {
        browser_path: field("browserPath"),
        name: field("name"),
        mime: field("mime"),
        size: item.get("size").and_then(Value::as_f64).unwrap_or(0.0) as u64,
        url: field("url"),
        host: item.get("host").and_then(Value::as_str).map(str::to_owned),
    };
    match downloads::adopt_download(&captured, hosts) {
        Ok(record) => {
            let mut out = json!({"downloadId": record.id, "name": record.name, "mime": record.mime, "size": record.size});
            if let Some(host) = record.host {
                out["host"] = json!(host);
            }
            out["notes"] = json!(record.notes);
            out["savedTo"] = json!(record.saved_to);
            Ok(out).into()
        }
        Err((code, message)) => {
            log::log(&format!("download refused: {code}: {message}"));
            Err(ActionError { code, message }).into()
        }
    }
}

/// Actions whose `url` is where they happen.
const URL_ACTIONS: &[&str] = &[
    "page.navigate",
    "page.openTab",
    "page.openWindow",
    "page.captureDownload",
];

fn offline() -> ActionResult {
    failure(
        "EXTENSION_OFFLINE",
        "The Vexdom extension is not connected — open your browser with the extension loaded, then retry",
    )
}

impl Daemon {
    /// `invokeOn`.
    pub(crate) async fn invoke_on(
        &self,
        link: Option<Rc<Link>>,
        action: &str,
        input: Value,
        opts: &InvokeOpts,
    ) -> Reply {
        if action.starts_with(RESERVED_PREFIX) {
            return unknown_action(action).into();
        }
        if action == "page.listDownloads" {
            return Ok(downloads::list_downloads(&input)).into();
        }
        if action.starts_with("bridge.auth") {
            return crate::auth_vault::handle(self, link, action, input, opts).await;
        }
        if action.starts_with("bridge.flow") {
            return crate::flow::handle(self, link, action, input, opts).await;
        }
        let mut input = input;
        if action == "page.a11yAudit" {
            match crate::devtools::axe_source().await {
                Ok(source) => input["axeSource"] = json!(source),
                Err(error) => return Err(error).into(),
            }
        }
        if action == "page.recordStart" {
            return self.record_start(link, input, opts).await;
        }
        if action == "page.recordStop" {
            return self.record_stop(link, input, opts).await;
        }
        let artifact_path = if crate::artifacts::is_artifact(action, &input) {
            match crate::artifacts::requested_path(&input) {
                Ok(path) => Some(path),
                Err(error) => return Err(error).into(),
            }
        } else {
            None
        };
        let resolved = match downloads::resolve_attachment(action, &input) {
            Ok(resolved) => resolved,
            Err((code, message)) => return Err(ActionError { code, message }).into(),
        };
        let Some(link) = link.filter(|link| link.is_open()) else {
            return offline().into();
        };
        let tab_id = opts.tab_id;
        let run_id = opts.run_id.clone();
        let reply = if action == "page.solveCaptcha" {
            let target = Rc::clone(&link);
            let invoke: Rc<dyn Fn(Value) -> futures::future::LocalBoxFuture<'static, Reply>> =
                Rc::new(move |next| {
                    let link = Rc::clone(&target);
                    let run_id = run_id.clone();
                    Box::pin(async move {
                        link.invoke("page.solveCaptcha", &next, tab_id, run_id.as_deref())
                            .await
                    })
                });
            agent::solve_captcha_with_analyst(invoke, resolved, &read_agent_config()).await
        } else {
            link.invoke(action, &resolved, tab_id, opts.run_id.as_deref())
                .await
        };
        let reply = crate::devtools::finish(action, &input, reply);
        let reply = match artifact_path {
            Some(path) => crate::artifacts::write_artifact(path, reply),
            None => reply,
        };
        let reply = persist_screenshot(action, &input, reply, opts);
        persist_download(action, reply, opts.hosts.as_deref())
    }

    /// `page.recordStart`: ffmpeg first, so a missing encoder fails before the tab is touched.
    async fn record_start(
        &self,
        link: Option<Rc<Link>>,
        mut input: Value,
        opts: &InvokeOpts,
    ) -> Reply {
        let Some(link) = link.filter(|link| link.is_open()) else {
            return offline().into();
        };
        let start = match crate::recording_video::start_of(&input) {
            Ok(start) => start,
            Err(error) => return Err(error).into(),
        };
        let id = match self.0.video.borrow_mut().start(start) {
            Ok(id) => id,
            Err(error) => return Err(error).into(),
        };
        input["recordingId"] = json!(id);
        let reply = link
            .invoke(
                "page.recordStart",
                &input,
                opts.tab_id,
                opts.run_id.as_deref(),
            )
            .await;
        if reply.result.is_err() {
            self.0.video.borrow_mut().abandon(&id);
        }
        reply
    }

    /// `page.recordStop`: the screencast stops, then the encoder writes the file out.
    async fn record_stop(&self, link: Option<Rc<Link>>, input: Value, opts: &InvokeOpts) -> Reply {
        let done = match self.0.video.borrow_mut().stop() {
            Ok(done) => done,
            Err(error) => return Err(error).into(),
        };
        if let Some(link) = link.filter(|link| link.is_open()) {
            let _ = link
                .invoke(
                    "page.recordStop",
                    &input,
                    opts.tab_id,
                    opts.run_id.as_deref(),
                )
                .await;
        }
        match done.await {
            Ok(outcome) => outcome.into(),
            Err(_) => failure("ACTION_FAILED", "The video encoder stopped unexpectedly.").into(),
        }
    }

    /// The site an action happens on: its URL's host, else the host of the tab it targets.
    async fn site_of(
        &self,
        target: Option<&Rc<Link>>,
        action: &str,
        input: &Value,
    ) -> Option<String> {
        let host = |url: &str| {
            url::Url::parse(url)
                .ok()
                .filter(|url| matches!(url.scheme(), "http" | "https"))
                .map(|url| vexdom_core::guardrails::scope::hostname(&url))
                .filter(|host| !host.is_empty())
        };
        // Only an action that goes to its `url` happens there; any other `url` key is the caller's.
        if URL_ACTIONS.contains(&action)
            && let Some(site) = input.get("url").and_then(Value::as_str).and_then(host)
        {
            return Some(site);
        }
        let link = target.filter(|link| link.is_open())?;
        let tab = input.get("tabId").and_then(Value::as_i64);
        let here = link
            .invoke("page.query", &json!({"what": "url"}), tab, None)
            .await
            .result
            .ok()?;
        here.get("value").and_then(Value::as_str).and_then(host)
    }

    /// `invokeExternal`: an MCP client with no run is unscoped. A confirm waits for the user
    /// (ADR-032) unless the policy's `unattended` setting allows it. The browser's side panel is
    /// told about the call.
    pub(crate) async fn invoke_external(
        &self,
        target: Option<Rc<Link>>,
        action: &str,
        mut input: Value,
        client: &str,
    ) -> Reply {
        let approver = crate::confirmations::take_approver(&mut input);
        let tool_id = vexdom_core::downloads::uuid_v4();
        let tell = |event: RunEvent| {
            if let Some(link) = target.as_ref().filter(|link| link.is_open()) {
                link.run(EXTERNAL_RUN_ID, &event);
            }
        };
        tell(RunEvent::Tool {
            tool_id: tool_id.clone(),
            action: action.to_owned(),
            input: crate::auth_vault::shown(action, &input),
            source: Some("external".into()),
        });
        let config = read_agent_config();
        let policy = policy_from(&config.guardrails, &config.require_approval);
        let scope = anywhere();
        let decision = decide(
            &GuardrailRequest {
                action,
                input: &input,
                caller: Caller::External,
                scope: &scope,
            },
            &policy,
        );
        if decision.effect == Effect::Deny {
            log::log(&format!(
                "external {client} → {action} blocked: {}",
                guardrails::describe(&decision)
            ));
            tell(RunEvent::ToolResult {
                tool_id,
                ok: false,
                summary: format!("blocked: {}", guardrails::summary(&decision)),
            });
            return failure("BLOCKED", guardrails::blocked_message(&decision)).into();
        }
        if decision.effect == Effect::Confirm {
            let site = self.site_of(target.as_ref(), action, &input).await;
            let shown = site.clone();
            let asked = self
                .await_confirmation(crate::confirmations::Request {
                    action,
                    summary: guardrails::summary(&decision),
                    site,
                    input: &input,
                    approver: approver.clone(),
                })
                .await;
            // The tab may have moved on while the user was asked: what they allowed was this site.
            let asked = match asked {
                Ok(()) if self.site_of(target.as_ref(), action, &input).await != shown => {
                    Err(ActionError {
                        code: "PAGE_CHANGED".into(),
                        message: "The page changed while the user was asked; nothing was done. Ask again if it is still needed.".into(),
                    })
                }
                other => other,
            };
            if let Err(error) = asked {
                log::log(&format!(
                    "external {client} → {action} not approved: {}",
                    error.code
                ));
                tell(RunEvent::ToolResult {
                    tool_id,
                    ok: false,
                    summary: error.code.clone(),
                });
                return Err(error).into();
            }
        } else if !decision.matched.is_empty() {
            log::log(&format!(
                "external {client} → {action} waived: {}",
                guardrails::describe(&decision)
            ));
        }
        // A recording tab logs every page action it runs (saved flows).
        let observed = action.starts_with("page.").then(|| input.clone());
        let started = log::now_ms();
        let opts = InvokeOpts {
            approver,
            ..InvokeOpts::default()
        };
        let reply = self.invoke_on(target.clone(), action, input, &opts).await;
        if let Some(observed) = observed {
            let tab = observed.get("tabId").and_then(Value::as_i64);
            crate::flow::observe(
                self,
                tab,
                action,
                &observed,
                started,
                log::now_ms(),
                reply.result.is_ok(),
            );
        }
        log::log(&format!(
            "external {client} → {action} {}",
            match &reply.result {
                Ok(_) => "ok".to_owned(),
                Err(error) => error.code.clone(),
            }
        ));
        tell(RunEvent::ToolResult {
            tool_id,
            ok: reply.result.is_ok(),
            summary: match &reply.result {
                Ok(_) => "ok".into(),
                Err(error) => seal_secrets(&format!("{}: {}", error.code, error.message)),
            },
        });
        reply
    }
}
