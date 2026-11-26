//! Claude Code (`runners/claude.ts`): the `claude -p` plans, and the stream-json reader.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value, json};
use vexdom_core::downloads::uuid_v4;
use vexdom_core::{log, paths};

use super::types::{
    Answer, JsonContext, McpServer, Plan, RunMode, Runner, StreamContext, StreamReader, StreamSink,
    WorkspaceFile,
};
use super::util::{browser_tools_did_not_start, conversation_dir, effort_of, sweep_run_dirs};
use crate::agent::file_report::FileKind;
use vexdom_pure::text::{js_text, js_trim};

pub const MCP_SERVER_NAME: &str = "vexdom";

const BUILTIN_DENIED: [&str; 26] = [
    "Bash",
    "Edit",
    "Write",
    "NotebookEdit",
    "Glob",
    "Grep",
    "Read",
    "Task",
    "Monitor",
    "Workflow",
    "Skill",
    "ToolSearch",
    "SendMessage",
    "TaskOutput",
    "TaskStop",
    "TodoWrite",
    "ReportFindings",
    "CronCreate",
    "CronDelete",
    "CronList",
    "ScheduleWakeup",
    "RemoteTrigger",
    "PushNotification",
    "DesignSync",
    "EnterWorktree",
    "ExitWorktree",
];

const WEB_TOOLS: [&str; 2] = ["WebSearch", "WebFetch"];

/// A server still `pending` may yet connect; anything else but `connected` never will this run.
const STARTING: [&str; 2] = ["connected", "pending"];

pub struct Claude;

/// `JSON.stringify({mcpServers: {vexdom: server}})`.
fn mcp_config(server: &McpServer) -> String {
    let env: Map<String, Value> = server
        .env
        .iter()
        .map(|(name, value)| (name.clone(), Value::String(value.clone())))
        .collect();
    let mut servers = Map::new();
    servers.insert(
        MCP_SERVER_NAME.into(),
        json!({"command": server.command, "args": server.args, "env": env}),
    );
    json!({ "mcpServers": servers }).to_string()
}

fn strings(names: &[&str]) -> impl Iterator<Item = String> {
    names.iter().map(|name| (*name).to_owned())
}

impl Runner for Claude {
    fn kind(&self) -> &'static str {
        "claude"
    }

    fn version_args(&self) -> &'static [&'static str] {
        &["--version"]
    }

    fn efforts(&self) -> &'static [&'static str] {
        &["low", "medium", "high", "xhigh", "max"]
    }

    fn workspace(&self, _mode: RunMode) -> PathBuf {
        paths::state_dir()
    }

    fn skill_dirs(&self) -> Vec<PathBuf> {
        vec![paths::home_dir().join(".claude").join("skills")]
    }

    // The message goes in on stdin and the system prompt from a file of its own: anyone can read
    // argv.
    fn stream(&self, context: &StreamContext) -> Plan {
        let settings = &context.settings;
        let research = context.research;
        let effort = effort_of(settings, self.efforts());
        let cwd = self.workspace(RunMode::Run);
        let prompts = paths::state_dir()
            .join("agents")
            .join("claude")
            .join("prompts");
        sweep_run_dirs(&prompts, None);
        let mut prompt = conversation_dir(&prompts, &context.run_id).into_os_string();
        prompt.push(".md");
        let prompt = PathBuf::from(prompt);
        let relative = prompt
            .strip_prefix(&cwd)
            .map_or_else(|_| prompt.clone(), Path::to_path_buf);

        let mut args: Vec<String> = strings(&[
            "-p",
            "--output-format",
            "stream-json",
            "--include-partial-messages",
            "--verbose",
            "--mcp-config",
        ])
        .collect();
        args.push(mcp_config(&context.mcp));
        args.extend(strings(&["--strict-mcp-config", "--tools"]));
        if research {
            args.extend(strings(&WEB_TOOLS));
        } else {
            args.push(String::new());
        }
        args.push("--allowedTools".into());
        args.push(format!("mcp__{MCP_SERVER_NAME}"));
        if research {
            args.extend(strings(&WEB_TOOLS));
        }
        args.push("--disallowedTools".into());
        args.extend(strings(&BUILTIN_DENIED));
        if !research {
            args.extend(strings(&WEB_TOOLS));
        }
        args.push("--append-system-prompt-file".into());
        args.push(prompt.to_string_lossy().into_owned());
        match &context.session_id {
            Some(session) => args.extend(["--resume".into(), session.clone()]),
            None => args.extend(["--session-id".into(), uuid_v4()]),
        }
        if let Some(model) = settings.model.as_deref().filter(|model| !model.is_empty()) {
            args.extend(["--model".into(), model.to_owned()]);
        }
        if let Some(effort) = effort {
            args.extend(["--effort".into(), effort]);
        }
        Plan {
            args,
            cwd,
            env: vec![("VEXDOM_AGENT_RUN".into(), context.run_id.clone())],
            files: vec![WorkspaceFile {
                path: relative,
                content: context.system_prompt.clone(),
            }],
            input: Some(context.instruction.clone()),
        }
    }

    fn reader(&self) -> Box<dyn StreamReader> {
        Box::new(ClaudeReader::default())
    }

    // Claude Code records the system prompt on a session's first request and resends that record
    // on every resume, whatever --append-system-prompt says (on by default in 2.1.283).
    fn keeps_first_prompt(&self) -> bool {
        true
    }

    fn json(&self, context: &JsonContext) -> Plan {
        let settings = &context.settings;
        let allowed: &[&str] = if context.reads { &["Read"] } else { &[] };
        let effort = effort_of(settings, self.efforts());
        let mut args: Vec<String> = vec![
            "-p".into(),
            context.prompt.clone(),
            "--output-format".into(),
            "json".into(),
            "--no-session-persistence".into(),
            "--mcp-config".into(),
            r#"{"mcpServers":{}}"#.into(),
            "--strict-mcp-config".into(),
            "--tools".into(),
        ];
        if allowed.is_empty() {
            args.push(String::new());
        } else {
            args.extend(strings(allowed));
            args.push("--allowedTools".into());
            args.extend(strings(allowed));
        }
        args.push("--disallowedTools".into());
        let denied = BUILTIN_DENIED
            .iter()
            .chain(WEB_TOOLS.iter())
            .filter(|tool| !allowed.contains(tool));
        args.extend(denied.map(|tool| (*tool).to_owned()));
        if let Some(model) = settings.model.as_deref().filter(|model| !model.is_empty()) {
            args.extend(["--model".into(), model.to_owned()]);
        }
        if let Some(effort) = effort {
            args.extend(["--effort".into(), effort]);
        }
        Plan {
            args,
            cwd: self.workspace(RunMode::Task),
            env: Vec::new(),
            files: Vec::new(),
            input: None,
        }
    }

    fn opens(&self) -> &'static [FileKind] {
        &[FileKind::Text, FileKind::Pdf, FileKind::Image]
    }

    fn answer(&self, stdout: &str) -> Answer {
        let Ok(parsed) = serde_json::from_str::<Value>(stdout) else {
            return Answer::default();
        };
        if truthy(parsed.get("is_error")) {
            let said = [parsed.get("result"), parsed.get("subtype")]
                .into_iter()
                .flatten()
                .find(|value| truthy(Some(value)))
                .map_or_else(|| "Claude Code reported an error".to_owned(), js_text);
            return Answer {
                text: None,
                error: Some(said),
            };
        }
        Answer {
            text: parsed
                .get("result")
                .and_then(Value::as_str)
                .map(str::to_owned),
            error: None,
        }
    }

    fn hint(&self, stderr_tail: &str) -> Option<String> {
        let lower = stderr_tail.to_lowercase();
        if !lower.contains("unknown option") && !lower.contains("unrecognized option") {
            return None;
        }
        Some(format!(
            "Your Claude Code does not understand the flags Vexdom uses to sandbox a run. Update Claude Code, then try again. ({})",
            js_trim(stderr_tail)
        ))
    }
}

/// JavaScript truthiness of an optional JSON value.
fn truthy(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) => false,
        Some(Value::Bool(flag)) => *flag,
        Some(Value::String(text)) => !text.is_empty(),
        Some(Value::Number(number)) => number.as_f64().is_some_and(|n| n != 0.0 && !n.is_nan()),
        Some(Value::Array(_) | Value::Object(_)) => true,
    }
}

fn text_of<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())
}

fn tokens(usage: &Value, key: &str) -> u64 {
    usage.get(key).and_then(Value::as_u64).unwrap_or(0)
}

fn prompt_of(usage: &Value) -> u64 {
    tokens(usage, "input_tokens")
        + tokens(usage, "cache_read_input_tokens")
        + tokens(usage, "cache_creation_input_tokens")
}

/// A message's usage is only final in its `message_delta`: the assistant lines repeat the opening
/// snapshot once per content block, so counting those under-reports and double-counts.
#[derive(Default)]
struct ClaudeReader {
    prompt: u64,
    generated: u64,
    counted: bool,
    /// Its own web tools are the only rows this reader opens, so only their results close one.
    opened: HashSet<String>,
}

impl ClaudeReader {
    fn report(&mut self, usage: &Value, sink: &mut dyn StreamSink) {
        self.counted = true;
        let output = tokens(usage, "output_tokens");
        self.generated += output;
        sink.usage(self.prompt + output, self.generated);
    }

    fn system(message: &Value, sink: &mut dyn StreamSink) {
        if message.get("subtype").and_then(Value::as_str) != Some("init") {
            return;
        }
        let session = message.get("session_id").and_then(Value::as_str);
        if let Some(id) = session.filter(|id| !id.is_empty()) {
            sink.session(id);
        }
        let builtins: Vec<&str> = message
            .get("tools")
            .and_then(Value::as_array)
            .map(|tools| {
                tools
                    .iter()
                    .filter_map(Value::as_str)
                    .filter(|tool| !tool.starts_with("mcp__"))
                    .collect()
            })
            .unwrap_or_default();
        let session = session.unwrap_or("undefined");
        log::log(&format!(
            "claude session {session} up, built-ins: {}",
            if builtins.is_empty() {
                "none".to_owned()
            } else {
                builtins.join(", ")
            }
        ));
        let browser = message
            .get("mcp_servers")
            .and_then(Value::as_array)
            .and_then(|servers| {
                servers.iter().find(|server| {
                    server.get("name").and_then(Value::as_str) == Some(MCP_SERVER_NAME)
                })
            });
        // Claude Code carries on without a server that failed, and the model answers without the page.
        if let Some(status) = browser.and_then(|server| text_of(server, "status"))
            && !STARTING.contains(&status)
        {
            log::log(&format!(
                "claude session {session}: the {MCP_SERVER_NAME} server is {status}"
            ));
            sink.fail("AGENT_FAILED", &browser_tools_did_not_start("Claude Code"));
        }
    }

    fn user(&mut self, message: &Value, sink: &mut dyn StreamSink) {
        if truthy(message.get("parent_tool_use_id")) {
            return;
        }
        let Some(content) = message
            .get("message")
            .and_then(|inner| inner.get("content"))
            .and_then(Value::as_array)
        else {
            return;
        };
        for block in content {
            if block.get("type").and_then(Value::as_str) != Some("tool_result") {
                continue;
            }
            let Some(id) = text_of(block, "tool_use_id") else {
                continue;
            };
            if self.opened.remove(id) {
                sink.tool_result(id, block.get("is_error") != Some(&Value::Bool(true)));
            }
        }
    }

    fn stream_event(&mut self, message: &Value, sink: &mut dyn StreamSink) {
        if truthy(message.get("parent_tool_use_id")) {
            return;
        }
        let Some(event) = message.get("event") else {
            return;
        };
        let kind = event.get("type").and_then(Value::as_str);
        if kind == Some("content_block_delta")
            && let Some(delta) = event.get("delta")
            && delta.get("type").and_then(Value::as_str) == Some("text_delta")
            && let Some(text) = text_of(delta, "text")
        {
            sink.text(text);
            return;
        }
        if kind == Some("content_block_start")
            && let Some(block) = event.get("content_block")
            && block.get("type").and_then(Value::as_str) == Some("tool_use")
        {
            let name = block.get("name").and_then(Value::as_str).unwrap_or("tool");
            if !WEB_TOOLS.contains(&name) {
                return;
            }
            let id = block
                .get("id")
                .and_then(Value::as_str)
                .map_or_else(uuid_v4, str::to_owned);
            self.opened.insert(id.clone());
            sink.tool(&id, name);
        }
        if kind == Some("message_start") {
            let usage = event
                .get("message")
                .and_then(|inner| inner.get("usage"))
                .cloned()
                .unwrap_or(Value::Null);
            self.prompt = prompt_of(&usage);
        }
        if kind == Some("message_delta")
            && let Some(usage) = event.get("usage").filter(|usage| truthy(Some(usage)))
        {
            self.report(usage, sink);
        }
    }

    fn result(&mut self, message: &Value, sink: &mut dyn StreamSink) {
        if truthy(message.get("is_error")) {
            let said = text_of(message, "result")
                .or_else(|| text_of(message, "subtype"))
                .unwrap_or("Claude Code reported an error");
            sink.fail("AGENT_FAILED", said);
            return;
        }
        if !self.counted
            && let Some(usage) = message.get("usage").filter(|usage| truthy(Some(usage)))
        {
            self.prompt = prompt_of(usage);
            self.report(usage, sink);
        }
        sink.done(text_of(message, "stop_reason").unwrap_or("end_turn"));
    }
}

impl StreamReader for ClaudeReader {
    fn read(&mut self, line: &str, sink: &mut dyn StreamSink) -> Result<(), String> {
        let Ok(message) = serde_json::from_str::<Value>(line) else {
            return Ok(());
        };
        match message.get("type").and_then(Value::as_str) {
            Some("system") => Self::system(&message, sink),
            Some("user") => self.user(&message, sink),
            Some("stream_event") => self.stream_event(&message, sink),
            Some("result") => self.result(&message, sink),
            _ => {}
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::test_support::scratch;
    use vexdom_core::config::AgentSettings;

    /// Every sink call, as `[signal, ...args]`.
    #[derive(Default)]
    struct Calls(Vec<Value>);

    impl StreamSink for Calls {
        fn text(&mut self, delta: &str) {
            self.0.push(json!(["text", delta]));
        }
        fn tool(&mut self, tool_id: &str, name: &str) {
            self.0.push(json!(["tool", tool_id, name]));
        }
        fn tool_result(&mut self, tool_id: &str, ok: bool) {
            self.0.push(json!(["toolResult", tool_id, ok]));
        }
        fn session(&mut self, id: &str) {
            self.0.push(json!(["session", id]));
        }
        fn usage(&mut self, context_tokens: u64, output_tokens: u64) {
            self.0.push(
                json!(["usage", {"contextTokens": context_tokens, "outputTokens": output_tokens}]),
            );
        }
        fn done(&mut self, stop_reason: &str) {
            self.0.push(json!(["done", stop_reason]));
        }
        fn fail(&mut self, code: &str, message: &str) {
            self.0.push(json!(["fail", code, message]));
        }
    }

    fn transcript(name: &str) -> Vec<String> {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/claude")
            .join(name);
        std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("fixture {}: {error}", path.display()))
            .lines()
            .filter(|line| !line.trim().is_empty() && !line.starts_with('#'))
            .map(str::to_owned)
            .collect()
    }

    fn read_through(lines: &[String], only: Option<&str>) -> Vec<Value> {
        let mut calls = Calls::default();
        let mut reader = Claude.reader();
        for line in lines {
            reader
                .read(line, &mut calls)
                .expect("the reader skips what it cannot read");
        }
        calls
            .0
            .into_iter()
            .filter(|call| only.is_none_or(|signal| call[0] == signal))
            .collect()
    }

    fn read(name: &str, only: Option<&str>) -> Vec<Value> {
        let _home = scratch("claude-read");
        read_through(&transcript(name), only)
    }

    fn read_lines(lines: &[Value]) -> Vec<Value> {
        let _home = scratch("claude-read-lines");
        let lines: Vec<String> = lines.iter().map(Value::to_string).collect();
        read_through(&lines, None)
    }

    #[test]
    fn a_recorded_turn_establishes_the_session_says_the_answer_and_finishes() {
        assert_eq!(
            read("2.1.278-fresh.jsonl", None),
            [
                json!(["session", "2e281281-10ef-47b8-8a23-d6137f6770f6"]),
                json!(["text", "ok"]),
                json!(["usage", {"contextTokens": 7389, "outputTokens": 41}]),
                json!(["done", "end_turn"]),
            ]
        );
    }

    #[test]
    fn a_recorded_follow_up_reports_the_session_it_resumed_and_counts_the_cache() {
        assert_eq!(
            read("2.1.278-resumed.jsonl", Some("session")),
            [json!(["session", "2e281281-10ef-47b8-8a23-d6137f6770f6"])]
        );
        assert_eq!(
            read("2.1.278-resumed.jsonl", Some("usage")),
            [json!(["usage", {"contextTokens": 7467, "outputTokens": 31}])]
        );
    }

    #[test]
    fn a_model_the_account_cannot_use_fails_with_what_claude_code_said() {
        let calls: Vec<Value> = read("2.1.278-unknown-model.jsonl", None)
            .into_iter()
            .filter(|call| call[0] != "session")
            .collect();
        assert_eq!(
            calls,
            [json!([
                "fail",
                "AGENT_FAILED",
                "There's an issue with the selected model (claude-nonexistent-9). It may not exist or you may not have access to it. Run --model to pick a different model."
            ])]
        );
    }

    #[test]
    fn browser_tools_are_not_announced_and_output_adds_up_across_messages() {
        assert_eq!(
            read("tool-loop.hand-written.jsonl", Some("tool")),
            Vec::<Value>::new()
        );
        assert_eq!(
            read("tool-loop.hand-written.jsonl", Some("usage")),
            [
                json!(["usage", {"contextTokens": 8260, "outputTokens": 60}]),
                json!(["usage", {"contextTokens": 9112, "outputTokens": 72}]),
            ]
        );
    }

    #[test]
    fn its_own_web_tools_are_announced_and_closed_as_their_results_come_back() {
        let calls: Vec<Value> = read("web-search.hand-written.jsonl", None)
            .into_iter()
            .filter(|call| call[0] == "tool" || call[0] == "toolResult")
            .collect();
        assert_eq!(
            calls,
            [
                json!(["tool", "toolu_ws", "WebSearch"]),
                json!(["tool", "toolu_wf", "WebFetch"]),
                json!(["toolResult", "toolu_ws", true]),
                json!(["toolResult", "toolu_wf", false]),
            ]
        );
    }

    #[test]
    fn browser_tools_that_did_not_start_stop_the_run() {
        assert_eq!(
            read("2.1.283-mcp-failed.jsonl", None)[..2],
            [
                json!(["session", "9160ef97-27f3-45bb-aaee-3024305cbc0c"]),
                json!([
                    "fail",
                    "AGENT_FAILED",
                    "Claude Code could not start Vexdom's browser tools, so this run could not reach the page. Run \"vexdom restart\", then send the message again."
                ]),
            ]
        );
        let init = |status: &str| json!({"type": "system", "subtype": "init", "session_id": "s-1", "mcp_servers": [{"name": "vexdom", "status": status}]});
        assert_eq!(read_lines(&[init("pending")]), [json!(["session", "s-1"])]);
        assert_eq!(
            read_lines(&[init("connected")]),
            [json!(["session", "s-1"])]
        );
    }

    #[test]
    fn nothing_a_subagent_says_calls_or_spends_reaches_the_run() {
        assert_eq!(
            read("subagent.hand-written.jsonl", None),
            [
                json!(["session", "c41a9e07-2f65-4b18-93d0-6e7a2b1c8f45"]),
                json!(["usage", {"contextTokens": 5540, "outputTokens": 40}]),
                json!(["text", "Done."]),
                json!(["usage", {"contextTokens": 6303, "outputTokens": 43}]),
                json!(["done", "end_turn"]),
            ]
        );
    }

    #[test]
    fn results_without_messages_or_deltas_still_end_the_run() {
        assert_eq!(
            read_lines(&[
                json!({"type": "result", "is_error": true, "subtype": "error_max_turns"})
            ]),
            [json!(["fail", "AGENT_FAILED", "error_max_turns"])]
        );
        assert_eq!(
            read_lines(&[json!({"type": "result", "subtype": "success"})]),
            [json!(["done", "end_turn"])]
        );
        let result = json!({"type": "result", "subtype": "success", "usage": {"input_tokens": 10, "cache_read_input_tokens": 90, "output_tokens": 5}});
        assert_eq!(
            read_lines(&[result])[0],
            json!(["usage", {"contextTokens": 105, "outputTokens": 5}])
        );
    }

    #[test]
    fn lines_it_does_not_understand_are_passed_over() {
        let _home = scratch("claude-skip");
        let lines = [
            "not json",
            r#"{"type":"rate_limit_event"}"#,
            r#"{"type":"system","subtype":"status"}"#,
            r#"{"type":"user"}"#,
        ]
        .map(str::to_owned);
        assert_eq!(read_through(&lines, None), Vec::<Value>::new());
    }

    fn mcp() -> McpServer {
        McpServer {
            command: "/usr/local/bin/vexdom".into(),
            args: vec!["mcp".into()],
            env: vec![("VEXDOM_AGENT_RUN".into(), "run-1".into())],
        }
    }

    fn stream(settings: AgentSettings, research: bool, session_id: Option<&str>) -> Plan {
        Claude.stream(&StreamContext {
            run_id: "run-1".into(),
            instruction: "what does this page cost".into(),
            system_prompt: "You are Vexdom.".into(),
            research,
            settings,
            session_id: session_id.map(str::to_owned),
            mcp: mcp(),
        })
    }

    fn settings(model: Option<&str>, effort: Option<&str>) -> AgentSettings {
        AgentSettings {
            bin: "claude".into(),
            model: model.map(str::to_owned),
            effort: effort.map(str::to_owned),
            transport: None,
        }
    }

    fn value_of<'a>(args: &'a [String], flag: &str) -> Option<&'a str> {
        let at = args.iter().position(|arg| arg == flag)?;
        args.get(at + 1).map(String::as_str)
    }

    fn values_of<'a>(args: &'a [String], flag: &str) -> Vec<&'a str> {
        let Some(at) = args.iter().position(|arg| arg == flag) else {
            return Vec::new();
        };
        args[at + 1..]
            .iter()
            .take_while(|arg| !arg.starts_with("--"))
            .map(String::as_str)
            .collect()
    }

    #[test]
    fn a_fresh_run_starts_a_session_of_its_own_and_reaches_only_the_browser() {
        let home = scratch("claude-plan");
        let plan = stream(settings(None, None), false, None);
        let state = home.path().to_string_lossy().into_owned();
        let shown: Vec<String> = plan
            .args
            .iter()
            .map(|arg| arg.replace(&state, "<state>"))
            .collect();
        let session = value_of(&shown, "--session-id")
            .unwrap_or_default()
            .to_owned();
        let mut expected: Vec<String> = [
            "-p",
            "--output-format",
            "stream-json",
            "--include-partial-messages",
            "--verbose",
            "--mcp-config",
            r#"{"mcpServers":{"vexdom":{"command":"/usr/local/bin/vexdom","args":["mcp"],"env":{"VEXDOM_AGENT_RUN":"run-1"}}}}"#,
            "--strict-mcp-config",
            "--tools",
            "",
            "--allowedTools",
            "mcp__vexdom",
            "--disallowedTools",
        ]
        .map(str::to_owned)
        .to_vec();
        expected.extend(strings(&BUILTIN_DENIED));
        expected.extend(strings(&WEB_TOOLS));
        expected.extend(
            [
                "--append-system-prompt-file",
                "<state>/agents/claude/prompts/run-1.md",
                "--session-id",
            ]
            .map(str::to_owned),
        );
        expected.push(session.clone());
        assert_eq!(shown, expected);
        assert_eq!(
            session.len(),
            36,
            "a fresh session gets a new UUID: {session}"
        );
        assert_eq!(plan.cwd, home.path());
        assert_eq!(
            plan.env,
            [("VEXDOM_AGENT_RUN".to_owned(), "run-1".to_owned())]
        );
        assert_eq!(
            plan.files,
            [WorkspaceFile {
                path: PathBuf::from("agents/claude/prompts/run-1.md"),
                content: "You are Vexdom.".into()
            }]
        );
        assert_eq!(plan.input.as_deref(), Some("what does this page cost"));
    }

    #[test]
    fn follow_ups_research_models_and_efforts_shape_the_arguments() {
        let _home = scratch("claude-plan-options");
        let resumed = stream(settings(None, None), false, Some("sess-1")).args;
        assert_eq!(value_of(&resumed, "--resume"), Some("sess-1"));
        assert!(!resumed.contains(&"--session-id".to_owned()));

        let research = stream(settings(None, None), true, None).args;
        assert_eq!(values_of(&research, "--tools"), ["WebSearch", "WebFetch"]);
        assert_eq!(
            values_of(&research, "--allowedTools"),
            ["mcp__vexdom", "WebSearch", "WebFetch"]
        );
        assert!(
            !values_of(&research, "--disallowedTools")
                .iter()
                .any(|tool| tool.starts_with("Web"))
        );

        let chosen = stream(settings(Some("claude-opus-5"), Some("max")), false, None).args;
        assert_eq!(
            [value_of(&chosen, "--model"), value_of(&chosen, "--effort")],
            [Some("claude-opus-5"), Some("max")]
        );
        let odd = stream(settings(None, Some("minimal")), false, None).args;
        assert!(!odd.contains(&"--effort".to_owned()));
    }

    #[test]
    fn a_one_shot_reaches_no_browser_and_reads_only_when_it_must() {
        let home = scratch("claude-json");
        let plan = Claude.json(&JsonContext {
            prompt: "summarize this".into(),
            settings: settings(None, None),
            reads: false,
        });
        let mut expected: Vec<String> = [
            "-p",
            "summarize this",
            "--output-format",
            "json",
            "--no-session-persistence",
            "--mcp-config",
            r#"{"mcpServers":{}}"#,
            "--strict-mcp-config",
            "--tools",
            "",
            "--disallowedTools",
        ]
        .map(str::to_owned)
        .to_vec();
        expected.extend(strings(&BUILTIN_DENIED));
        expected.extend(strings(&WEB_TOOLS));
        assert_eq!(plan.args, expected);
        assert_eq!(plan.cwd, home.path());

        let reading = Claude
            .json(&JsonContext {
                prompt: "summarize this".into(),
                settings: settings(Some("claude-haiku-4-5"), Some("low")),
                reads: true,
            })
            .args;
        assert_eq!(values_of(&reading, "--tools"), ["Read"]);
        assert_eq!(values_of(&reading, "--allowedTools"), ["Read"]);
        assert!(!values_of(&reading, "--disallowedTools").contains(&"Read"));
        assert_eq!(
            [
                value_of(&reading, "--model"),
                value_of(&reading, "--effort")
            ],
            [Some("claude-haiku-4-5"), Some("low")]
        );
    }

    #[test]
    fn the_answer_to_a_one_shot_is_its_result() {
        let answer = |stdout: &str| Claude.answer(stdout);
        assert_eq!(
            answer(r#"{"type":"result","is_error":false,"result":"A pricing page."}"#).text,
            Some("A pricing page.".into())
        );
        assert_eq!(
            answer(r#"{"is_error":true,"result":"Credit balance is too low"}"#).error,
            Some("Credit balance is too low".into())
        );
        assert_eq!(
            answer(r#"{"is_error":true,"subtype":"error_during_execution"}"#).error,
            Some("error_during_execution".into())
        );
        assert_eq!(answer("Segmentation fault"), Answer::default());
    }

    #[test]
    fn a_claude_code_too_old_for_the_sandbox_flags_is_told_to_update() {
        assert_eq!(
            Claude
                .hint("error: unknown option '--strict-mcp-config'\n")
                .as_deref(),
            Some(
                "Your Claude Code does not understand the flags Vexdom uses to sandbox a run. Update Claude Code, then try again. (error: unknown option '--strict-mcp-config')"
            )
        );
        assert_eq!(Claude.hint("Error: connect ECONNREFUSED"), None);
    }

    #[test]
    fn facts_about_claude_code() {
        let _home = scratch("claude-facts");
        assert!(Claude.keeps_first_prompt());
        assert_eq!(
            Claude.skill_dirs(),
            [paths::home_dir().join(".claude").join("skills")]
        );
        assert_eq!(
            [
                Claude.workspace(RunMode::Run),
                Claude.workspace(RunMode::Task)
            ],
            [paths::state_dir(), paths::state_dir()]
        );
    }
}
