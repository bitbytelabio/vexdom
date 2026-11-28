//! `vexdom doctor [--fix] [--json] [--offline]`: checks this install end to end — the state
//! folders, the Bridge, the native host, the browsers and their extension, the agent, config,
//! permissions, ffmpeg and the network — and with `--fix` repairs what it safely can. The check
//! model, printers, summary and exit rule are agent-browser's (`cli/src/doctor/mod.rs`, f0a9835,
//! Apache-2.0, see NOTICE).

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

use serde_json::{Value, json};
use vexdom_client::{Bridge, block_on};
use vexdom_core::{lockfile, paths, project_config};

use super::{Args, agent_state, setup, text_of};

const VERSION: &str = env!("CARGO_PKG_VERSION");
/// Below this much free space in the state folder, a warning.
const LOW_DISK_BYTES: u64 = 500 * 1024 * 1024;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Status {
    Pass,
    Warn,
    Fail,
    Info,
}

impl Status {
    fn as_str(self) -> &'static str {
        match self {
            Self::Pass => "pass",
            Self::Warn => "warn",
            Self::Fail => "fail",
            Self::Info => "info",
        }
    }
}

/// What `--fix` does about a check.
#[derive(Clone, Debug, PartialEq)]
enum Repair {
    RemoveStaleLockfile,
    RestartBridge,
    RegisterNativeHost,
    RefreshUnpackedExtension,
    Chmod(PathBuf, u32),
}

struct Check {
    id: &'static str,
    category: &'static str,
    status: Status,
    message: String,
    fix: Option<String>,
    repair: Option<Repair>,
}

impl Check {
    fn new(
        id: &'static str,
        category: &'static str,
        status: Status,
        message: impl Into<String>,
    ) -> Self {
        Self {
            id,
            category,
            status,
            message: message.into(),
            fix: None,
            repair: None,
        }
    }

    fn with_fix(mut self, fix: impl Into<String>) -> Self {
        self.fix = Some(fix.into());
        self
    }

    fn with_repair(mut self, repair: Repair) -> Self {
        self.repair = Some(repair);
        self
    }
}

pub(super) fn run(args: &Args) -> ExitCode {
    let offline = args.flag("--offline");
    let mut checks = Vec::new();
    environment(&mut checks);
    native_host(&mut checks);
    block_on(bridge_and_browsers(&mut checks));
    config(&mut checks);
    security(&mut checks);
    recording(&mut checks);
    if !offline {
        block_on(network(&mut checks));
    }
    let mut fixed = Vec::new();
    if args.flag("--fix") {
        repair(&checks, &mut fixed);
    }
    let summary = summarize(&checks);
    if args.json() {
        print_json(&checks, summary, &fixed);
    } else {
        print_text(&checks, summary, &fixed, args.flag("--fix"));
    }
    if summary.2 > 0 {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

fn environment(checks: &mut Vec<Check>) {
    let category = "Environment";
    checks.push(Check::new(
        "env.version",
        category,
        Status::Pass,
        format!(
            "vexdom {VERSION} ({} {})",
            std::env::consts::OS,
            std::env::consts::ARCH
        ),
    ));
    let state = paths::state_dir();
    let probe = state.join(format!(".doctor-{}", std::process::id()));
    let writable = std::fs::create_dir_all(&state).is_ok() && std::fs::write(&probe, b"").is_ok();
    let _ = std::fs::remove_file(&probe);
    checks.push(if writable {
        Check::new(
            "env.state_dir",
            category,
            Status::Pass,
            format!("{} is writable", state.display()),
        )
    } else {
        Check::new(
            "env.state_dir",
            category,
            Status::Fail,
            format!("{} is not writable", state.display()),
        )
        .with_fix(format!("chmod u+rwx {}", state.display()))
        .with_repair(Repair::Chmod(state.clone(), 0o700))
    });
    let user = paths::user_dir();
    checks.push(Check::new(
        "env.user_dir",
        category,
        Status::Info,
        if user.is_dir() {
            format!("Files Vexdom saves for you go to {}", user.display())
        } else {
            format!(
                "{} is created when Vexdom first saves a file",
                user.display()
            )
        },
    ));
    match free_bytes(&state) {
        Some(free) if free < LOW_DISK_BYTES => checks.push(Check::new(
            "env.disk_free",
            category,
            Status::Warn,
            format!(
                "Only {} MB free where Vexdom keeps its state",
                free / (1024 * 1024)
            ),
        )),
        Some(free) => checks.push(Check::new(
            "env.disk_free",
            category,
            Status::Pass,
            format!("{} GB free", free / (1024 * 1024 * 1024)),
        )),
        None => {}
    }
}

/// Free bytes on the filesystem holding `dir` (`df -Pk`).
fn free_bytes(dir: &Path) -> Option<u64> {
    let output = std::process::Command::new("df")
        .arg("-Pk")
        .arg(dir)
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&output.stdout);
    let kilobytes: u64 = text
        .lines()
        .nth(1)?
        .split_whitespace()
        .nth(3)?
        .parse()
        .ok()?;
    Some(kilobytes * 1024)
}

fn pid_alive(pid: u32) -> bool {
    #[cfg(unix)]
    {
        i32::try_from(pid)
            .is_ok_and(|pid| nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid), None).is_ok())
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
        true
    }
}

async fn bridge_and_browsers(checks: &mut Vec<Check>) {
    let category = "Bridge";
    let Some(lock) = vexdom_daemon::ensure::probe_existing().await else {
        match lockfile::read(&paths::lockfile()) {
            Some(stale) if !pid_alive(stale.pid) => checks.push(
                Check::new(
                    "bridge.running",
                    category,
                    Status::Warn,
                    format!("Stale lockfile from pid {}, which is gone", stale.pid),
                )
                .with_fix("vexdom doctor --fix removes it")
                .with_repair(Repair::RemoveStaleLockfile),
            ),
            _ => checks.push(
                Check::new(
                    "bridge.running",
                    category,
                    Status::Info,
                    "Bridge is not running",
                )
                .with_fix("vexdom start"),
            ),
        }
        return;
    };
    let bridge = match Bridge::connect_to(lock.port, &lock.token, None).await {
        Ok(bridge) => bridge,
        Err(error) => {
            checks.push(Check::new(
                "bridge.running",
                category,
                Status::Fail,
                format!("The Bridge (pid {}) does not answer: {error}", lock.pid),
            ));
            return;
        }
    };
    let Ok(status) = bridge.status().await else {
        checks.push(Check::new(
            "bridge.running",
            category,
            Status::Fail,
            "The Bridge does not answer",
        ));
        return;
    };
    checks.push(Check::new(
        "bridge.running",
        category,
        Status::Pass,
        format!("Running on 127.0.0.1:{} (pid {})", status["port"], lock.pid),
    ));
    let daemon = status["daemonVersion"].as_str().unwrap_or("");
    checks.push(if daemon == VERSION {
        Check::new(
            "bridge.version",
            category,
            Status::Pass,
            format!("Bridge v{daemon}"),
        )
    } else {
        Check::new(
            "bridge.version",
            category,
            Status::Warn,
            format!("The Bridge runs v{daemon}, this CLI is v{VERSION}"),
        )
        .with_fix("vexdom restart")
        .with_repair(Repair::RestartBridge)
    });
    extension_checks(checks, &bridge, &status).await;
    agent_check(checks, &bridge).await;
}

async fn extension_checks(checks: &mut Vec<Check>, bridge: &Bridge, status: &Value) {
    let category = "Extension";
    checks.push(if status["connected"] == Value::Bool(true) {
        Check::new(
            "extension.connected",
            category,
            Status::Pass,
            format!(
                "{} connected (extension v{})",
                status["browser"].as_str().unwrap_or("A browser"),
                status["extensionVersion"].as_str().unwrap_or("?")
            ),
        )
    } else {
        Check::new(
            "extension.connected",
            category,
            Status::Warn,
            "No browser is connected",
        )
        .with_fix("vexdom setup")
    });
    let stamp = setup::unpacked_version();
    if let Ok(reply) = bridge.control.sessions_request().send().promise.await
        && let Ok(sessions) = reply.get().and_then(|reply| reply.get_sessions())
    {
        for session in sessions.iter() {
            let source = text_of(session.get_source());
            let version = text_of(session.get_extension_version());
            let browser = text_of(session.get_browser());
            if (source.is_empty() || source == "unpacked")
                && let Some(stamp) = stamp.as_ref().filter(|stamp| **stamp != version)
            {
                let mut check = Check::new(
                    "extension.version",
                    category,
                    Status::Warn,
                    format!(
                        "{browser} runs extension v{version}; the unpacked folder holds v{stamp}"
                    ),
                )
                .with_fix(format!(
                    "Reload the extension at {}",
                    setup::extensions_page(&browser)
                ));
                if newer_than(VERSION, stamp) {
                    check = check.with_repair(Repair::RefreshUnpackedExtension);
                }
                checks.push(check);
            }
        }
    }
    if status["manifestInSync"] == Value::Bool(false) {
        checks.push(
            Check::new(
                "extension.manifest",
                category,
                Status::Warn,
                "The extension and the Bridge are different versions; the extension's own tool list is used",
            )
            .with_fix("vexdom update"),
        );
    }
}

fn newer_than(candidate: &str, current: &str) -> bool {
    vexdom_update::newer(candidate, current)
}

async fn agent_check(checks: &mut Vec<Check>, bridge: &Bridge) {
    let state = agent_state(bridge, "", "", "").await;
    let active = state
        .get("active")
        .and_then(Value::as_str)
        .unwrap_or(vexdom_core::agents::DEFAULT_AGENT);
    let runner = state
        .get("runners")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .find(|runner| runner.get("kind").and_then(Value::as_str) == Some(active));
    let label = vexdom_core::agents::agent_label(active);
    let check = match runner {
        Some(runner) if runner.get("ready") == Some(&Value::Bool(true)) => Check::new(
            "agent.claude",
            "Agent",
            Status::Pass,
            format!(
                "{label} {}",
                runner
                    .get("version")
                    .and_then(Value::as_str)
                    .unwrap_or("ready")
            ),
        ),
        Some(runner) => {
            let problem = runner.get("problem");
            let message = problem
                .and_then(|problem| problem.get("message"))
                .and_then(Value::as_str)
                .unwrap_or("unavailable");
            let mut check = Check::new(
                "agent.claude",
                "Agent",
                Status::Warn,
                format!("{label}: {message}"),
            );
            if let Some(fix) = problem
                .and_then(|problem| problem.get("fix"))
                .and_then(Value::as_str)
            {
                check = check.with_fix(fix);
            }
            check
        }
        None => Check::new(
            "agent.claude",
            "Agent",
            Status::Warn,
            format!("{label} is not installed"),
        ),
    };
    checks.push(check);
}

fn native_host(checks: &mut Vec<Check>) {
    checks.push(if vexdom_daemon::native_host::registration_current() {
        Check::new(
            "native_host.registered",
            "Bridge",
            Status::Pass,
            "Browsers can start the Bridge",
        )
    } else {
        Check::new(
            "native_host.registered",
            "Bridge",
            Status::Warn,
            "The browsers' native-host registration is missing or points at another folder",
        )
        .with_fix("vexdom setup")
        .with_repair(Repair::RegisterNativeHost)
    });
}

fn config(checks: &mut Vec<Check>) {
    let category = "Config";
    let user = paths::state_dir().join("config.json");
    match std::fs::read(&user) {
        Ok(bytes) => checks.push(match serde_json::from_slice::<Value>(&bytes) {
            Ok(_) => Check::new(
                "config.user",
                category,
                Status::Pass,
                format!("{} reads", user.display()),
            ),
            Err(error) => Check::new(
                "config.user",
                category,
                Status::Fail,
                format!("{} is not valid JSON: {error}", user.display()),
            ),
        }),
        Err(_) => checks.push(Check::new(
            "config.user",
            category,
            Status::Info,
            "No config.json; defaults apply",
        )),
    }
    let cwd = std::env::current_dir().unwrap_or_default();
    if cwd.join(project_config::PROJECT_FILE).is_file() {
        let (_, warnings) = project_config::load(&cwd);
        checks.push(if warnings.is_empty() {
            Check::new(
                "config.project",
                category,
                Status::Pass,
                "./vexdom.json reads",
            )
        } else {
            Check::new("config.project", category, Status::Warn, warnings.join(" "))
        });
    }
}

#[cfg(unix)]
fn mode_of(path: &Path) -> Option<u32> {
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::metadata(path)
        .ok()
        .map(|meta| meta.permissions().mode() & 0o777)
}

#[cfg(not(unix))]
fn mode_of(_: &Path) -> Option<u32> {
    None
}

fn security(checks: &mut Vec<Check>) {
    let category = "Security";
    let state = paths::state_dir();
    let mut loose = Vec::new();
    let wanted = [
        (state.clone(), 0o700),
        (state.join("auth.json"), 0o600),
        (state.join("config.json"), 0o600),
    ];
    for (path, mode) in wanted {
        if let Some(found) = mode_of(&path)
            && found & !mode != 0
        {
            loose.push((path, mode, found));
        }
    }
    if loose.is_empty() {
        checks.push(Check::new(
            "security.state_perms",
            category,
            Status::Pass,
            "Only you can read Vexdom's state",
        ));
    }
    for (path, mode, found) in loose {
        checks.push(
            Check::new(
                "security.state_perms",
                category,
                Status::Warn,
                format!(
                    "{} is mode {found:o}; it should be {mode:o}",
                    path.display()
                ),
            )
            .with_fix(format!("chmod {mode:o} {}", path.display()))
            .with_repair(Repair::Chmod(path, mode)),
        );
    }
    let key = state.join(".vault-key");
    match mode_of(&key) {
        None => checks.push(Check::new(
            "security.vault_key",
            category,
            Status::Info,
            "No vault key yet; it is made when you save the first login",
        )),
        Some(found) if found & !0o600 != 0 => checks.push(
            Check::new(
                "security.vault_key",
                category,
                Status::Warn,
                format!("{} is mode {found:o}; it should be 600", key.display()),
            )
            .with_fix(format!("chmod 600 {}", key.display()))
            .with_repair(Repair::Chmod(key, 0o600)),
        ),
        Some(_) => checks.push(Check::new(
            "security.vault_key",
            category,
            Status::Pass,
            "The vault key is private",
        )),
    }
}

fn recording(checks: &mut Vec<Check>) {
    let version = std::process::Command::new("ffmpeg")
        .arg("-version")
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| {
            String::from_utf8_lossy(&output.stdout)
                .lines()
                .next()
                .unwrap_or("")
                .to_owned()
        });
    checks.push(match version {
        Some(line) => Check::new("recording.ffmpeg", "Recording", Status::Pass, line),
        None => Check::new(
            "recording.ffmpeg",
            "Recording",
            Status::Warn,
            "ffmpeg not found (only needed for record)",
        )
        .with_fix(if cfg!(target_os = "macos") {
            "brew install ffmpeg"
        } else {
            "install ffmpeg with your package manager"
        }),
    });
}

async fn network(checks: &mut Vec<Check>) {
    let category = "Network";
    let url = format!("https://github.com/{}/releases/latest", vexdom_update::REPO);
    vexdom_core::install_tls_provider();
    let client = reqwest::Client::builder()
        .user_agent(concat!("vexdom/", env!("CARGO_PKG_VERSION"), " doctor"))
        .timeout(Duration::from_secs(3))
        .connect_timeout(Duration::from_secs(3))
        .build();
    let reached = match client {
        Ok(client) => client
            .head(&url)
            .send()
            .await
            .map(|response| response.status()),
        Err(error) => {
            checks.push(Check::new(
                "net.github",
                category,
                Status::Fail,
                format!("Could not build an HTTP client: {error}"),
            ));
            return;
        }
    };
    checks.push(match reached {
        Ok(status) if status.is_success() || status.is_redirection() => Check::new(
            "net.github",
            category,
            Status::Pass,
            "GitHub releases are reachable",
        ),
        Ok(status) => Check::new(
            "net.github",
            category,
            Status::Warn,
            format!("GitHub answered {status}"),
        ),
        Err(error) => Check::new(
            "net.github",
            category,
            Status::Fail,
            format!("GitHub is unreachable: {error}"),
        )
        .with_fix("Check the network, or run vexdom doctor --offline"),
    });
    if let Ok(release) = vexdom_update::latest().await
        && vexdom_update::newer(&release.version, VERSION)
    {
        checks.push(
            Check::new(
                "update.available",
                category,
                Status::Info,
                format!("vexdom {} is available", release.version),
            )
            .with_fix("vexdom update"),
        );
    }
}

fn repair(checks: &[Check], fixed: &mut Vec<String>) {
    for check in checks {
        let Some(repair) = &check.repair else {
            continue;
        };
        let done = match repair {
            Repair::RemoveStaleLockfile => std::fs::remove_file(paths::lockfile())
                .map(|()| "Removed the stale lockfile".to_owned())
                .map_err(|error| error.to_string()),
            Repair::RestartBridge => {
                if block_on(super::restart()) == ExitCode::SUCCESS {
                    Ok("Restarted the Bridge".to_owned())
                } else {
                    Err("the Bridge did not restart".to_owned())
                }
            }
            Repair::RegisterNativeHost => std::env::current_exe()
                .map_err(|error| error.to_string())
                .map(|exe| {
                    let installed = vexdom_daemon::native_host::install_native_host(&exe);
                    format!(
                        "Registered the native host for {}",
                        installed.browsers.join(", ")
                    )
                }),
            Repair::RefreshUnpackedExtension => {
                super::install::install(&paths::extension_dir(None), false)
                    .map(|installed| {
                        format!(
                            "Refreshed the unpacked extension to v{}; reload it in the browser",
                            installed.version
                        )
                    })
                    .map_err(|error| format!("{error:?}"))
            }
            Repair::Chmod(path, mode) => chmod(path, *mode)
                .map(|()| format!("chmod {mode:o} {}", path.display()))
                .map_err(|error| error.to_string()),
        };
        match done {
            Ok(line) => fixed.push(line),
            Err(error) => fixed.push(format!("{} could not be fixed: {error}", check.id)),
        }
    }
}

#[cfg(unix)]
fn chmod(path: &Path, mode: u32) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
}

#[cfg(not(unix))]
fn chmod(_: &Path, _: u32) -> std::io::Result<()> {
    Ok(())
}

/// `(pass, warn, fail)`; info does not count.
fn summarize(checks: &[Check]) -> (usize, usize, usize) {
    checks
        .iter()
        .fold((0, 0, 0), |(pass, warn, fail), check| match check.status {
            Status::Pass => (pass + 1, warn, fail),
            Status::Warn => (pass, warn + 1, fail),
            Status::Fail => (pass, warn, fail + 1),
            Status::Info => (pass, warn, fail),
        })
}

fn print_text(checks: &[Check], summary: (usize, usize, usize), fixed: &[String], fix_ran: bool) {
    println!("vexdom doctor");
    let mut category = "";
    for check in checks {
        if check.category != category {
            category = check.category;
            println!("\n{category}");
        }
        println!("  {}  {}", check.status.as_str(), check.message);
        if let Some(fix) = &check.fix {
            println!("        fix: {fix}");
        }
    }
    if !fixed.is_empty() {
        println!("\nFixed");
        for line in fixed {
            println!("  done  {line}");
        }
    }
    println!(
        "\nSummary: {} pass, {} warn, {} fail",
        summary.0, summary.1, summary.2
    );
    if !fix_ran && checks.iter().any(|check| check.repair.is_some()) {
        println!("\ntip: Run with --fix to attempt repairs.");
    }
}

fn print_json(checks: &[Check], summary: (usize, usize, usize), fixed: &[String]) {
    let checks: Vec<Value> = checks
        .iter()
        .map(|check| {
            let mut out = json!({
                "id": check.id,
                "category": check.category,
                "status": check.status.as_str(),
                "message": check.message,
            });
            if let Some(fix) = &check.fix {
                out["fix"] = json!(fix);
            }
            out
        })
        .collect();
    println!(
        "{}",
        json!({
            "success": summary.2 == 0,
            "summary": {"pass": summary.0, "warn": summary.1, "fail": summary.2},
            "checks": checks,
            "fixed": fixed,
        })
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn info_does_not_count_and_a_failure_fails() {
        let checks = vec![
            Check::new("a", "C", Status::Pass, ""),
            Check::new("b", "C", Status::Warn, ""),
            Check::new("c", "C", Status::Fail, ""),
            Check::new("d", "C", Status::Info, ""),
        ];
        assert_eq!(summarize(&checks), (1, 1, 1));
    }
}
