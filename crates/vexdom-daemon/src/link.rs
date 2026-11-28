//! One paired browser (`src/daemon/extension-link.ts`): the extension's `Page` and `ExtensionUi`
//! capabilities, per-action timeouts, and the pushes the side panel listens for.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::{Duration, Instant};

use serde_json::Value;
use vexdom_core::result::{ActionError, ActionResult, failure};
use vexdom_schema::{extension_ui, page};

use crate::wire::{self, RunEvent};

const DEFAULT_TIMEOUT_MS: u64 = 30_000;
const DESCRIBE_TIMEOUT_MS: u64 = 10_000;
const RUN_TASK_TIMEOUT_MS: u64 = 30_000;
const SCREENSHOT_TIMEOUT_MS: u64 = 120_000;
const AWAIT_DEFAULT_TIMEOUT_MS: u64 = 120_000;
const PICK_DEFAULT_TIMEOUT_MS: u64 = 60_000;
const SOLVE_CAPTCHA_TIMEOUT_MS: u64 = 120_000;

/// The socket a link (or a handshake in progress) runs on: closing it drops the RPC system.
#[derive(Default)]
pub struct Conn {
    pub abort: RefCell<Option<tokio::task::AbortHandle>>,
}

impl Conn {
    pub fn close(&self) {
        if let Some(abort) = self.abort.borrow_mut().take() {
            abort.abort();
        }
    }
}

/// What a page call answers: the result, and a screenshot's bytes when there are any.
#[derive(Debug, Clone, PartialEq)]
pub struct Reply {
    pub result: ActionResult,
    pub image: Option<Vec<u8>>,
}

impl From<ActionResult> for Reply {
    fn from(result: ActionResult) -> Self {
        Self {
            result,
            image: None,
        }
    }
}

/// Runs once when a link ends.
type OnClose = Box<dyn FnOnce(&Link)>;

pub struct Link {
    /// Unique per connection, so a superseded link is never mistaken for its successor.
    pub serial: u64,
    pub install_id: String,
    pub origin: String,
    pub extension_version: String,
    pub manifest_hash: String,
    pub browser: Option<String>,
    pub protocol_version: u32,
    page: page::Client,
    ui: extension_ui::Client,
    /// What this browser offers: a Firefox build lists fewer tools, a drifted build its own.
    pub tools: RefCell<Rc<Vec<Value>>>,
    /// A daemon serial taken each time `tools` is set: part of `describe`'s stamp.
    pub tools_serial: Cell<u64>,
    pub last_active: Cell<u64>,
    pub last_ping: Cell<Instant>,
    closed: RefCell<Option<String>>,
    conn: Rc<Conn>,
    on_close: RefCell<Option<OnClose>>,
}

pub struct Hello {
    pub install_id: String,
    pub origin: String,
    pub extension_version: String,
    pub manifest_hash: String,
    pub browser: Option<String>,
    pub protocol_version: u32,
}

impl Link {
    pub fn new(
        serial: u64,
        hello: Hello,
        page: page::Client,
        ui: extension_ui::Client,
        conn: Rc<Conn>,
        last_active: u64,
    ) -> Self {
        Self {
            serial,
            install_id: hello.install_id,
            origin: hello.origin,
            extension_version: hello.extension_version,
            manifest_hash: hello.manifest_hash,
            browser: hello.browser,
            protocol_version: hello.protocol_version,
            page,
            ui,
            tools: RefCell::new(Rc::new(Vec::new())),
            tools_serial: Cell::new(0),
            last_active: Cell::new(last_active),
            last_ping: Cell::new(Instant::now()),
            closed: RefCell::new(None),
            conn,
            on_close: RefCell::new(None),
        }
    }

    pub fn set_on_close(&self, callback: impl FnOnce(&Link) + 'static) {
        *self.on_close.borrow_mut() = Some(Box::new(callback));
    }

    pub fn label(&self) -> &str {
        self.browser.as_deref().unwrap_or(&self.origin)
    }

    pub fn speaks(&self, version: u32) -> bool {
        self.protocol_version >= version
    }

    pub fn is_open(&self) -> bool {
        self.closed.borrow().is_none()
    }

    /// Ends the link: pending calls fail, the socket goes.
    pub fn close(&self, reason: &str) {
        self.dispose(reason);
        self.conn.close();
    }

    /// The connection ended (or is being ended): run the close callback once.
    pub fn dispose(&self, reason: &str) {
        if self.closed.borrow().is_some() {
            return;
        }
        *self.closed.borrow_mut() = Some(reason.to_owned());
        if let Some(callback) = self.on_close.borrow_mut().take() {
            callback(self);
        }
    }

    fn offline(&self, error: &capnp::Error) -> ActionError {
        let reason = self
            .closed
            .borrow()
            .clone()
            .unwrap_or_else(|| error.to_string());
        ActionError {
            code: "EXTENSION_OFFLINE".into(),
            message: format!("Connection lost: {reason}"),
        }
    }

    fn not_connected() -> ActionResult {
        failure("EXTENSION_OFFLINE", "The Vexdom extension is not connected")
    }

    fn timed_out(ms: u64) -> ActionError {
        ActionError {
            code: "TIMEOUT".into(),
            message: format!("The extension did not respond within {ms}ms"),
        }
    }

    /// Any page action; `page.screenshot` goes through `Page.screenshot` so the image is bytes.
    pub async fn invoke(
        &self,
        action: &str,
        input: &Value,
        tab_id: Option<i64>,
        run_id: Option<&str>,
    ) -> Reply {
        if !self.is_open() {
            return Self::not_connected().into();
        }
        let ms = timeout_for(action, input);
        if action == "page.screenshot" {
            let mut request = self.page.screenshot_request();
            request.get().set_input(wire::json_text(input));
            request.get().set_tab_id(tab_id.unwrap_or(-1));
            let call = request.send().promise;
            return match tokio::time::timeout(Duration::from_millis(ms), call).await {
                Err(_) => Err(Self::timed_out(ms)).into(),
                Ok(Err(error)) => Err(self.offline(&error)).into(),
                Ok(Ok(response)) => match response.get() {
                    Ok(reply) => {
                        let result = wire::read_result(reply.get_result());
                        let image = reply
                            .get_image()
                            .ok()
                            .filter(|bytes| !bytes.is_empty())
                            .map(<[u8]>::to_vec);
                        Reply { result, image }
                    }
                    Err(error) => Err(self.offline(&error)).into(),
                },
            };
        }
        if crate::artifacts::is_artifact(action, input) {
            let mut request = self.page.artifact_request();
            request.get().set_action(action);
            request.get().set_input(wire::json_text(input));
            request.get().set_tab_id(tab_id.unwrap_or(-1));
            let call = request.send().promise;
            return match tokio::time::timeout(Duration::from_millis(ms), call).await {
                Err(_) => Err(Self::timed_out(ms)).into(),
                Ok(Err(error)) => Err(self.offline(&error)).into(),
                Ok(Ok(response)) => match response.get() {
                    Ok(reply) => Reply {
                        result: wire::read_result(reply.get_result()),
                        image: reply.get_data().ok().map(<[u8]>::to_vec),
                    },
                    Err(error) => Err(self.offline(&error)).into(),
                },
            };
        }
        let mut request = self.page.invoke_request();
        {
            let mut params = request.get();
            params.set_action(action);
            params.set_input(wire::json_text(input));
            params.set_tab_id(tab_id.unwrap_or(-1));
            params.set_run_id(run_id.unwrap_or(""));
        }
        let call = request.send().promise;
        match tokio::time::timeout(Duration::from_millis(ms), call).await {
            Err(_) => Err(Self::timed_out(ms)).into(),
            Ok(Err(error)) => Err(self.offline(&error)).into(),
            Ok(Ok(response)) => match response.get() {
                Ok(reply) => wire::read_result(reply.get_result()).into(),
                Err(error) => Err(self.offline(&error)).into(),
            },
        }
    }

    /// The extension's own tool list, for a build whose manifest drifted.
    pub async fn describe(&self) -> Option<Vec<Value>> {
        if !self.is_open() {
            return None;
        }
        let call = self.page.describe_request().send().promise;
        let response = tokio::time::timeout(Duration::from_millis(DESCRIBE_TIMEOUT_MS), call)
            .await
            .ok()?
            .ok()?;
        let tools = wire::parse_json(&wire::text(response.get().ok()?.get_tools()));
        tools.as_array().cloned()
    }

    pub async fn run_task(&self, order: &Value) -> ActionResult {
        if !self.is_open() {
            return Self::not_connected();
        }
        let mut request = self.page.run_task_request();
        request.get().set_order(wire::json_text(order));
        let call = request.send().promise;
        match tokio::time::timeout(Duration::from_millis(RUN_TASK_TIMEOUT_MS), call).await {
            Err(_) => Err(Self::timed_out(RUN_TASK_TIMEOUT_MS)),
            Ok(Err(error)) => Err(self.offline(&error)),
            Ok(Ok(response)) => match response.get() {
                Ok(reply) => wire::read_result(reply.get_result()),
                Err(error) => Err(self.offline(&error)),
            },
        }
    }

    /// Pushes are fire-and-forget; a failed one is logged.
    fn push<T: 'static>(
        &self,
        call: impl std::future::Future<Output = Result<T, capnp::Error>> + 'static,
    ) {
        tokio::task::spawn_local(async move {
            if let Err(error) = call.await {
                vexdom_core::log::log_detail("failed to push to the extension", &error.to_string());
            }
        });
    }

    pub fn run(&self, id: &str, event: &RunEvent) {
        if !self.is_open() {
            return;
        }
        let mut request = self.ui.run_request();
        request.get().set_id(id);
        wire::write_run_event(request.get().init_event(), event);
        self.push(request.send().promise);
    }

    pub fn site_map_draft(&self, id: &str, draft: &Value) {
        if !self.is_open() {
            return;
        }
        let mut request = self.ui.site_map_draft_request();
        request.get().set_id(id);
        request.get().set_draft(wire::json_text(draft));
        self.push(request.send().promise);
    }

    pub fn open_notice(&self, notice_id: &str) {
        if !self.is_open() {
            return;
        }
        let mut request = self.ui.open_notice_request();
        request.get().set_notice_id(notice_id);
        self.push(request.send().promise);
    }

    pub fn agent_info(&self, result: &ActionResult) {
        if !self.is_open() {
            return;
        }
        let mut request = self.ui.agent_info_request();
        wire::write_result(request.get().init_result(), result);
        self.push(request.send().promise);
    }

    pub fn skill_catalog(&self, result: &ActionResult) {
        if !self.is_open() {
            return;
        }
        let mut request = self.ui.skill_catalog_request();
        wire::write_result(request.get().init_result(), result);
        self.push(request.send().promise);
    }

    pub fn preferences_info(&self, result: &ActionResult) {
        if !self.is_open() {
            return;
        }
        let mut request = self.ui.preferences_info_request();
        wire::write_result(request.get().init_result(), result);
        self.push(request.send().promise);
    }

    pub fn task_list(&self, result: &ActionResult) {
        if !self.is_open() {
            return;
        }
        let mut request = self.ui.task_list_request();
        wire::write_result(request.get().init_result(), result);
        self.push(request.send().promise);
    }
}

fn declared_timeout(input: &Value) -> Option<u64> {
    input
        .get("timeoutMs")
        .and_then(Value::as_f64)
        .filter(|ms| *ms > 0.0)
        .map(|ms| ms as u64)
}

/// `typingDurationMs`: the pace a `page.typeText` will take, so its timeout covers it.
pub fn typing_duration_ms(input: &Value) -> u64 {
    let Some(text) = input
        .get("text")
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())
    else {
        return 0;
    };
    let base = match input.get("charDelayMs").and_then(Value::as_f64) {
        Some(ms) => ms,
        None => match input
            .get("speed")
            .and_then(Value::as_str)
            .unwrap_or("natural")
        {
            "slow" => 400.0,
            "fast" => 110.0,
            "instant" => 0.0,
            _ => 220.0,
        },
    };
    let jitter = input.get("jitter").and_then(Value::as_f64).unwrap_or(0.35);
    let total: f64 = text
        .chars()
        .map(|c| {
            base * match c {
                '.' | '?' | '!' => 3.0,
                '\n' => 2.5,
                ',' | ';' | ':' => 2.0,
                _ => 1.0,
            }
        })
        .sum();
    (total * (1.0 + jitter)).round() as u64
}

/// `timeoutFor` in `extension-link.ts`.
pub fn timeout_for(action: &str, input: &Value) -> u64 {
    match action {
        "page.screenshot" => SCREENSHOT_TIMEOUT_MS,
        "page.typeText" => typing_duration_ms(input) + DEFAULT_TIMEOUT_MS,
        "page.startMonitor" => DEFAULT_TIMEOUT_MS,
        "page.awaitMonitor" => declared_timeout(input).unwrap_or(AWAIT_DEFAULT_TIMEOUT_MS) + 5_000,
        "page.pickElement" => declared_timeout(input).unwrap_or(PICK_DEFAULT_TIMEOUT_MS) + 5_000,
        "page.solveCaptcha" => declared_timeout(input).unwrap_or(SOLVE_CAPTCHA_TIMEOUT_MS) + 5_000,
        // A trace is read back in chunks after a stop that may itself wait 30 s.
        "page.traceStop" | "page.pdf" | "page.harStop" | "page.vitals" => SCREENSHOT_TIMEOUT_MS,
        "page.a11yAudit" | "page.react" => SCREENSHOT_TIMEOUT_MS,
        _ => declared_timeout(input).map_or(DEFAULT_TIMEOUT_MS, |ms| ms + 5_000),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn timeouts() {
        assert_eq!(timeout_for("page.screenshot", &json!({})), 120_000);
        assert_eq!(
            timeout_for("page.clickElement", &json!({"timeoutMs": 1000})),
            6_000
        );
        assert_eq!(timeout_for("page.clickElement", &json!({})), 30_000);
        assert_eq!(timeout_for("page.awaitMonitor", &json!({})), 125_000);
        assert_eq!(
            typing_duration_ms(&json!({"text": "ab.", "jitter": 0})),
            220 + 220 + 660
        );
        assert_eq!(
            timeout_for("page.typeText", &json!({"text": "a", "speed": "instant"})),
            30_000
        );
    }
}
