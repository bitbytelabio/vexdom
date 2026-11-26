//! `vexdom mcp`: a stdio MCP server on rmcp (ADR-029), dual era — 2026-07-28 (`server/discover`,
//! stateless requests) and the legacy `initialize` handshake. Three tools (`snapshot`, `act`,
//! `guide`), the playbooks as prompts, `skill://` resources and skills-extension entries, and the
//! active page as `vexdom://page/*` resources.

pub mod host;

use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, HashMap};
use std::process::ExitCode;
use std::rc::Rc;
use std::time::Duration;

use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, ElicitRequestParams,
    GetPromptRequestParams, GetPromptResponse, GetPromptResult, GetSkillRequestParams,
    GetSkillResult, Implementation, InputRequest, InputRequiredResult, InputResponses,
    ListPromptsResult, ListResourcesResult, ListSkillsResult, ListToolsResult,
    PaginatedRequestParams, Prompt, PromptMessage, ReadResourceRequestParams, ReadResourceResponse,
    ReadResourceResult, Resource, ResourceContents, Role, ServerCapabilities, ServerConfig, Skill,
    SkillFrontmatter, SkillResource, SkillResources, Tool, ToolAnnotations,
};
use rmcp::service::RequestContext;
use rmcp::{ErrorData, RoleServer, ServerHandler, ServiceExt as _};
use serde_json::{Value, json};
use tokio::sync::oneshot;
use vexdom_core::{log, skills};
use vexdom_schema::Topic;

use host::{Host, ToolReply};
use vexdom_client::Bridge;

/// The most a tool result says, in chars of its body (about 5,000 tokens): a whole page's tree or
/// text past this is cut with a note saying how to narrow it.
pub const DEFAULT_MAX_OUTPUT: usize = 20_000;

pub struct Options {
    pub max_output: Option<usize>,
}

const RESOURCES: [(&str, &str, &str, &str); 3] = [
    (
        "vexdom://page/current",
        "Active page snapshot",
        "Full page.getPageInfo snapshot of the active tab: metadata, layout diagram, headings, interactive inventory.",
        "application/json",
    ),
    (
        "vexdom://page/diagram",
        "Active page layout diagram",
        "Text diagram of the active tab’s landmark regions — the cheapest useful view of a page.",
        "text/plain",
    ),
    (
        "vexdom://page/text",
        "Active page text",
        "Rendered text of the active tab.",
        "text/plain",
    ),
];

pub fn run(opts: Options) -> ExitCode {
    vexdom_client::block_on(async move {
        match serve(opts).await {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("vexdom mcp: {error}");
                ExitCode::FAILURE
            }
        }
    })
}

async fn serve(opts: Options) -> Result<(), String> {
    let run_id = std::env::var("VEXDOM_AGENT_RUN")
        .ok()
        .filter(|id| !id.is_empty());
    let bridge = Bridge::connect(run_id).await?;
    let result_bytes = std::env::var("VEXDOM_RESULT_BYTES")
        .ok()
        .and_then(|bytes| bytes.parse().ok())
        .filter(|b| *b > 0);
    // Unset: DEFAULT_MAX_OUTPUT; 0: no limit.
    let max_output = match opts.max_output.or_else(|| {
        std::env::var("VEXDOM_MAX_OUTPUT")
            .ok()
            .and_then(|max| max.parse().ok())
            .or_else(|| crate::cli::project_prefs().max_output)
    }) {
        Some(0) => None,
        Some(max) => Some(max),
        None => Some(DEFAULT_MAX_OUTPUT),
    };
    let server = Server {
        host: Rc::new(Host::new(bridge, max_output, result_bytes)),
        session: vexdom_core::downloads::uuid_v4(),
        calls: Cell::new(0),
        parked: Rc::default(),
    };
    log::log("MCP server on stdio");
    let running = server
        .serve(rmcp::transport::stdio())
        .await
        .map_err(|error| error.to_string())?;
    running.waiting().await.map_err(|error| error.to_string())?;
    Ok(())
}

pub struct Server {
    host: Rc<Host>,
    /// Approver tags read `mcp:<session>:<call>`.
    session: String,
    calls: Cell<u64>,
    /// 2026-07-28 calls parked on an approval the client was asked about, by confirmation id.
    parked: Rc<RefCell<HashMap<String, Pending>>>,
}

/// What the user is asked when an action waits for approval.
fn approval_form(confirmation: &Value) -> Option<ElicitRequestParams> {
    let field = |key: &str| confirmation.get(key).and_then(Value::as_str).unwrap_or("");
    let site = match field("site") {
        "" => String::new(),
        site => format!(" on {site}"),
    };
    let message = format!(
        "Allow {}{site}? {}",
        vexdom_core::tools::short_name(field("action")),
        field("summary")
    );
    serde_json::from_value(json!({
        "mode": "form",
        "message": message,
        "requestedSchema": {
            "type": "object",
            "properties": {
                "decision": {
                    "type": "string",
                    "title": "Decision",
                    "enum": ["allow", "always", "deny"],
                    "enumNames": ["Allow once", "Always allow on this site", "Deny"]
                }
            },
            "required": ["decision"]
        }
    }))
    .ok()
}

/// An elicitation answer as `(allow, remember)`; `None` when the user dismissed it.
fn decision_of(answer: &Value) -> Option<(bool, bool)> {
    match answer.get("action").and_then(Value::as_str) {
        Some("accept") => match answer.pointer("/content/decision").and_then(Value::as_str) {
            Some("allow") => Some((true, false)),
            Some("always") => Some((true, true)),
            _ => Some((false, false)),
        },
        Some("decline") => Some((false, false)),
        _ => None,
    }
}

fn tool_of(definition: Value) -> Tool {
    let text = |key: &str| {
        definition
            .get(key)
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned()
    };
    let schema = definition
        .get("inputSchema")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let mut tool = Tool::new_with_raw(text("name"), Some(text("description").into()), schema)
        .with_title(text("title"));
    if let Some(annotations) = definition
        .get("annotations")
        .cloned()
        .and_then(|value| serde_json::from_value::<ToolAnnotations>(value).ok())
    {
        tool = tool.with_annotations(annotations);
    }
    tool
}

/// A host reply as an MCP tool result.
pub fn result_of(reply: ToolReply) -> CallToolResult {
    let content: Vec<ContentBlock> = reply
        .content
        .iter()
        .map(|part| {
            let field = |key: &str| part.get(key).and_then(Value::as_str).unwrap_or("");
            match field("type") {
                "image" => ContentBlock::image(field("data"), field("mimeType")),
                _ => ContentBlock::text(field("text")),
            }
        })
        .collect();
    if reply.is_error {
        CallToolResult::error(content)
    } else {
        CallToolResult::success(content)
    }
}

/// The playbooks as skills-extension entries.
fn skill_entries() -> Vec<Skill> {
    skills::mcp_skills()
        .iter()
        .map(|skill| {
            Skill::new(
                skill.uri(skills::SKILL_FILE),
                SkillFrontmatter::new(&skill.name, &skill.description),
                SkillResources::Files(
                    skill
                        .files
                        .iter()
                        .map(|(path, body)| {
                            SkillResource::from_bytes(skill.uri(path), body.as_bytes())
                        })
                        .collect(),
                ),
            )
        })
        .collect()
}

/// A `skill://<name>/<path>` file's body.
fn skill_file(uri: &str) -> Option<&'static str> {
    let (name, path) = uri.strip_prefix("skill://")?.split_once('/')?;
    skills::mcp_skills()
        .into_iter()
        .find(|skill| skill.name == name)?
        .files
        .into_iter()
        .find(|(file, _)| file == path)
        .map(|(_, body)| body)
}

/// A finished call's reply, or why there is none.
fn finished(reply: Result<ToolReply, oneshot::error::RecvError>) -> ToolReply {
    reply.unwrap_or_else(|_| ToolReply::error(&vexdom_client::unreachable()))
}

impl Server {
    /// A 2026-07-28 client's next call, with its user's answer to the approval it was asked.
    async fn resume(
        &self,
        state: &str,
        answers: Option<&InputResponses>,
        context: &RequestContext<RoleServer>,
    ) -> CallToolResponse {
        let Some(pending) = self.parked.borrow_mut().remove(state) else {
            return CallToolResponse::Complete(result_of(ToolReply::error(
                &vexdom_core::result::ActionError {
                    code: "CONFIRM_EXPIRED".into(),
                    message: "That approval is no longer waiting; call the tool again.".into(),
                },
            )));
        };
        if let Some((allow, remember)) = answers
            .and_then(|answers| answers.get("confirm"))
            .and_then(decision_of)
        {
            self.host.bridge.confirm(state, allow, remember).await;
        }
        self.drive(pending, context).await
    }

    /// Waits for the call, asking the client about every approval it meets on the way (a batch
    /// can meet several).
    async fn drive(
        &self,
        mut pending: Pending,
        context: &RequestContext<RoleServer>,
    ) -> CallToolResponse {
        loop {
            let confirmation = tokio::select! {
                answered = &mut pending.reply => {
                    return CallToolResponse::Complete(result_of(finished(answered)));
                }
                Some(confirmation) = pending.appeared.recv() => confirmation,
            };
            let id = confirmation
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned();
            // The list is re-sent on every change: ask about each request once.
            if pending.asked.contains(&id) {
                continue;
            }
            pending.asked.push(id.clone());
            let Some(form) = approval_form(&confirmation) else {
                continue;
            };
            // 2026-07-28 forbids asking mid-request: the client asks and calls again.
            if context.meta.protocol_version().is_some() {
                self.park(id.clone(), pending);
                let requests = BTreeMap::from([(
                    "confirm".to_owned(),
                    InputRequest::Elicitation(rmcp::model::Request::new(form)),
                )]);
                return CallToolResponse::InputRequired(InputRequiredResult::new(
                    Some(requests),
                    Some(id),
                ));
            }
            let wait = Some(Duration::from_millis(vexdom_core::proto::CONFIRM_WAIT_MS));
            if let Ok(answer) = context
                .peer
                .create_elicitation_with_timeout(form, wait)
                .await
                && let Some((allow, remember)) =
                    decision_of(&serde_json::to_value(answer).unwrap_or_default())
            {
                self.host.bridge.confirm(&id, allow, remember).await;
            }
        }
    }

    /// Keeps a call for the client's next request; one that never comes is dropped once the
    /// approval it waits on has timed out.
    fn park(&self, id: String, pending: Pending) {
        self.parked.borrow_mut().insert(id.clone(), pending);
        let parked = Rc::clone(&self.parked);
        tokio::task::spawn_local(async move {
            tokio::time::sleep(Duration::from_millis(
                vexdom_core::proto::CONFIRM_WAIT_MS + 10_000,
            ))
            .await;
            parked.borrow_mut().remove(&id);
        });
    }
}

/// A call waiting on approvals: its reply, the approvals it meets, and those already asked.
struct Pending {
    reply: oneshot::Receiver<ToolReply>,
    appeared: tokio::sync::mpsc::UnboundedReceiver<Value>,
    asked: Vec<String>,
    _watch: Option<vexdom_schema::handle::Client>,
}

impl ServerHandler for Server {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(
            ServerCapabilities::builder()
                .enable_prompts()
                .enable_resources()
                .enable_tools()
                .enable_skills()
                .build(),
        )
        .with_server_info(Implementation::new("vexdom", vexdom_daemon::VERSION))
        .with_instructions(host::instructions())
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        Ok(ListToolsResult::with_all_items(
            host::tool_definitions().into_iter().map(tool_of).collect(),
        ))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        if let Some(state) = &request.request_state {
            return Ok(self
                .resume(state, request.input_responses.as_ref(), &context)
                .await);
        }
        let mut args = Value::Object(request.arguments.unwrap_or_default());
        // A run's approvals belong to the side panel; only `act` changes a page.
        if self.host.bridge.run_id().is_some() || request.name != host::ACT_TOOL {
            let reply = self.host.call(&request.name, args).await;
            return Ok(CallToolResponse::Complete(result_of(reply)));
        }
        // Tagged even when the client cannot ask: the call then waits as long as the approval.
        self.calls.set(self.calls.get() + 1);
        let tag = format!("mcp:{}:{}", self.session, self.calls.get());
        args[host::APPROVER_KEY] = json!(tag);
        let can_ask = context
            .client_capabilities()
            .is_some_and(|caps| caps.elicitation.is_some());
        if !can_ask {
            let reply = self.host.call(&request.name, args).await;
            return Ok(CallToolResponse::Complete(result_of(reply)));
        }
        // Watch first, so a request made the instant the call starts is not missed.
        let (seen, appeared) = tokio::sync::mpsc::unbounded_channel::<Value>();
        let watch = self
            .host
            .bridge
            .watch(Topic::ConfirmationsChanged, None, false, move |_, list| {
                for confirmation in list.as_array().into_iter().flatten() {
                    if confirmation.get("approver").and_then(Value::as_str) == Some(tag.as_str()) {
                        let _ = seen.send(confirmation.clone());
                    }
                }
            })
            .await
            .ok();
        let (done, reply) = oneshot::channel();
        let host = Rc::clone(&self.host);
        let name = request.name.to_string();
        tokio::task::spawn_local(async move {
            let _ = done.send(host.call(&name, args).await);
        });
        let pending = Pending {
            reply,
            appeared,
            asked: Vec::new(),
            _watch: watch,
        };
        Ok(self.drive(pending, &context).await)
    }

    async fn list_prompts(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListPromptsResult, ErrorData> {
        Ok(ListPromptsResult::with_all_items(
            skills::mcp_skills()
                .into_iter()
                .map(|skill| Prompt::new(skill.name, Some(skill.description), None))
                .collect(),
        ))
    }

    async fn get_prompt(
        &self,
        request: GetPromptRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<GetPromptResponse, ErrorData> {
        let skill = skills::mcp_skills()
            .into_iter()
            .find(|skill| skill.name == request.name)
            .ok_or_else(|| {
                ErrorData::invalid_params(format!("unknown prompt {:?}", request.name), None)
            })?;
        let mut result =
            GetPromptResult::new(vec![PromptMessage::new_text(Role::User, skill.body())]);
        result.description = Some(skill.description);
        Ok(GetPromptResponse::Complete(result))
    }

    async fn list_resources(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListResourcesResult, ErrorData> {
        let mut resources: Vec<Resource> = RESOURCES
            .iter()
            .map(|(uri, name, description, mime)| {
                Resource::new(*uri, *name)
                    .with_description(*description)
                    .with_mime_type(*mime)
            })
            .collect();
        for skill in skills::mcp_skills() {
            for (path, body) in &skill.files {
                resources.push(
                    Resource::new(skill.uri(path), format!("{}/{path}", skill.name))
                        .with_mime_type("text/markdown")
                        .with_size(body.len() as u64),
                );
            }
        }
        Ok(ListResourcesResult::with_all_items(resources))
    }

    async fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResponse, ErrorData> {
        let uri = request.uri.as_str();
        let (text, mime) = match skill_file(uri) {
            Some(body) => (body.to_owned(), "text/markdown"),
            None => read_page_resource(&self.host, uri)
                .await
                .map_err(|message| ErrorData::invalid_params(message, None))?,
        };
        Ok(ReadResourceResponse::Complete(ReadResourceResult::new(
            vec![ResourceContents::text(text, uri).with_mime_type(mime)],
        )))
    }

    async fn list_skills(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListSkillsResult, ErrorData> {
        Ok(ListSkillsResult::with_all_items(skill_entries()))
    }

    async fn get_skill(
        &self,
        request: GetSkillRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<GetSkillResult, ErrorData> {
        skill_entries()
            .into_iter()
            .find(|skill| skill.uri == request.uri)
            .map(GetSkillResult::new)
            .ok_or_else(|| ErrorData::invalid_params(format!("no skill at {}", request.uri), None))
    }
}

async fn read_page_resource(host: &Host, uri: &str) -> Result<(String, &'static str), String> {
    let Some((_, _, _, mime)) = RESOURCES.iter().find(|(known, ..)| *known == uri) else {
        return Err(format!("Unknown resource: {uri}"));
    };
    let (action, input) = if uri == "vexdom://page/text" {
        ("page.extractText", json!({"format": "text"}))
    } else {
        (
            "page.getPageInfo",
            json!({"maxPerKind": if uri == "vexdom://page/diagram" { 1 } else { 30 }}),
        )
    };
    let data = host
        .bridge
        .invoke(action, &input)
        .await
        .result
        .map_err(|error| format!("{}: {}", error.code, error.message))?;
    let body = match uri {
        "vexdom://page/text" => data
            .get("content")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned(),
        "vexdom://page/diagram" => data
            .get("layout")
            .and_then(|l| l.get("diagram"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned(),
        _ => serde_json::to_string_pretty(&data).unwrap_or_default(),
    };
    let config = vexdom_core::config::read_agent_config();
    let policy = vexdom_core::guardrails::policy_from(&config.guardrails, &config.require_approval);
    let sealed = vexdom_core::guardrails::secrets::seal_secrets(&body);
    let text = if policy.fence.enabled {
        vexdom_core::guardrails::fence::fence(&sealed, &vexdom_core::guardrails::fence::fence_tag())
    } else {
        sealed
    };
    Ok((text, mime))
}

/// `vexdom mcp call <tool> [json]`: one tool call through the same host an MCP client talks to, for
/// trying a tool from a shell. Prints each text part; an image part prints as its type and size.
pub fn call(tool: &str, args: Option<&str>) -> ExitCode {
    let args: Value = match args.map(serde_json::from_str::<Value>) {
        None => json!({}),
        Some(Ok(args)) if args.is_object() => args,
        Some(_) => {
            eprintln!(
                "The arguments are one JSON object, e.g. '{{\"do\":\"navigate\",\"url\":\"https://example.com\"}}'"
            );
            return ExitCode::FAILURE;
        }
    };
    vexdom_client::block_on(async move {
        let bridge = match Bridge::connect(None).await {
            Ok(bridge) => bridge,
            Err(error) => {
                eprintln!("{error}");
                return ExitCode::FAILURE;
            }
        };
        let host = Host::new(bridge, Some(DEFAULT_MAX_OUTPUT), None);
        let reply = host.call(tool, args).await;
        for part in &reply.content {
            match part.get("type").and_then(Value::as_str) {
                Some("image") => {
                    let data = part.get("data").and_then(Value::as_str).unwrap_or("");
                    let padding = data.bytes().rev().take_while(|b| *b == b'=').count();
                    let bytes = (data.len() / 4 * 3).saturating_sub(padding);
                    let mime = part.get("mimeType").and_then(Value::as_str).unwrap_or("?");
                    println!("[image {mime}, {bytes} bytes]");
                }
                _ => println!("{}", part.get("text").and_then(Value::as_str).unwrap_or("")),
            }
        }
        if reply.is_error {
            ExitCode::FAILURE
        } else {
            ExitCode::SUCCESS
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_playbooks_are_valid_skills() {
        let entries = skill_entries();
        assert!(
            entries
                .iter()
                .any(|skill| skill.uri == "skill://vexdom/SKILL.md")
        );
        rmcp::model::validate_skills(&entries).expect("valid skills-extension entries");
        assert_eq!(
            skill_file("skill://vexdom/references/batch.md")
                .map(|body| body.starts_with("# Batch")),
            Some(true)
        );
    }
}
