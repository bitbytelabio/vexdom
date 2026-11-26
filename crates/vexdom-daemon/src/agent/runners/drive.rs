//! Spawning an agent CLI for a turn or a one-shot, and reading what it prints
//! (`runners/drive.ts`).

use std::collections::BTreeMap;
use std::process::ExitStatus;
use std::time::{Duration, SystemTime};

use serde_json::json;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStderr, ChildStdout};
use tokio::sync::oneshot;
use vexdom_core::agents::{self, agent_label};
use vexdom_core::config::{AgentSettings, config_path};
use vexdom_core::guardrails::secrets::StreamSealer;
use vexdom_core::guardrails::spawn::{
    SpawnMode, SpawnPlan, describe_containment, seal_env, vet_plan,
};
use vexdom_core::{log, paths};

use super::command::{SpawnError, SpawnOptions, Stop, spawn_cli, stop_tree};
use super::types::{JsonContext, Plan, RunOutcome, Runner, StreamContext, StreamSink};
use super::util::install_hint;
use crate::agent::abort::{Abort, RunError, run_error};
use crate::wire::RunEvent;
use vexdom_pure::text::{js_trim, utf16_suffix};

const KILL_GRACE: Duration = Duration::from_secs(5);

/// What the parent agent marks its own processes with, and the run this daemon may have been
/// started from: none of it belongs to a CLI the daemon starts.
const STRIPPED: [&str; 3] = ["CLAUDECODE", "CLAUDE_CODE_ENTRYPOINT", "VEXDOM_AGENT_RUN"];

const STDERR_TAIL: usize = 2_000;

/// A CLI that is running: its output, and how to stop it.
pub struct Launched {
    pub stdout: ChildStdout,
    pub stderr: ChildStderr,
    /// How it ended, once it has; `None` when it could not be waited for.
    exit: oneshot::Receiver<Option<ExitStatus>>,
    /// Asks it to stop, then kills it once the grace period is over.
    stop: Abort,
}

/// Vet, then spawn. Every run reaches the CLI through here, so this is where a plan that has lost
/// its containment is stopped: before the process exists, not after it has been asked nicely to
/// behave.
pub fn launch(
    kind: &str,
    mode: SpawnMode,
    settings: &AgentSettings,
    plan: &Plan,
    signal: &Abort,
) -> Result<Launched, SpawnError> {
    stage(kind, mode, plan).map_err(SpawnError::Refused)?;
    start(kind, settings, plan, signal, KILL_GRACE)
}

/// Vets the plan and lays its files down: everything before a process of the CLI exists.
fn stage(kind: &str, mode: SpawnMode, plan: &Plan) -> Result<(), RunError> {
    let files: Vec<String> = plan
        .files
        .iter()
        .map(|file| file.path.to_string_lossy().into_owned())
        .collect();
    let spawn = SpawnPlan {
        args: &plan.args,
        cwd: &plan.cwd,
        files: &files,
    };
    let problems = vet_plan(kind, mode, &spawn, &paths::state_dir());
    if !problems.is_empty() {
        return Err(run_error(
            "AGENT_UNSAFE",
            format!(
                "Vexdom refused to start {}: {} This is a bug in Vexdom, not something you did — please report it.",
                agent_label(kind),
                problems.join(" ")
            ),
        ));
    }
    log::log(&describe_containment(kind));

    let failed = |error: std::io::Error| run_error("AGENT_FAILED", error.to_string());
    paths::ensure_private_dir(&plan.cwd).map_err(failed)?;
    for file in &plan.files {
        paths::write_private(&plan.cwd.join(&file.path), file.content.as_bytes())
            .map_err(failed)?;
    }
    // The sweep ages a workspace by its folder's time, which rewriting a file already in it leaves
    // alone, so a conversation's folder would be swept a day after its first turn, not its last.
    touch(&plan.cwd);
    Ok(())
}

fn touch(dir: &std::path::Path) {
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    if let Ok(folder) = options.open(dir) {
        let _ = folder.set_modified(SystemTime::now());
    }
}

/// The daemon's own environment, as far as it is text.
fn parent_env() -> BTreeMap<String, String> {
    std::env::vars_os()
        .filter_map(|(key, value)| Some((key.into_string().ok()?, value.into_string().ok()?)))
        .collect()
}

/// The environment any process of this agent's CLI starts with, before a plan adds its own, and
/// how many credential-shaped variables were sealed out of `parent` to make it.
fn child_env(kind: &str, parent: &BTreeMap<String, String>) -> (BTreeMap<String, String>, usize) {
    let mut env = seal_env(kind, parent);
    let sealed = parent.len() - env.len();
    for key in STRIPPED {
        env.remove(key);
    }
    (env, sealed)
}

fn start(
    kind: &str,
    settings: &AgentSettings,
    plan: &Plan,
    signal: &Abort,
    grace: Duration,
) -> Result<Launched, SpawnError> {
    let (mut env, sealed) = child_env(kind, &parent_env());
    if sealed > 0 {
        log::log(&format!(
            "sealed {sealed} credential-shaped variables out of the {kind} environment"
        ));
    }
    env.extend(plan.env.iter().cloned());

    let options = SpawnOptions {
        cwd: Some(&plan.cwd),
        env: Some(&env),
        stdin: plan.input.is_some(),
    };
    let mut child = spawn_cli(&settings.bin, &plan.args, &options)?;
    if let (Some(input), Some(mut stdin)) = (plan.input.clone(), child.stdin.take()) {
        let kind = kind.to_owned();
        tokio::task::spawn_local(async move {
            // A CLI that is gone before it has read its stdin fails the write, and that must not
            // take the daemon with it. Dropping the pipe closes it.
            if let Err(error) = stdin.write_all(input.as_bytes()).await {
                log::log(&format!("{kind} stopped reading its stdin: {error}"));
            }
        });
    }
    let stdout = child.stdout.take().expect("spawn_cli pipes stdout");
    let stderr = child.stderr.take().expect("spawn_cli pipes stderr");
    let stop = Abort::new();
    let (exited, exit) = oneshot::channel();
    tokio::task::spawn_local(supervise(
        child,
        signal.clone(),
        stop.clone(),
        grace,
        exited,
    ));
    Ok(Launched {
        stdout,
        stderr,
        exit,
        stop,
    })
}

/// Waits the CLI out, and stops it when the run is aborted or the driver asks: first politely,
/// then for good once the grace period is over.
async fn supervise(
    mut child: Child,
    signal: Abort,
    stop: Abort,
    grace: Duration,
    exited: oneshot::Sender<Option<ExitStatus>>,
) {
    let status = tokio::select! {
        status = child.wait() => status.ok(),
        () = signal.wait() => end(&mut child, grace).await,
        () = stop.wait() => end(&mut child, grace).await,
    };
    let _ = exited.send(status);
}

async fn end(child: &mut Child, grace: Duration) -> Option<ExitStatus> {
    stop_tree(child, Stop::Term).await;
    tokio::select! {
        status = child.wait() => return status.ok(),
        () = tokio::time::sleep(grace) => {}
    }
    stop_tree(child, Stop::Kill).await;
    child.wait().await.ok()
}

/// Stops the CLI when the run is dropped before it settled: nobody would read it any more.
struct StopOnDrop(Option<Abort>);

impl StopOnDrop {
    fn disarm(&mut self) {
        self.0 = None;
    }
}

impl Drop for StopOnDrop {
    fn drop(&mut self) {
        if let Some(stop) = self.0.take() {
            stop.abort();
        }
    }
}

/// Reads what is left of the CLI's output once the run no longer listens, so a CLI that goes on
/// printing is not blocked on a full pipe.
fn drain(mut stdout: impl AsyncRead + Unpin + 'static, mut stderr: ChildStderr) {
    tokio::task::spawn_local(async move {
        let mut out = tokio::io::sink();
        let mut err = tokio::io::sink();
        let _ = tokio::join!(
            tokio::io::copy(&mut stdout, &mut out),
            tokio::io::copy(&mut stderr, &mut err)
        );
    });
}

fn not_installed(kind: &str, settings: &AgentSettings) -> RunError {
    let bin = agents::agent(kind).map_or(kind, |agent| agent.bin);
    run_error(
        "AGENT_MISSING",
        format!(
            "Could not run \"{}\" — {} is not installed, or not on the daemon's PATH. Install it ({}), or set {{\"agents\":{{\"{kind}\":{{\"bin\":\"/absolute/path/to/{bin}\"}}}}}} in {}.",
            settings.bin,
            agent_label(kind),
            install_hint(kind),
            config_path().display()
        ),
    )
}

fn spawn_error(kind: &str, settings: &AgentSettings, error: SpawnError) -> RunError {
    if error.not_found() {
        return not_installed(kind, settings);
    }
    match error {
        SpawnError::Refused(error) => error,
        os @ SpawnError::Os { .. } => run_error("AGENT_FAILED", os.message()),
    }
}

fn push_tail(tail: &mut String, chunk: &[u8]) {
    tail.push_str(&String::from_utf8_lossy(chunk));
    let kept = utf16_suffix(tail, STDERR_TAIL).len();
    tail.drain(..tail.len() - kept);
}

/// `: what it printed`, or nothing when it printed nothing.
fn said(stderr: &str) -> String {
    let stderr = js_trim(stderr);
    if stderr.is_empty() {
        String::new()
    } else {
        format!(": {stderr}")
    }
}

/// What a turn tells the run and the panel, line by line.
struct Turn<'a> {
    emit: &'a dyn Fn(RunEvent),
    // What the agent says reaches the user and the transcript, so it goes through the same
    // sanitizer page text does. Deltas split a credential across two writes, which is why this
    // holds back the tail of the stream rather than sealing each one.
    outbound: StreamSealer,
    session_id: Option<String>,
    settled: Option<Result<String, RunError>>,
}

impl Turn<'_> {
    fn say(&self, delta: String) {
        if !delta.is_empty() {
            (self.emit)(RunEvent::Text(delta));
        }
    }

    fn flush(&mut self) {
        let rest = self.outbound.flush();
        self.say(rest);
    }
}

impl StreamSink for Turn<'_> {
    fn text(&mut self, delta: &str) {
        if !delta.is_empty() {
            let sealed = self.outbound.push(delta);
            self.say(sealed);
        }
    }

    fn tool(&mut self, tool_id: &str, name: &str) {
        (self.emit)(RunEvent::Tool {
            tool_id: tool_id.to_owned(),
            action: name.to_owned(),
            input: json!({}),
            source: None,
        });
    }

    fn tool_result(&mut self, tool_id: &str, ok: bool) {
        (self.emit)(RunEvent::ToolResult {
            tool_id: tool_id.to_owned(),
            ok,
            summary: if ok { "done" } else { "failed" }.to_owned(),
        });
    }

    fn session(&mut self, id: &str) {
        if !id.is_empty() {
            self.session_id = Some(id.to_owned());
        }
    }

    fn usage(&mut self, context_tokens: u64, output_tokens: u64) {
        (self.emit)(RunEvent::Usage {
            context_tokens,
            output_tokens,
        });
    }

    fn done(&mut self, stop_reason: &str) {
        if self.settled.is_none() {
            self.flush();
            self.settled = Some(Ok(stop_reason.to_owned()));
        }
    }

    fn fail(&mut self, code: &str, message: &str) {
        if self.settled.is_none() {
            self.flush();
            self.settled = Some(Err(run_error(code, message)));
        }
    }
}

/// A turn: the runner's plan, spawned and read line by line until the CLI says the turn is over.
pub async fn run_stream(
    runner: &dyn Runner,
    context: &StreamContext,
    signal: &Abort,
    emit: &dyn Fn(RunEvent),
) -> Result<RunOutcome, RunError> {
    let kind = runner.kind();
    let plan = runner.stream(context);
    stage(kind, SpawnMode::Run, &plan)?;
    let Launched {
        stdout,
        mut stderr,
        mut exit,
        stop,
    } = start(kind, &context.settings, &plan, signal, KILL_GRACE)
        .map_err(|error| spawn_error(kind, &context.settings, error))?;
    let mut guard = StopOnDrop(Some(stop.clone()));

    let mut turn = Turn {
        emit,
        outbound: StreamSealer::default(),
        session_id: None,
        settled: None,
    };
    let mut reader = runner.reader();
    let mut stdout = BufReader::new(stdout);
    let (mut line, mut chunk, mut tail) = (Vec::new(), [0u8; 4096], String::new());
    let (mut out_open, mut err_open) = (true, true);
    let status = loop {
        tokio::select! {
            read = stdout.read_until(b'\n', &mut line), if out_open => {
                if matches!(read, Ok(0) | Err(_)) {
                    out_open = false;
                }
                let text = String::from_utf8_lossy(&line);
                let text = text.trim_end_matches(['\n', '\r']);
                if !js_trim(text).is_empty()
                    && let Err(error) = reader.read(text, &mut turn)
                {
                    log::log(&format!("{kind} runner could not read a stream line: {error}"));
                }
                line.clear();
                if let Some(settled) = turn.settled.take() {
                    guard.disarm();
                    // A run the reader has failed is over, and its process would otherwise go on
                    // spending and acting.
                    if settled.is_err() {
                        stop.abort();
                    }
                    drain(stdout, stderr);
                    return settled.map(|stop_reason| RunOutcome {
                        stop_reason,
                        session_id: turn.session_id,
                    });
                }
            }
            read = stderr.read(&mut chunk), if err_open => match read {
                Ok(0) | Err(_) => err_open = false,
                Ok(read) => push_tail(&mut tail, &chunk[..read]),
            },
            status = &mut exit, if !out_open && !err_open => break status.ok().flatten(),
        }
    };
    guard.disarm();
    if signal.aborted() {
        return Err(run_error("CANCELLED", "Run cancelled."));
    }
    let message = runner.hint(&tail).unwrap_or_else(|| {
        format!(
            "{} exited with code {} before finishing{}",
            agent_label(kind),
            exit_code(status),
            said(&tail)
        )
    });
    Err(run_error("AGENT_FAILED", message))
}

/// The code Node's `close` reports: `null` when a signal ended it.
fn exit_code(status: Option<ExitStatus>) -> String {
    status
        .and_then(|status| status.code())
        .map_or_else(|| "null".to_owned(), |code| code.to_string())
}

pub struct JsonOptions<'a> {
    pub timed_out: &'a str,
    pub empty: &'a str,
    /// A complete answer ends the task there and then: the CLI is stopped rather than waited out.
    pub accept: Option<&'a dyn Fn(&str) -> bool>,
}

/// A one-shot: the runner's plan, spawned with no browser, and its answer once it is done.
pub async fn run_json(
    runner: &dyn Runner,
    context: &JsonContext,
    signal: &Abort,
    options: &JsonOptions<'_>,
) -> Result<String, RunError> {
    let kind = runner.kind();
    let plan = runner.json(context);
    let Launched {
        mut stdout,
        mut stderr,
        mut exit,
        stop,
    } = launch(kind, SpawnMode::Task, &context.settings, &plan, signal)
        .map_err(|error| spawn_error(kind, &context.settings, error))?;
    let mut guard = StopOnDrop(Some(stop.clone()));

    let mut printed = Vec::new();
    let (mut out_chunk, mut err_chunk, mut tail) = ([0u8; 4096], [0u8; 4096], String::new());
    let (mut out_open, mut err_open) = (true, true);
    loop {
        tokio::select! {
            read = stdout.read(&mut out_chunk), if out_open => match read {
                Ok(0) | Err(_) => out_open = false,
                Ok(read) => {
                    printed.extend_from_slice(&out_chunk[..read]);
                    let Some(accept) = options.accept else { continue };
                    let answer = runner.answer(&String::from_utf8_lossy(&printed));
                    let text = answer.text.as_deref().map(js_trim).unwrap_or_default();
                    if !text.is_empty() && accept(text) {
                        let text = text.to_owned();
                        guard.disarm();
                        stop.abort();
                        drain(stdout, stderr);
                        return Ok(text);
                    }
                }
            },
            read = stderr.read(&mut err_chunk), if err_open => match read {
                Ok(0) | Err(_) => err_open = false,
                Ok(read) => push_tail(&mut tail, &err_chunk[..read]),
            },
            _ = &mut exit, if !out_open && !err_open => break,
        }
    }
    guard.disarm();
    if signal.aborted() {
        return Err(signal
            .reason()
            .unwrap_or_else(|| run_error("TIMEOUT", options.timed_out)));
    }
    let answer = runner.answer(&String::from_utf8_lossy(&printed));
    if let Some(error) = answer.error {
        return Err(run_error("AGENT_FAILED", error));
    }
    match answer
        .text
        .as_deref()
        .map(js_trim)
        .filter(|text| !text.is_empty())
    {
        Some(text) => Ok(text.to_owned()),
        None => {
            let message = runner.hint(&tail).unwrap_or_else(|| {
                let stderr = js_trim(&tail);
                if stderr.is_empty() {
                    options.empty.to_owned()
                } else {
                    format!("{}: {stderr}", agent_label(kind))
                }
            });
            Err(run_error("AGENT_FAILED", message))
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::path::{Path, PathBuf};
    use std::time::Instant;

    use serde_json::Value;

    use super::*;
    use crate::agent::file_report::FileKind;
    use crate::agent::runners::claude::Claude;
    use crate::agent::runners::types::{Answer, McpServer, RunMode, StreamReader};
    use crate::agent::test_support::{local, scratch};

    /// Each line the stand-in prints is one sink call, as JSON: `["text", "hello"]`.
    struct Replay;

    impl StreamReader for Replay {
        fn read(&mut self, line: &str, sink: &mut dyn StreamSink) -> Result<(), String> {
            let call: Vec<Value> = serde_json::from_str(line).map_err(|error| error.to_string())?;
            let text = |at: usize| call.get(at).and_then(Value::as_str).unwrap_or_default();
            match text(0) {
                "text" => sink.text(text(1)),
                "tool" => sink.tool(text(1), text(2)),
                "toolResult" => sink.tool_result(text(1), call[2].as_bool() == Some(true)),
                "session" => sink.session(text(1)),
                "usage" => sink.usage(
                    call[1]["contextTokens"].as_u64().unwrap_or_default(),
                    call[1]["outputTokens"].as_u64().unwrap_or_default(),
                ),
                "done" => sink.done(text(1)),
                "fail" => sink.fail(text(1), text(2)),
                other => return Err(format!("no sink call {other}")),
            }
            Ok(())
        }
    }

    /// Claude Code's real plan, run by `sh` with a script in place of the CLI and the real
    /// arguments after it, so it is vetted as a real run is.
    fn with_script(mut real: Plan, script: &str) -> Plan {
        let mut args = vec!["-c".to_owned(), script.to_owned(), "sh".to_owned()];
        args.append(&mut real.args);
        real.args = args;
        real
    }

    struct StandIn {
        script: String,
        plan: fn(Plan) -> Plan,
    }

    fn stand_in(script: &str) -> StandIn {
        StandIn {
            script: script.to_owned(),
            plan: |real| real,
        }
    }

    impl Runner for StandIn {
        fn kind(&self) -> &'static str {
            Claude.kind()
        }
        fn version_args(&self) -> &'static [&'static str] {
            Claude.version_args()
        }
        fn efforts(&self) -> &'static [&'static str] {
            Claude.efforts()
        }
        fn workspace(&self, mode: RunMode) -> PathBuf {
            Claude.workspace(mode)
        }
        fn skill_dirs(&self) -> Vec<PathBuf> {
            Claude.skill_dirs()
        }
        fn stream(&self, context: &StreamContext) -> Plan {
            (self.plan)(with_script(Claude.stream(context), &self.script))
        }
        fn reader(&self) -> Box<dyn StreamReader> {
            Box::new(Replay)
        }
        fn keeps_first_prompt(&self) -> bool {
            Claude.keeps_first_prompt()
        }
        fn json(&self, context: &JsonContext) -> Plan {
            (self.plan)(with_script(Claude.json(context), &self.script))
        }
        fn opens(&self) -> &'static [FileKind] {
            Claude.opens()
        }
        fn answer(&self, stdout: &str) -> Answer {
            Claude.answer(stdout)
        }
        fn hint(&self, stderr_tail: &str) -> Option<String> {
            Claude.hint(stderr_tail)
        }
    }

    fn quoted(text: &str) -> String {
        format!("'{}'", text.replace('\'', r"'\''"))
    }

    /// A script printing each line; JSON values are printed as their JSON text.
    fn printing(lines: &[Value]) -> String {
        let lines: Vec<String> = lines
            .iter()
            .map(|line| match line {
                Value::String(text) => quoted(text),
                other => quoted(&other.to_string()),
            })
            .collect();
        format!("printf '%s\\n' {};", lines.join(" "))
    }

    const SH: &str = "/bin/sh";

    fn settings(bin: &str) -> AgentSettings {
        AgentSettings {
            bin: bin.to_owned(),
            model: None,
            effort: None,
            transport: None,
        }
    }

    fn stream_context(bin: &str, run_id: &str) -> StreamContext {
        StreamContext {
            run_id: run_id.to_owned(),
            instruction: "what does this page cost".into(),
            system_prompt: "You are Vexdom.".into(),
            research: false,
            settings: settings(bin),
            session_id: None,
            mcp: McpServer {
                command: "/usr/local/bin/vexdom".into(),
                args: vec!["mcp".into()],
                env: vec![("VEXDOM_AGENT_RUN".into(), "run-1".into())],
            },
        }
    }

    struct Ran {
        outcome: Result<RunOutcome, RunError>,
        events: Vec<RunEvent>,
    }

    impl Ran {
        fn said(&self) -> String {
            self.events
                .iter()
                .filter_map(|event| match event {
                    RunEvent::Text(delta) => Some(delta.as_str()),
                    _ => None,
                })
                .collect()
        }
    }

    async fn run_as(runner: &dyn Runner, run_id: &str, signal: &Abort, bin: &str) -> Ran {
        let events = RefCell::new(Vec::new());
        let context = stream_context(bin, run_id);
        let outcome = run_stream(runner, &context, signal, &|event| {
            events.borrow_mut().push(event);
        })
        .await;
        Ran {
            outcome,
            events: events.into_inner(),
        }
    }

    async fn run(runner: &dyn Runner) -> Ran {
        run_as(runner, "drive-run", &Abort::new(), SH).await
    }

    fn failure<T: std::fmt::Debug>(outcome: Result<T, RunError>) -> (String, String) {
        let error = outcome.expect_err("the run fails");
        (error.code, error.message)
    }

    fn json_context(bin: &str) -> JsonContext {
        JsonContext {
            prompt: "summarize this".into(),
            settings: settings(bin),
            reads: false,
        }
    }

    const OPTIONS: JsonOptions<'static> = JsonOptions {
        timed_out: "The summary took too long.",
        empty: "The agent said nothing.",
        accept: None,
    };

    async fn task(runner: &dyn Runner, signal: &Abort, bin: &str) -> Result<String, RunError> {
        run_json(runner, &json_context(bin), signal, &OPTIONS).await
    }

    fn alive(pid: i32) -> bool {
        nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid), None).is_ok()
    }

    /// Waits for the stand-in that wrote its pid to `file` to be gone.
    async fn gone(file: &Path) -> bool {
        let pid: i32 = std::fs::read_to_string(file)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        for _ in 0..100 {
            if !alive(pid) {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        false
    }

    const FOREVER: &str = "while :; do sleep 1; done";

    #[test]
    fn each_line_reaches_the_reader_in_order_and_the_run_ends_where_the_reader_says() {
        let _home = scratch("drive-order");
        local(async {
            let script = printing(&[
                json!(["session", "conv-1"]),
                json!(""),
                json!(["tool", "search-1", "search_web"]),
                json!(["usage", {"contextTokens": 900, "outputTokens": 12}]),
                json!(["text", "It costs $12."]),
                json!(["done", "end_turn"]),
            ]);
            let ran = run(&stand_in(&script)).await;
            assert_eq!(
                ran.outcome,
                Ok(RunOutcome {
                    stop_reason: "end_turn".into(),
                    session_id: Some("conv-1".into()),
                })
            );
            assert_eq!(
                ran.events,
                [
                    RunEvent::Tool {
                        tool_id: "search-1".into(),
                        action: "search_web".into(),
                        input: json!({}),
                        source: None,
                    },
                    RunEvent::Usage {
                        context_tokens: 900,
                        output_tokens: 12,
                    },
                    RunEvent::Text("It costs $12.".into()),
                ]
            );
        });
    }

    #[test]
    fn the_input_reaches_the_agent_on_stdin_which_is_then_closed_and_never_on_its_command_line() {
        let home = scratch("drive-stdin");
        let heard = home.path().join("heard");
        let argv = home.path().join("argv");
        local(async {
            let input = "what does \"this\" page cost?\nand the next one";
            let script = format!(
                "cat > {heard}; printf '%s\\n' \"$@\" > {argv}; printf '%s\\n' '[\"done\",\"end_turn\"]'",
                heard = quoted(&heard.to_string_lossy()),
                argv = quoted(&argv.to_string_lossy()),
            );
            let runner = StandIn {
                script,
                plan: |mut real| {
                    real.input = Some("what does \"this\" page cost?\nand the next one".into());
                    real
                },
            };
            assert!(run(&runner).await.outcome.is_ok());
            assert_eq!(std::fs::read_to_string(&heard).unwrap(), input);
            let args = std::fs::read_to_string(&argv).unwrap();
            assert!(
                !args.contains("page cost"),
                "the message is not on argv: {args}"
            );
        });
    }

    #[test]
    fn an_agent_gone_before_it_reads_its_input_fails_the_run_as_usual() {
        let _home = scratch("drive-gone");
        local(async {
            let runner = StandIn {
                script: "exit 3".into(),
                plan: |mut real| {
                    real.input = Some("x".repeat(1 << 20));
                    real
                },
            };
            assert_eq!(failure(run(&runner).await.outcome).0, "AGENT_FAILED");
        });
    }

    #[test]
    fn a_run_that_never_names_a_session_reports_none() {
        let _home = scratch("drive-no-session");
        local(async {
            let ran = run(&stand_in(&printing(&[json!(["done", "end_turn"])]))).await;
            assert_eq!(
                ran.outcome,
                Ok(RunOutcome {
                    stop_reason: "end_turn".into(),
                    session_id: None,
                })
            );
        });
    }

    #[test]
    fn a_line_the_reader_chokes_on_is_logged_and_the_run_carries_on() {
        let _home = scratch("drive-choke");
        local(async {
            let script = printing(&[
                json!("Loading model…"),
                json!(["text", "Done."]),
                json!(["done", "end_turn"]),
            ]);
            let ran = run(&stand_in(&script)).await;
            assert_eq!(ran.said(), "Done.");
            let logged = std::fs::read_to_string(paths::log_path()).unwrap();
            assert!(logged.contains("claude runner could not read a stream line"));
        });
    }

    #[test]
    fn a_credential_the_agent_repeats_is_sealed_even_split_across_two_lines() {
        let _home = scratch("drive-seal");
        local(async {
            let script = printing(&[
                json!(["text", "The key is sk-ant-api03-AbCdEfGhIj"]),
                json!(["text", "KlMnOpQrStUvWxYz012345, as asked."]),
                json!(["done", "end_turn"]),
            ]);
            let said = run(&stand_in(&script)).await.said();
            assert!(!said.contains("AbCdEfGhIj"), "{said}");
            assert!(said.contains("⟦api-key:"), "{said}");
            assert!(said.ends_with(", as asked."), "{said}");
        });
    }

    #[test]
    fn the_plans_files_are_in_place_before_the_agent_starts_readable_only_by_the_user() {
        use std::os::unix::fs::PermissionsExt;

        let _home = scratch("drive-files");
        local(async {
            let prompt = Claude
                .workspace(RunMode::Run)
                .join("agents/claude/prompts/files-run.md");
            let script = format!(
                "printf '[\"text\",\"%s\"]\\n' \"$(cat {})\"; printf '%s\\n' '[\"done\",\"end_turn\"]'",
                quoted(&prompt.to_string_lossy())
            );
            let ran = run_as(&stand_in(&script), "files-run", &Abort::new(), SH).await;
            assert_eq!(ran.said(), "You are Vexdom.");
            let mode = std::fs::metadata(&prompt).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        });
    }

    #[test]
    fn the_folder_a_run_starts_in_is_new_again_on_every_turn() {
        let _home = scratch("drive-touch");
        local(async {
            let cwd = Claude.workspace(RunMode::Run);
            let done = stand_in(&printing(&[json!(["done", "end_turn"])]));
            run_as(&done, "kept-1", &Abort::new(), SH).await;
            let nearly_a_day_ago = SystemTime::now() - Duration::from_secs(23 * 60 * 60);
            std::fs::File::open(&cwd)
                .unwrap()
                .set_modified(nearly_a_day_ago)
                .unwrap();
            run_as(&done, "kept-2", &Abort::new(), SH).await;
            let modified = std::fs::metadata(&cwd).unwrap().modified().unwrap();
            assert!(modified > SystemTime::now() - Duration::from_secs(60));
        });
    }

    #[test]
    fn a_run_that_finished_is_not_undone_by_the_exit_code_that_follows() {
        let _home = scratch("drive-finished");
        local(async {
            let script = format!("{} exit 2", printing(&[json!(["done", "end_turn"])]));
            let ran = run(&stand_in(&script)).await;
            assert_eq!(ran.outcome.unwrap().stop_reason, "end_turn");
        });
    }

    #[test]
    fn a_failure_the_reader_reports_is_the_error_the_run_ends_with() {
        let _home = scratch("drive-fail");
        local(async {
            let script = printing(&[json!(["fail", "AGENT_FAILED", "quota exceeded"])]);
            assert_eq!(
                failure(run(&stand_in(&script)).await.outcome),
                ("AGENT_FAILED".into(), "quota exceeded".into())
            );
        });
    }

    #[test]
    fn a_failure_the_reader_reports_stops_the_agent() {
        let home = scratch("drive-fail-stops");
        let pid_file = home.path().join("stopped-after-fail");
        local(async {
            let script = format!(
                "echo $$ > {}; {} {FOREVER}",
                quoted(&pid_file.to_string_lossy()),
                printing(&[json!(["fail", "AGENT_UNSAFE", "offered the shell"])])
            );
            let refused = failure(run(&stand_in(&script)).await.outcome);
            assert!(
                gone(&pid_file).await,
                "the agent is stopped once the run fails"
            );
            assert_eq!(refused, ("AGENT_UNSAFE".into(), "offered the shell".into()));
        });
    }

    #[test]
    fn an_agent_that_exits_early_fails_with_the_runners_explanation() {
        let _home = scratch("drive-hint");
        local(async {
            let script =
                "printf '%s\\n' \"error: unknown option '--strict-mcp-config'\" >&2; exit 1";
            assert_eq!(
                failure(run(&stand_in(script)).await.outcome),
                (
                    "AGENT_FAILED".into(),
                    "Your Claude Code does not understand the flags Vexdom uses to sandbox a run. Update Claude Code, then try again. (error: unknown option '--strict-mcp-config')".into()
                )
            );
        });
    }

    #[test]
    fn with_no_explanation_the_error_names_the_agent_the_exit_code_and_what_it_printed() {
        let _home = scratch("drive-exit");
        local(async {
            let script = "printf 'segmentation fault\\n' >&2; exit 3";
            assert_eq!(
                failure(run(&stand_in(script)).await.outcome),
                (
                    "AGENT_FAILED".into(),
                    "Claude Code exited with code 3 before finishing: segmentation fault".into()
                )
            );
            assert_eq!(
                failure(run(&stand_in("exit 3")).await.outcome),
                (
                    "AGENT_FAILED".into(),
                    "Claude Code exited with code 3 before finishing".into()
                )
            );
        });
    }

    #[test]
    fn a_cancelled_run_ends_as_cancelled_not_as_a_failure() {
        let _home = scratch("drive-cancel");
        local(async {
            let signal = Abort::new();
            let script = format!("{} {FOREVER}", printing(&[json!(["session", "conv-1"])]));
            let runner = stand_in(&script);
            let (ran, ()) = tokio::join!(run_as(&runner, "drive-run", &signal, SH), async {
                tokio::time::sleep(Duration::from_millis(100)).await;
                signal.abort();
            });
            assert_eq!(
                failure(ran.outcome),
                ("CANCELLED".into(), "Run cancelled.".into())
            );
        });
    }

    #[test]
    fn an_agent_that_is_not_installed_is_missing_with_how_to_install_it() {
        let home = scratch("drive-missing");
        let bin = home.path().join("no-such-claude");
        local(async {
            let bin = bin.to_string_lossy();
            let ran = run_as(&stand_in(""), "drive-run", &Abort::new(), &bin).await;
            assert_eq!(
                failure(ran.outcome),
                (
                    "AGENT_MISSING".into(),
                    format!(
                        "Could not run \"{bin}\" — Claude Code is not installed, or not on the daemon's PATH. Install it ({}), or set {{\"agents\":{{\"claude\":{{\"bin\":\"/absolute/path/to/claude\"}}}}}} in {}.",
                        agents::CLAUDE.install,
                        config_path().display()
                    )
                )
            );
            let missing = task(&stand_in(""), &Abort::new(), &bin).await;
            assert_eq!(failure(missing).0, "AGENT_MISSING");
        });
    }

    fn unsafe_plan(mut real: Plan) -> Plan {
        real.cwd = paths::state_dir().join("agents").join("unsafe-run");
        real.args.retain(|arg| arg != "--strict-mcp-config");
        real
    }

    #[test]
    fn a_plan_that_has_lost_its_containment_is_refused_before_anything_is_spawned_or_written() {
        let home = scratch("drive-unsafe");
        let marker = home.path().join("unsafe-ran");
        local(async {
            let runner = StandIn {
                script: format!("echo ran > {}", quoted(&marker.to_string_lossy())),
                plan: unsafe_plan,
            };
            assert_eq!(
                failure(run(&runner).await.outcome),
                (
                    "AGENT_UNSAFE".into(),
                    "Vexdom refused to start Claude Code: Claude Code (run) is spawned without --strict-mcp-config. This is a bug in Vexdom, not something you did — please report it.".into()
                )
            );
            assert!(!marker.exists());
            assert!(!home.path().join("agents").join("unsafe-run").exists());

            let refused = task(&runner, &Abort::new(), SH).await;
            assert_eq!(failure(refused).0, "AGENT_UNSAFE");
            assert!(!marker.exists());
        });
    }

    #[test]
    fn a_cli_loses_the_parent_agents_markers_and_other_vendors_credentials_and_keeps_its_own() {
        let parent: BTreeMap<String, String> = [
            ("CLAUDECODE", "1"),
            ("CLAUDE_CODE_ENTRYPOINT", "cli"),
            ("VEXDOM_AGENT_RUN", "the-run-that-started-this-daemon"),
            ("AWS_SECRET_ACCESS_KEY", "aws-secret"),
            ("GEMINI_API_KEY", "gemini-key"),
            ("ANTHROPIC_API_KEY", "anthropic-key"),
            ("PATH", "/usr/bin"),
        ]
        .into_iter()
        .map(|(key, value)| (key.to_owned(), value.to_owned()))
        .collect();
        let (env, sealed) = child_env("claude", &parent);
        let kept: Vec<&str> = env.keys().map(String::as_str).collect();
        assert_eq!(kept, ["ANTHROPIC_API_KEY", "PATH"]);
        assert_eq!(sealed, 2, "the AWS and Gemini credentials are sealed away");
    }

    #[test]
    fn a_run_sees_its_own_run_id_and_a_one_shot_none() {
        let _home = scratch("drive-env");
        local(async {
            let script = "printf '[\"text\",\"%s\"]\\n' \"${VEXDOM_AGENT_RUN-none}\"; printf '%s\\n' '[\"done\",\"end_turn\"]'";
            assert_eq!(run(&stand_in(script)).await.said(), "drive-run");
            let script =
                "printf '{\"type\":\"result\",\"result\":\"%s\"}\\n' \"${VEXDOM_AGENT_RUN-none}\"";
            assert_eq!(
                task(&stand_in(script), &Abort::new(), SH).await,
                Ok("none".into())
            );
        });
    }

    async fn printed(stdout: &mut BufReader<ChildStdout>, word: &str) {
        let mut line = String::new();
        while !line.contains(word) {
            line.clear();
            let read = stdout.read_line(&mut line).await.unwrap();
            assert!(
                read > 0,
                "the stand-in printed {word} before it closed stdout"
            );
        }
    }

    #[test]
    fn aborting_asks_the_agent_to_stop_and_only_kills_it_once_the_grace_period_is_over() {
        use std::os::unix::process::ExitStatusExt;

        let _home = scratch("drive-grace");
        local(async {
            let signal = Abort::new();
            // The shell runs its trap once the sleep it waits on ends, so the sleeps are short.
            let script = "trap 'echo asked to stop' TERM; echo ready; while :; do sleep 0.1; done";
            let plan = with_script(Claude.stream(&stream_context(SH, "stubborn-run")), script);
            stage("claude", SpawnMode::Run, &plan).unwrap();
            let grace = Duration::from_millis(800);
            let Ok(launched) = start("claude", &settings(SH), &plan, &signal, grace) else {
                panic!("the stand-in starts");
            };
            let Launched {
                stdout, mut exit, ..
            } = launched;
            let mut stdout = BufReader::new(stdout);
            printed(&mut stdout, "ready").await;

            signal.abort();
            let asked = Instant::now();
            printed(&mut stdout, "asked to stop").await;
            assert!(
                exit.try_recv().is_err(),
                "a CLI that handles SIGTERM is not killed before the grace period is over"
            );
            let status = (&mut exit).await.unwrap().unwrap();
            assert_eq!(status.signal(), Some(9), "{status:?}");
            assert!(asked.elapsed() >= grace);
        });
    }

    #[test]
    fn a_run_whose_signal_already_fired_is_stopped_the_moment_it_starts() {
        use std::os::unix::process::ExitStatusExt;

        let _home = scratch("drive-late");
        local(async {
            let signal = Abort::new();
            signal.abort();
            let plan = with_script(Claude.stream(&stream_context(SH, "late-run")), FOREVER);
            let Ok(Launched {
                exit,
                stdout: _stdout,
                stderr: _stderr,
                ..
            }) = launch("claude", SpawnMode::Run, &settings(SH), &plan, &signal)
            else {
                panic!("the stand-in starts");
            };
            let status = exit.await.unwrap().unwrap();
            assert_eq!(status.signal(), Some(15), "{status:?}");
        });
    }

    #[test]
    fn the_answer_to_a_one_shot_is_what_the_runner_reads_out_of_what_it_printed() {
        let _home = scratch("drive-answer");
        local(async {
            let answer = printing(&[json!({"type": "result", "result": "  A pricing page.  "})]);
            assert_eq!(
                task(&stand_in(&answer), &Abort::new(), SH).await,
                Ok("A pricing page.".into())
            );
            let error = printing(&[
                json!({"type": "result", "is_error": true, "result": "quota exceeded"}),
            ]);
            assert_eq!(
                failure(task(&stand_in(&error), &Abort::new(), SH).await),
                ("AGENT_FAILED".into(), "quota exceeded".into())
            );
        });
    }

    #[test]
    fn a_one_shot_with_no_answer_explains_what_it_can() {
        let _home = scratch("drive-no-answer");
        local(async {
            let flags = "printf '%s' \"error: unknown option '--tools'\" >&2";
            let (code, message) = failure(task(&stand_in(flags), &Abort::new(), SH).await);
            assert_eq!(code, "AGENT_FAILED");
            assert!(message.starts_with(
                "Your Claude Code does not understand the flags Vexdom uses to sandbox a run."
            ));
            let full = "printf 'disk full\\n' >&2";
            assert_eq!(
                failure(task(&stand_in(full), &Abort::new(), SH).await),
                ("AGENT_FAILED".into(), "Claude Code: disk full".into())
            );
            assert_eq!(
                failure(task(&stand_in(""), &Abort::new(), SH).await),
                ("AGENT_FAILED".into(), "The agent said nothing.".into())
            );
        });
    }

    #[test]
    fn a_one_shot_stopped_by_its_deadline_times_out_and_one_stopped_for_a_reason_says_it() {
        let _home = scratch("drive-deadline");
        local(async {
            let runner = stand_in(FOREVER);
            let signal = Abort::new();
            let (outcome, ()) = tokio::join!(task(&runner, &signal, SH), async {
                tokio::time::sleep(Duration::from_millis(100)).await;
                signal.abort();
            });
            assert_eq!(
                failure(outcome),
                ("TIMEOUT".into(), "The summary took too long.".into())
            );

            let signal = Abort::new();
            let (outcome, ()) = tokio::join!(task(&runner, &signal, SH), async {
                tokio::time::sleep(Duration::from_millis(100)).await;
                signal.abort_with(Some(run_error(
                    "CANCELLED",
                    "The file was removed before it was read.",
                )));
            });
            assert_eq!(
                failure(outcome),
                (
                    "CANCELLED".into(),
                    "The file was removed before it was read.".into()
                )
            );
        });
    }

    #[test]
    fn an_answer_the_caller_accepts_ends_the_task_and_the_agent_is_stopped() {
        let home = scratch("drive-accept");
        let pid_file = home.path().join("stopped-after-answering");
        local(async {
            let script = format!(
                "echo $$ > {}; {} {FOREVER}",
                quoted(&pid_file.to_string_lossy()),
                printing(&[json!({"type": "result", "result": "A pricing page."})])
            );
            let accept = |text: &str| text.ends_with('.');
            let options = JsonOptions {
                accept: Some(&accept),
                ..OPTIONS
            };
            let answer = run_json(
                &stand_in(&script),
                &json_context(SH),
                &Abort::new(),
                &options,
            )
            .await;
            assert_eq!(answer, Ok("A pricing page.".into()));
            assert!(
                gone(&pid_file).await,
                "the agent is stopped once it answered"
            );
        });
    }
}
