//! Conversions between the capnp wire (`vexdom.capnp`) and the daemon's Rust values.

use serde_json::Value;
use vexdom_core::result::{ActionError, ActionResult};
use vexdom_schema::{action_result, run_event};

/// `Json` text → value; empty or invalid text is `null`.
pub fn parse_json(text: &str) -> Value {
    if text.is_empty() {
        return Value::Null;
    }
    serde_json::from_str(text).unwrap_or(Value::Null)
}

pub fn json_text(value: &Value) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "null".into())
}

/// A capnp text field as a `String`; invalid UTF-8 and errors read as empty.
pub fn text(reader: capnp::Result<capnp::text::Reader<'_>>) -> String {
    reader
        .ok()
        .and_then(|text| text.to_str().ok().map(str::to_owned))
        .unwrap_or_default()
}

/// Empty text means absent (schema convention).
pub fn optional(text: String) -> Option<String> {
    (!text.is_empty()).then_some(text)
}

pub fn read_result(reader: capnp::Result<action_result::Reader<'_>>) -> ActionResult {
    let reader = reader.map_err(|error| ActionError {
        code: "BAD_RESULT".into(),
        message: error.to_string(),
    })?;
    match reader.which() {
        Ok(action_result::Ok(json)) => Ok(parse_json(&text(json))),
        Ok(action_result::Error(error)) => {
            let error = error.map_err(|error| ActionError {
                code: "BAD_RESULT".into(),
                message: error.to_string(),
            })?;
            Err(ActionError {
                code: text(error.get_code()),
                message: text(error.get_message()),
            })
        }
        Err(error) => Err(ActionError {
            code: "BAD_RESULT".into(),
            message: error.to_string(),
        }),
    }
}

pub fn write_result(mut builder: action_result::Builder<'_>, result: &ActionResult) {
    match result {
        Ok(data) => builder.set_ok(json_text(data)),
        Err(error) => {
            let mut out = builder.init_error();
            out.set_code(&error.code);
            out.set_message(&error.message);
        }
    }
}

/// `RunEvent` (`src/lib/actions/protocol.ts`): what a run tells the side panel.
#[derive(Debug, Clone, PartialEq)]
pub enum RunEvent {
    Started {
        skill: String,
        attached: Option<Value>,
        overlays: Option<Value>,
    },
    Text(String),
    Tool {
        tool_id: String,
        action: String,
        input: Value,
        source: Option<String>,
    },
    Approval {
        tool_id: String,
        action: String,
        input: Value,
        site: Option<String>,
    },
    ToolResult {
        tool_id: String,
        ok: bool,
        summary: String,
    },
    Session {
        agent: String,
        agent_session_id: Option<String>,
    },
    Attachments(Value),
    Usage {
        context_tokens: u64,
        output_tokens: u64,
    },
    Done(String),
    Error(ActionError),
}

fn json_or_empty(value: &Option<Value>) -> String {
    value.as_ref().map(json_text).unwrap_or_default()
}

pub fn write_run_event(mut builder: run_event::Builder<'_>, event: &RunEvent) {
    match event {
        RunEvent::Started {
            skill,
            attached,
            overlays,
        } => {
            let mut started = builder.init_started();
            started.set_skill(skill);
            started.set_attached(json_or_empty(attached));
            started.set_overlays(json_or_empty(overlays));
        }
        RunEvent::Text(delta) => builder.set_text(delta),
        RunEvent::Tool {
            tool_id,
            action,
            input,
            source,
        } => {
            let mut tool = builder.init_tool();
            tool.set_tool_id(tool_id);
            tool.set_action(action);
            tool.set_input(json_text(input));
            tool.set_source(source.as_deref().unwrap_or(""));
        }
        RunEvent::Approval {
            tool_id,
            action,
            input,
            site,
        } => {
            let mut approval = builder.init_approval();
            approval.set_tool_id(tool_id);
            approval.set_action(action);
            approval.set_input(json_text(input));
            approval.set_site(site.as_deref().unwrap_or(""));
        }
        RunEvent::ToolResult {
            tool_id,
            ok,
            summary,
        } => {
            let mut result = builder.init_tool_result();
            result.set_tool_id(tool_id);
            result.set_ok(*ok);
            result.set_summary(summary);
        }
        RunEvent::Session {
            agent,
            agent_session_id,
        } => {
            let mut session = builder.init_session();
            session.set_agent(agent);
            session.set_agent_session_id(agent_session_id.as_deref().unwrap_or(""));
        }
        RunEvent::Attachments(files) => builder.set_attachments(json_text(files)),
        RunEvent::Usage {
            context_tokens,
            output_tokens,
        } => {
            let mut usage = builder.init_usage();
            usage.set_context_tokens(*context_tokens);
            usage.set_output_tokens(*output_tokens);
        }
        RunEvent::Done(stop_reason) => builder.set_done(stop_reason),
        RunEvent::Error(error) => {
            let mut out = builder.init_error();
            out.set_code(&error.code);
            out.set_message(&error.message);
        }
    }
}

/// axum's WebSocket message carries the capnp bytes on the daemon side.
impl vexdom_link::Frame for WsMessage {
    fn binary(bytes: Vec<u8>) -> Self {
        WsMessage(axum::extract::ws::Message::Binary(bytes.into()))
    }

    fn classify(self) -> vexdom_link::Incoming {
        use axum::extract::ws::Message;
        match self.0 {
            Message::Binary(bytes) => vexdom_link::Incoming::Data(bytes),
            Message::Text(_) => vexdom_link::Incoming::Text,
            Message::Close(_) => vexdom_link::Incoming::End,
            Message::Ping(_) | Message::Pong(_) => vexdom_link::Incoming::Ignore,
        }
    }
}

/// A newtype so the daemon can implement the link's `Frame` for axum's message.
pub struct WsMessage(pub axum::extract::ws::Message);
