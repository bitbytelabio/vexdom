//! Starting the configured agent for a turn, or for a one-shot with no browser (`runner.ts`).

use std::path::{Path, PathBuf};

use vexdom_core::config::AgentConfig;
use vexdom_core::paths;

use super::abort::{Abort, RunError};
use super::runners::drive::{JsonOptions, run_json, run_stream};
use super::runners::types::{JsonContext, RunMode, RunOutcome, StreamContext};
use super::runners::{mcp_server_for, runner_for};
use crate::wire::RunEvent;

/// One turn of a panel conversation.
pub struct RunRequest<'a> {
    pub run_id: String,
    pub instruction: String,
    pub system_prompt: String,
    pub config: &'a AgentConfig,
    pub research: bool,
    /// A session this same agent established earlier in the conversation, or none to start fresh.
    pub session_id: Option<String>,
    pub signal: &'a Abort,
    pub emit: &'a dyn Fn(RunEvent),
}

pub async fn run_instruction(request: RunRequest<'_>) -> Result<RunOutcome, RunError> {
    let (runner, settings) = runner_for(request.config);
    let mcp = mcp_server_for(&request.run_id);
    let context = StreamContext {
        run_id: request.run_id,
        instruction: request.instruction,
        system_prompt: request.system_prompt,
        research: request.research,
        settings: settings.clone(),
        session_id: request.session_id,
        mcp,
    };
    run_stream(runner, &context, request.signal, request.emit).await
}

/// One-shot, no browser: summarize a file, name a conversation, read back a recording. `reads`
/// says the prompt names a file in the workspace that the agent has to open.
pub async fn run_agent_json(
    prompt: String,
    config: &AgentConfig,
    signal: &Abort,
    reads: bool,
    options: &JsonOptions<'_>,
) -> Result<String, RunError> {
    let (runner, settings) = runner_for(config);
    let context = JsonContext {
        prompt,
        settings: settings.clone(),
        reads,
    };
    run_json(runner, &context, signal, options).await
}

/// Where a one-shot's scratch files go: inside the agent's own workspace, so it is allowed to
/// read them.
pub fn task_dir(config: &AgentConfig) -> PathBuf {
    runner_for(config).0.workspace(RunMode::Task).join("tmp")
}

/// A file handed to a one-shot in its scratch folder, readable only by the user; it is deleted
/// when this is dropped, whether or not the agent managed.
pub struct TaskFile(PathBuf);

impl TaskFile {
    /// Writes `bytes` as `name` in [`task_dir`], creating the folder (0700) if need be.
    pub fn write(config: &AgentConfig, name: &str, bytes: &[u8]) -> std::io::Result<Self> {
        let path = task_dir(config).join(name);
        paths::write_private(&path, bytes)?;
        Ok(Self(path))
    }

    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TaskFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    use std::cell::RefCell;
    #[cfg(unix)]
    use std::path::Path;

    use vexdom_core::config::read_agent_config;
    use vexdom_core::paths;

    use super::*;
    #[cfg(unix)]
    use crate::agent::test_support::local;
    use crate::agent::test_support::scratch;

    #[test]
    fn a_one_shot_works_in_the_agents_own_workspace() {
        let _home = scratch("runner-task-dir");
        assert_eq!(
            task_dir(&read_agent_config()),
            paths::state_dir().join("tmp")
        );
    }

    /// The configured binary, started with the configured agent's real plan: a stand-in that
    /// answers as Claude Code would.
    #[cfg(unix)]
    fn stand_in(home: &Path, script: &str) -> AgentConfig {
        use std::os::unix::fs::PermissionsExt;

        let bin = home.join("claude-stand-in");
        std::fs::write(&bin, format!("#!/bin/sh\n{script}\n")).unwrap();
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        let mut config = read_agent_config();
        config.claude.bin = bin.to_string_lossy().into_owned();
        config
    }

    #[cfg(unix)]
    #[test]
    fn a_turn_runs_the_configured_agent_for_that_run_and_reads_its_stream() {
        let home = scratch("runner-turn");
        let config = stand_in(
            home.path(),
            r#"cat > /dev/null
printf '%s\n' '{"type":"system","subtype":"init","session_id":"s-1","mcp_servers":[{"name":"vexdom","status":"connected"}]}'
printf '{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"run %s"}}}\n' "$VEXDOM_AGENT_RUN"
printf '%s\n' '{"type":"result","subtype":"success","is_error":false,"result":"done","stop_reason":"end_turn","session_id":"s-1","usage":{"input_tokens":1,"output_tokens":2}}'"#,
        );
        local(async {
            let said = RefCell::new(String::new());
            let outcome = run_instruction(RunRequest {
                run_id: "turn-1".into(),
                instruction: "what does this page cost".into(),
                system_prompt: "You are Vexdom.".into(),
                config: &config,
                research: false,
                session_id: None,
                signal: &Abort::new(),
                emit: &|event| {
                    if let RunEvent::Text(delta) = event {
                        said.borrow_mut().push_str(&delta);
                    }
                },
            })
            .await
            .expect("the turn finishes");
            assert_eq!(outcome.stop_reason, "end_turn");
            assert_eq!(outcome.session_id.as_deref(), Some("s-1"));
            assert_eq!(said.into_inner(), "run turn-1");
        });
    }

    #[cfg(unix)]
    #[test]
    fn a_one_shot_answers_with_its_result() {
        let home = scratch("runner-json");
        let config = stand_in(
            home.path(),
            r#"printf '%s\n' '{"type":"result","is_error":false,"result":" Pricing "}'"#,
        );
        local(async {
            let options = JsonOptions {
                timed_out: "slow",
                empty: "nothing",
                accept: None,
            };
            let answer =
                run_agent_json("name this".into(), &config, &Abort::new(), false, &options).await;
            assert_eq!(answer, Ok("Pricing".into()));
        });
    }
}
