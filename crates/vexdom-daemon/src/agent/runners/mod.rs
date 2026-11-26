//! The agent CLIs a run can drive (`runners/index.ts`), and whether each is ready. Only Claude
//! Code.

pub mod claude;
pub mod command;
pub mod drive;
pub mod models;
pub mod types;
pub mod util;

use std::sync::Mutex;
use std::time::{Duration, Instant};

use futures::future::join_all;
use serde_json::{Map, Value, json};
use tokio::io::AsyncReadExt;
use vexdom_core::agents::{AGENT_KINDS, agent_label};
use vexdom_core::config::{AgentConfig, AgentSettings, config_path};

use command::{SpawnOptions, Stop, spawn_cli, stop_tree};
use models::models_for;
use types::{McpServer, Runner};
use util::install_hint;

use vexdom_pure::text::{js_trim, utf16_prefix};

/// The stdio MCP server every agent talks to: this same binary's `mcp` subcommand, speaking for
/// one run through `VEXDOM_AGENT_RUN`.
pub fn mcp_server_for(run_id: &str) -> McpServer {
    let command = std::env::current_exe()
        .map(|path| path.to_string_lossy().into_owned())
        .unwrap_or_else(|_| "vexdom".into());
    McpServer {
        command,
        args: vec!["mcp".into()],
        env: vec![("VEXDOM_AGENT_RUN".into(), run_id.to_owned())],
    }
}

/// `RUNNERS[kind]`: every agent kind is Claude Code.
pub fn runner_of(kind: &str) -> &'static dyn Runner {
    let _ = kind;
    &claude::Claude
}

/// `runnerFor`: the configured agent's runner, with its own settings.
pub fn runner_for(config: &AgentConfig) -> (&'static dyn Runner, &AgentSettings) {
    let (kind, settings) = config.active();
    (runner_of(kind), settings)
}

/// `config.agents[kind]`: Claude Code is the only agent there is.
fn settings_of<'a>(config: &'a AgentConfig, kind: &str) -> &'a AgentSettings {
    let _ = kind;
    &config.claude
}

const PROBE_TIMEOUT: Duration = Duration::from_secs(8);
const PROBE_TTL: Duration = Duration::from_secs(30);

/// The last round of probes, for the settings it was made with.
pub struct Probed {
    at: Instant,
    signature: String,
    runners: Vec<Value>,
}

static PROBED: Mutex<Option<Probed>> = Mutex::new(None);

/// `agentState(config, {refresh})`: `AgentState` (`src/lib/agents/catalog.ts`), each agent
/// probed with its version flag; a round is reused for half a minute unless `refresh`.
pub async fn agent_state(config: &AgentConfig, refresh: bool) -> Value {
    state_in(&PROBED, config, refresh).await
}

async fn state_in(cache: &Mutex<Option<Probed>>, config: &AgentConfig, refresh: bool) -> Value {
    let signature = AGENT_KINDS
        .iter()
        .map(|kind| format!("{kind}:{:?}", settings_of(config, kind)))
        .collect::<Vec<_>>()
        .join("\n");
    if !refresh {
        let held = cache
            .lock()
            .expect("the probe cache is only read and replaced");
        if let Some(probed) = held.as_ref()
            && probed.signature == signature
            && probed.at.elapsed() < PROBE_TTL
        {
            return json!({"active": config.agent, "runners": probed.runners});
        }
    }
    let runners = join_all(
        AGENT_KINDS
            .iter()
            .map(|kind| probe(runner_of(kind), settings_of(config, kind))),
    )
    .await;
    let state = json!({"active": config.agent, "runners": runners});
    *cache
        .lock()
        .expect("the probe cache is only read and replaced") = Some(Probed {
        at: Instant::now(),
        signature,
        runners,
    });
    state
}

/// `RunnerStatus`: ready with its version and models, or the problem and how to fix it.
async fn probe(runner: &dyn Runner, settings: &AgentSettings) -> Value {
    let kind = runner.kind();
    let found = version(&settings.bin, runner.version_args()).await;
    let problem = |code: &str, message: String, fix: String| json!({"code": code, "message": message, "fix": fix});
    let problem = match found.code.as_deref() {
        Some("ENOENT") => Some(problem(
            "AGENT_MISSING",
            format!(
                "{} is not installed, or \"{}\" is not on the daemon's PATH.",
                agent_label(kind),
                settings.bin
            ),
            install_hint(kind).to_owned(),
        )),
        Some("AGENT_UNUSABLE") => Some(problem(
            "AGENT_UNUSABLE",
            found
                .detail
                .clone()
                .unwrap_or_else(|| format!("\"{}\" could not be started.", settings.bin)),
            format!(
                "{{\"agents\":{{\"{kind}\":{{\"bin\":\"<the program itself>\"}}}}}} in {}",
                config_path().display()
            ),
        )),
        _ if !found.ok => Some(problem(
            "AGENT_UNUSABLE",
            format!(
                "\"{} {}\" failed{}.",
                settings.bin,
                runner.version_args().join(" "),
                found
                    .detail
                    .as_ref()
                    .map_or_else(String::new, |detail| format!(": {detail}"))
            ),
            install_hint(kind).to_owned(),
        )),
        _ => None,
    };

    let mut status = Map::new();
    status.insert("kind".into(), kind.into());
    status.insert("bin".into(), settings.bin.clone().into());
    if let Some(model) = &settings.model {
        status.insert("model".into(), model.clone().into());
    }
    status.insert("ready".into(), problem.is_none().into());
    match problem {
        Some(problem) => {
            status.insert("problem".into(), problem);
        }
        None => {
            if let Some(version) = found.detail {
                status.insert("version".into(), version.into());
            }
            status.insert("models".into(), models_for(kind));
        }
    }
    Value::Object(status)
}

/// What a version probe found: `code` names why it could not run at all.
struct Found {
    ok: bool,
    code: Option<String>,
    detail: Option<String>,
}

async fn version(bin: &str, args: &[&str]) -> Found {
    let args: Vec<String> = args.iter().map(|arg| (*arg).to_owned()).collect();
    let mut child = match spawn_cli(bin, &args, &SpawnOptions::default()) {
        Ok(child) => child,
        // One agent that cannot be started must not take the others' answers down with it.
        Err(error) => {
            return Found {
                ok: false,
                code: Some(error.code()),
                detail: Some(error.message()),
            };
        }
    };
    let (Some(mut stdout), Some(mut stderr)) = (child.stdout.take(), child.stderr.take()) else {
        unreachable!("spawn_cli pipes stdout and stderr");
    };
    let mut output = Vec::new();
    let finished = tokio::time::timeout(PROBE_TIMEOUT, async {
        let (mut out_chunk, mut err_chunk) = ([0u8; 1024], [0u8; 1024]);
        let (mut out_open, mut err_open) = (true, true);
        while out_open || err_open {
            tokio::select! {
                read = stdout.read(&mut out_chunk), if out_open => match read {
                    Ok(0) | Err(_) => out_open = false,
                    Ok(read) => output.extend_from_slice(&out_chunk[..read]),
                },
                read = stderr.read(&mut err_chunk), if err_open => match read {
                    Ok(0) | Err(_) => err_open = false,
                    Ok(read) => output.extend_from_slice(&err_chunk[..read]),
                },
            }
        }
        child.wait().await
    })
    .await;
    match finished {
        Ok(status) => Found {
            ok: status.is_ok_and(|status| status.success()),
            code: None,
            detail: first_line(&String::from_utf8_lossy(&output)),
        },
        Err(_) => {
            stop_tree(&mut child, Stop::Kill).await;
            let _ = child.wait().await;
            Found {
                ok: false,
                code: None,
                detail: Some("timed out".into()),
            }
        }
    }
}

fn first_line(output: &str) -> Option<String> {
    output
        .split('\n')
        .map(js_trim)
        .find(|line| !line.is_empty())
        .map(|line| utf16_prefix(line, 80).to_owned())
}

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    use std::path::{Path, PathBuf};

    #[cfg(unix)]
    use vexdom_core::agents::CLAUDE;
    use vexdom_core::config::read_agent_config;

    use super::*;
    use crate::agent::test_support::{local, scratch};

    #[test]
    fn a_run_is_handed_this_binary_serving_mcp_for_it() {
        let server = mcp_server_for("run-7");
        assert_eq!(
            server.command,
            std::env::current_exe().unwrap().to_string_lossy()
        );
        assert_eq!(server.args, ["mcp"]);
        assert_eq!(
            server.env,
            [("VEXDOM_AGENT_RUN".to_owned(), "run-7".to_owned())]
        );
    }

    #[test]
    fn every_agent_in_the_catalog_has_a_runner_that_answers_to_its_name() {
        for kind in AGENT_KINDS {
            assert_eq!(runner_of(kind).kind(), kind);
        }
    }

    #[test]
    fn the_configured_agent_is_the_one_that_runs_with_its_own_settings() {
        let _home = scratch("runner-for");
        let mut config = read_agent_config();
        config.claude.bin = "/opt/claude".into();
        config.claude.model = Some("opus".into());
        let (runner, settings) = runner_for(&config);
        assert_eq!(runner.kind(), "claude");
        assert_eq!(settings.bin, "/opt/claude");
        assert_eq!(settings.model.as_deref(), Some("opus"));
    }

    /// A stand-in CLI that notes each version probe, then answers `--version` the given way.
    #[cfg(unix)]
    fn stub(dir: &Path, probes: &Path, name: &str, prints: &str, exit: i32) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;

        let path = dir.join(name);
        let script = format!(
            "#!/bin/sh\nif [ \"$1\" = --version ]; then echo {name} >> '{}'; fi\necho '{prints}' >&2\nexit {exit}\n",
            probes.display()
        );
        std::fs::write(&path, script).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    #[cfg(unix)]
    struct Stubs {
        bin: PathBuf,
        probes: PathBuf,
    }

    #[cfg(unix)]
    impl Stubs {
        fn new(home: &Path) -> Self {
            let bin = home.join("stub-bin");
            std::fs::create_dir_all(&bin).unwrap();
            let probes = home.join("probes.log");
            stub(&bin, &probes, "claude", "claude 1.0.0", 0);
            stub(&bin, &probes, "claude-next", "claude-next 1.0.0", 0);
            stub(&bin, &probes, "claude-expired", "licence expired", 3);
            Self { bin, probes }
        }

        fn config(&self, bin: &str) -> AgentConfig {
            let mut config = read_agent_config();
            config.claude.bin = self.bin.join(bin).to_string_lossy().into_owned();
            config
        }

        /// The stubs probed so far, a round at a time.
        fn rounds(&self) -> Vec<Vec<String>> {
            let probed = std::fs::read_to_string(&self.probes).unwrap_or_default();
            let lines: Vec<String> = probed.lines().map(str::to_owned).collect();
            lines
                .chunks(AGENT_KINDS.len())
                .map(|round| {
                    let mut round = round.to_vec();
                    round.sort();
                    round
                })
                .collect()
        }
    }

    fn claude_of(state: &Value) -> &Value {
        state["runners"]
            .as_array()
            .and_then(|runners| runners.iter().find(|runner| runner["kind"] == "claude"))
            .expect("the state lists Claude Code")
    }

    #[cfg(unix)]
    #[test]
    fn an_installed_agent_is_ready_with_its_version_and_the_curated_models() {
        let home = scratch("probe-ready");
        let stubs = Stubs::new(home.path());
        local(async {
            let cache = Mutex::new(None);
            let config = stubs.config("claude");
            let state = state_in(&cache, &config, true).await;
            assert_eq!(state["active"], "claude");
            assert_eq!(
                claude_of(&state),
                &json!({
                    "kind": "claude",
                    "bin": config.claude.bin,
                    "model": "sonnet",
                    "ready": true,
                    "version": "claude 1.0.0",
                    "models": {"ids": CLAUDE.models, "from": "catalog"},
                })
            );
        });
    }

    #[cfg(unix)]
    #[test]
    fn an_agent_that_is_not_installed_is_missing_with_the_command_that_installs_it() {
        let home = scratch("probe-missing");
        let stubs = Stubs::new(home.path());
        local(async {
            let config = stubs.config("no-such-claude");
            let state = state_in(&Mutex::new(None), &config, true).await;
            let runner = claude_of(&state);
            assert_eq!(runner["ready"], false);
            assert_eq!(runner["problem"]["code"], "AGENT_MISSING");
            assert_eq!(runner["problem"]["fix"], CLAUDE.install);
            assert_eq!(
                runner["problem"]["message"],
                format!(
                    "Claude Code is not installed, or \"{}\" is not on the daemon's PATH.",
                    config.claude.bin
                )
            );
        });
    }

    #[cfg(unix)]
    #[test]
    fn an_agent_whose_version_check_fails_is_unusable_and_says_what_it_printed() {
        let home = scratch("probe-expired");
        let stubs = Stubs::new(home.path());
        local(async {
            let config = stubs.config("claude-expired");
            let state = state_in(&Mutex::new(None), &config, true).await;
            assert_eq!(
                claude_of(&state)["problem"],
                json!({
                    "code": "AGENT_UNUSABLE",
                    "message": format!("\"{} --version\" failed: licence expired.", config.claude.bin),
                    "fix": CLAUDE.install,
                })
            );
        });
    }

    #[test]
    fn an_agent_that_cannot_be_started_at_all_says_where_to_point_vexdom() {
        let _home = scratch("probe-unstartable");
        local(async {
            let mut config = read_agent_config();
            config.claude.bin = "claude\0".into();
            let state = state_in(&Mutex::new(None), &config, true).await;
            let runner = claude_of(&state);
            assert_eq!(runner["ready"], false);
            assert_eq!(runner["problem"]["code"], "AGENT_UNUSABLE");
            let fix = runner["problem"]["fix"].as_str().unwrap();
            assert!(fix.contains("\"claude\":{\"bin\""), "{fix}");
        });
    }

    #[cfg(unix)]
    #[test]
    fn probes_are_reused_for_half_a_minute_unless_refreshed_or_the_binary_changes() {
        let home = scratch("probe-cache");
        let stubs = Stubs::new(home.path());
        local(async {
            let cache = Mutex::new(None);
            let config = stubs.config("claude");
            state_in(&cache, &config, true).await;
            let reused = state_in(&cache, &config, false).await;
            assert_eq!(stubs.rounds(), [["claude"]]);
            assert_eq!(claude_of(&reused)["version"], "claude 1.0.0");

            state_in(&cache, &config, true).await;
            assert_eq!(stubs.rounds().len(), 2, "a refresh probes again");

            let next = stubs.config("claude-next");
            let probed = state_in(&cache, &next, false).await;
            assert_eq!(
                stubs.rounds(),
                [["claude"], ["claude"], ["claude-next"]],
                "a different binary is probed afresh"
            );
            assert_eq!(claude_of(&probed)["version"], "claude-next 1.0.0");
        });
    }

    #[test]
    fn the_first_line_is_what_a_version_says() {
        assert_eq!(
            first_line("\n  \n  claude 2.1.0 (Claude Code)  \nmore"),
            Some("claude 2.1.0 (Claude Code)".into())
        );
        assert_eq!(first_line(" \n"), None);
        assert_eq!(first_line(&"v".repeat(100)).unwrap().len(), 80);
    }
}
