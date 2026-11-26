//! What a runner plans and what its stream reader reports (`runners/types.ts`).

use std::path::PathBuf;

use vexdom_core::config::AgentSettings;

use crate::agent::file_report::FileKind;

/// `run` drives the browser and resumes conversations; `task` is a one-shot with no browser.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunMode {
    Run,
    Task,
}

/// The stdio MCP server a run's agent is handed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpServer {
    pub command: String,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
}

/// A file the runner needs in its working directory before the CLI starts; `path` is relative to
/// the plan's `cwd`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceFile {
    pub path: PathBuf,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    pub args: Vec<String>,
    pub cwd: PathBuf,
    pub env: Vec<(String, String)>,
    pub files: Vec<WorkspaceFile>,
    /// Written to the CLI's stdin, which is then closed: what is too long, or too page-made, for
    /// argv.
    pub input: Option<String>,
}

pub struct StreamContext {
    pub run_id: String,
    pub instruction: String,
    pub system_prompt: String,
    pub research: bool,
    pub settings: AgentSettings,
    /// A session this same agent established earlier in the conversation, or none to start fresh.
    pub session_id: Option<String>,
    pub mcp: McpServer,
}

pub struct JsonContext {
    pub prompt: String,
    pub settings: AgentSettings,
    /// The prompt names a file in the workspace that the agent has to open.
    pub reads: bool,
}

/// What a stream reader tells the run, line by line.
pub trait StreamSink {
    fn text(&mut self, delta: &str);
    /// A tool the daemon cannot see itself (web search and the like); MCP calls report themselves.
    fn tool(&mut self, tool_id: &str, name: &str);
    /// That tool finished.
    fn tool_result(&mut self, tool_id: &str, ok: bool);
    fn session(&mut self, id: &str);
    /// Token counts, normalized: the context after the last call, and output over the run.
    fn usage(&mut self, context_tokens: u64, output_tokens: u64);
    fn done(&mut self, stop_reason: &str);
    fn fail(&mut self, code: &str, message: &str);
}

/// Fresh per run: readers carry state across lines. `Err` is a line it could not read.
pub trait StreamReader {
    fn read(&mut self, line: &str, sink: &mut dyn StreamSink) -> Result<(), String>;
}

/// What a one-shot printed, read as an answer.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Answer {
    pub text: Option<String>,
    pub error: Option<String>,
}

/// One agent CLI: how it is started, and how its output is read.
pub trait Runner {
    fn kind(&self) -> &'static str;
    /// Arguments that make the binary print its version, used to prove it is installed.
    fn version_args(&self) -> &'static [&'static str];
    /// Reasoning-effort names this CLI accepts; anything else is dropped.
    fn efforts(&self) -> &'static [&'static str];
    fn workspace(&self, mode: RunMode) -> PathBuf;
    /// Directories where this CLI keeps the user's own skills.
    fn skill_dirs(&self) -> Vec<PathBuf>;
    fn stream(&self, context: &StreamContext) -> Plan;
    fn reader(&self) -> Box<dyn StreamReader>;
    /// A resumed session goes on sending the system prompt it began with, so what changed since
    /// has to reach it in the turn's own message.
    fn keeps_first_prompt(&self) -> bool;
    fn json(&self, context: &JsonContext) -> Plan;
    /// What a one-shot of this CLI opens when handed a file.
    fn opens(&self) -> &'static [FileKind];
    fn answer(&self, stdout: &str) -> Answer;
    /// Turns a stderr tail into something more useful than "exited with code 1".
    fn hint(&self, stderr_tail: &str) -> Option<String>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunOutcome {
    pub stop_reason: String,
    /// The session the agent established, to resume on the next turn.
    pub session_id: Option<String>,
}
