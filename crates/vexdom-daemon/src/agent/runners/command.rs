//! How the daemon starts an agent CLI, and stops one (`runners/command.ts`): a spawn into a
//! process group of its own and a signal to that group. macOS and Linux only (ADR-026).

use std::collections::BTreeMap;
use std::io::ErrorKind;
use std::path::Path;
use std::process::Stdio;

use tokio::process::{Child, Command as Process};

use crate::agent::abort::{RunError, run_error};

/// How an agent CLI is started.
#[derive(Default)]
pub struct SpawnOptions<'a> {
    pub cwd: Option<&'a Path>,
    /// The whole environment it starts with; the daemon's own when `None`.
    pub env: Option<&'a BTreeMap<String, String>>,
    /// Pipe stdin, to be written to, rather than closing it.
    pub stdin: bool,
}

/// Why an agent CLI did not start.
#[derive(Debug)]
pub enum SpawnError {
    /// Refused before any process existed.
    Refused(RunError),
    /// The system could not start `file`.
    Os { file: String, error: std::io::Error },
}

impl SpawnError {
    /// The refusal's code, or the system error as Node names it (`ENOENT`).
    pub fn code(&self) -> String {
        match self {
            Self::Refused(error) => error.code.clone(),
            Self::Os { error, .. } => errno_name(error),
        }
    }

    /// The refusal's message, or the system error as Node says it (`spawn claude ENOENT`).
    pub fn message(&self) -> String {
        match self {
            Self::Refused(error) => error.message.clone(),
            Self::Os { file, error } => format!("spawn {file} {}", errno_name(error)),
        }
    }

    pub fn not_found(&self) -> bool {
        matches!(self, Self::Os { error, .. } if error.kind() == ErrorKind::NotFound)
    }
}

fn errno_name(error: &std::io::Error) -> String {
    match error.kind() {
        ErrorKind::NotFound => return "ENOENT".into(),
        ErrorKind::PermissionDenied => return "EACCES".into(),
        _ => {}
    }
    #[cfg(unix)]
    if let Some(raw) = error.raw_os_error() {
        return format!("{:?}", nix::errno::Errno::from_raw(raw));
    }
    error.to_string()
}

/// Starts an agent CLI: stdout and stderr piped, stdin closed unless it is to be written to, in a
/// process group of its own so it can be stopped with everything it started.
pub fn spawn_cli(
    bin: &str,
    args: &[String],
    options: &SpawnOptions<'_>,
) -> Result<Child, SpawnError> {
    // What no system could start is refused the same way everywhere, before the spawn.
    if bin.contains('\0') || args.iter().any(|arg| arg.contains('\0')) {
        return Err(SpawnError::Refused(run_error(
            "AGENT_UNUSABLE",
            format!("Could not start \"{bin}\": nul byte found in provided data"),
        )));
    }

    let mut process = Process::new(bin);
    process
        .args(args)
        .stdin(if options.stdin {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(cwd) = options.cwd {
        process.current_dir(cwd);
    }
    if let Some(env) = options.env {
        process.env_clear().envs(env);
    }
    #[cfg(unix)]
    process.process_group(0);
    process.spawn().map_err(|error| {
        // What the system never got to try (a NUL in the environment) is a CLI that cannot be
        // started.
        if error.kind() == ErrorKind::InvalidInput {
            SpawnError::Refused(run_error(
                "AGENT_UNUSABLE",
                format!("Could not start \"{bin}\": {error}"),
            ))
        } else {
            SpawnError::Os {
                file: bin.to_owned(),
                error,
            }
        }
    })
}

/// The signal a stop asks with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stop {
    Term,
    Kill,
}

/// Stops a CLI and everything it started: its process group. A process already reaped is left
/// alone, so its id cannot reach whatever reused it.
pub async fn stop_tree(child: &mut Child, signal: Stop) {
    #[cfg(unix)]
    {
        use nix::sys::signal::{Signal, kill, killpg};
        use nix::unistd::Pid;

        let Some(pid) = child.id().and_then(|pid| i32::try_from(pid).ok()) else {
            return;
        };
        let pid = Pid::from_raw(pid);
        let signal = match signal {
            Stop::Term => Signal::SIGTERM,
            Stop::Kill => Signal::SIGKILL,
        };
        if killpg(pid, signal).is_err() {
            let _ = kill(pid, signal);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::test_support::local;

    #[cfg(unix)]
    fn sh(script: &str, rest: &[&str]) -> Vec<String> {
        ["-c", script, "sh"]
            .iter()
            .chain(rest)
            .map(|arg| (*arg).to_owned())
            .collect()
    }

    #[cfg(unix)]
    async fn stdout_of(child: &mut Child) -> String {
        use tokio::io::AsyncReadExt;

        let mut said = String::new();
        let mut stdout = child.stdout.take().unwrap();
        stdout.read_to_string(&mut said).await.unwrap();
        said
    }

    #[cfg(unix)]
    #[test]
    fn stdin_is_closed_unless_asked_for_and_then_what_is_written_reaches_the_cli() {
        use tokio::io::AsyncWriteExt;

        local(async {
            let echo = sh("printf 'got:%s' \"$(cat)\"", &[]);
            let closed = spawn_cli("/bin/sh", &echo, &SpawnOptions::default()).unwrap();
            assert!(closed.stdin.is_none());

            let options = SpawnOptions {
                stdin: true,
                ..SpawnOptions::default()
            };
            let mut held = spawn_cli("/bin/sh", &echo, &options).unwrap();
            let mut stdin = held.stdin.take().unwrap();
            stdin.write_all(b"hello").await.unwrap();
            drop(stdin);
            assert_eq!(stdout_of(&mut held).await, "got:hello");
            held.wait().await.unwrap();
        });
    }

    #[test]
    fn a_spawn_no_system_could_start_fails_as_a_cli_that_cannot_be_started() {
        local(async {
            let program = std::env::current_exe().unwrap();
            let program = program.to_string_lossy();
            for (bin, args) in [
                (&*program, vec!["bad\0argument".to_owned()]),
                ("claude\0", vec![]),
            ] {
                let Err(SpawnError::Refused(refused)) =
                    spawn_cli(bin, &args, &SpawnOptions::default())
                else {
                    panic!("a NUL in {bin:?} {args:?} is refused before anything starts");
                };
                assert_eq!(refused.code, "AGENT_UNUSABLE");
                assert_eq!(
                    refused.message,
                    format!("Could not start \"{bin}\": nul byte found in provided data")
                );
            }
        });
    }

    #[test]
    fn a_program_that_is_not_there_is_enoent_as_node_says_it() {
        local(async {
            let missing = std::env::temp_dir().join("vexdom-no-such-cli");
            let Err(error) = spawn_cli(&missing.to_string_lossy(), &[], &SpawnOptions::default())
            else {
                panic!("a missing program does not start");
            };
            assert!(error.not_found());
            assert_eq!(error.code(), "ENOENT");
            assert_eq!(
                error.message(),
                format!("spawn {} ENOENT", missing.display())
            );
        });
    }

    #[cfg(unix)]
    #[test]
    fn stopping_one_ends_it_and_what_it_started() {
        local(async {
            let args = sh("sleep 30 & sleep 30; wait", &[]);
            let mut child = spawn_cli("/bin/sh", &args, &SpawnOptions::default()).unwrap();
            stop_tree(&mut child, Stop::Term).await;
            let status = child.wait().await.unwrap();
            assert!(!status.success());
            // The pipes close only once the backgrounded sleep is gone too.
            assert_eq!(stdout_of(&mut child).await, "");
        });
    }
}
