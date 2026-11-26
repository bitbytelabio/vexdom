//! Finding, starting and stopping the Bridge from a client (`src/daemon/ensure-daemon.ts`): the
//! lockfile names it, `/health` confirms its pid; a start lock keeps browsers that wake it at once
//! from each spawning their own.

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use vexdom_core::lockfile::{self, Lockfile};
use vexdom_core::{log, paths};

const SPAWN_TIMEOUT: Duration = Duration::from_millis(8_000);
const POLL_INTERVAL: Duration = Duration::from_millis(150);
const STALE_START_MS: i64 = 16_000;
const HEALTH_TIMEOUT: Duration = Duration::from_secs(1);

fn start_lock() -> PathBuf {
    paths::state_dir().join("daemon.start.lock")
}

/// `process.kill(pid, 0)`: alive, or alive and not ours to signal. A zombie has exited: `kill`
/// still succeeds on it until its parent reaps it, which a parent that spawned the daemon itself
/// may not have done yet.
pub fn is_running(pid: u32) -> bool {
    let Ok(pid) = i32::try_from(pid) else {
        return false;
    };
    match nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid), None) {
        Ok(()) => !is_zombie(pid),
        Err(error) => error == nix::errno::Errno::EPERM,
    }
}

/// A process `kill` still finds: a zombie when its BSD info says `SZOMB` (`<sys/proc.h>`), or when
/// there is no BSD info to read at all, which is how macOS 27 answers for a zombie (`ESRCH`).
#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
fn is_zombie(pid: i32) -> bool {
    let mut info = std::mem::MaybeUninit::<libc::proc_bsdinfo>::zeroed();
    let size = std::mem::size_of::<libc::proc_bsdinfo>() as libc::c_int;
    // SAFETY: `info` is a writable buffer of exactly `size` bytes, which is all proc_pidinfo
    // writes for PROC_PIDTBSDINFO; it is read only when the call reports it filled all of it.
    let filled = unsafe {
        libc::proc_pidinfo(
            pid,
            libc::PROC_PIDTBSDINFO,
            0,
            info.as_mut_ptr().cast(),
            size,
        )
    };
    if filled != size {
        return nix::errno::Errno::last() == nix::errno::Errno::ESRCH;
    }
    // SAFETY: proc_pidinfo wrote the whole struct (checked), and it was zeroed before.
    unsafe { info.assume_init() }.pbi_status == libc::SZOMB
}

/// `/proc/<pid>/stat`: the state follows the command name, which may itself hold `) `.
#[cfg(target_os = "linux")]
fn is_zombie(pid: i32) -> bool {
    std::fs::read_to_string(format!("/proc/{pid}/stat")).is_ok_and(|stat| {
        stat.rfind(") ")
            .and_then(|at| stat[at + 2..].chars().next())
            == Some('Z')
    })
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn is_zombie(_pid: i32) -> bool {
    false
}

/// The pid `/health` on that port reports, within a second.
pub async fn healthy_pid(port: u16) -> Option<u32> {
    let probe = async {
        let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .ok()?;
        let request =
            format!("GET /health HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n");
        stream.write_all(request.as_bytes()).await.ok()?;
        let mut response = Vec::new();
        stream.read_to_end(&mut response).await.ok()?;
        let text = String::from_utf8_lossy(&response);
        let (head, body) = text.split_once("\r\n\r\n")?;
        if !head.starts_with("HTTP/1.1 200") {
            return None;
        }
        let health: serde_json::Value = serde_json::from_str(body.trim()).ok()?;
        health
            .get("pid")?
            .as_u64()
            .and_then(|pid| u32::try_from(pid).ok())
    };
    tokio::time::timeout(HEALTH_TIMEOUT, probe)
        .await
        .ok()
        .flatten()
}

/// The Bridge the lockfile names, if it answers as that pid; else one on another port whose pid
/// the lockfile names.
pub async fn probe_existing() -> Option<Lockfile> {
    let lock = lockfile::read(&paths::lockfile());
    if let Some(lock) = &lock
        && is_running(lock.pid)
        && healthy_pid(lock.port).await == Some(lock.pid)
    {
        return Some(lock.clone());
    }
    for port in paths::daemon_ports().unwrap_or_default() {
        if Some(port) == lock.as_ref().map(|lock| lock.port) {
            continue;
        }
        let Some(pid) = healthy_pid(port).await else {
            continue;
        };
        if let Some(current) =
            lockfile::read(&paths::lockfile()).filter(|current| current.pid == pid)
        {
            return Some(current);
        }
    }
    None
}

/// Held while this process spawns; `None` while another one is.
fn take_start_lock(retry: bool) -> Option<StartLock> {
    let _ = paths::ensure_private_dir(&paths::state_dir());
    let path = start_lock();
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    match options.open(&path) {
        Ok(mut file) => {
            let _ = std::io::Write::write_all(
                &mut file,
                format!("{} {}\n", std::process::id(), log::now_ms()).as_bytes(),
            );
            return Some(StartLock(Some(path)));
        }
        Err(error) if error.kind() != std::io::ErrorKind::AlreadyExists => {
            return Some(StartLock(None));
        }
        Err(_) => {}
    }
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    let mut parts = text
        .split_whitespace()
        .map(|part| part.parse::<i64>().unwrap_or(0));
    let (pid, at) = (parts.next().unwrap_or(0), parts.next().unwrap_or(0));
    if retry && (!is_running(pid as u32) || log::now_ms() - at > STALE_START_MS) {
        let _ = std::fs::remove_file(&path);
        return take_start_lock(false);
    }
    None
}

struct StartLock(Option<PathBuf>);

impl Drop for StartLock {
    fn drop(&mut self) {
        if let Some(path) = &self.0 {
            let _ = std::fs::remove_file(path);
        }
    }
}

/// The running Bridge, starting one (`<this binary> daemon`) when none answers: a detached child
/// of this process, through launchd when this process runs quarantined.
pub async fn ensure_daemon() -> Result<Lockfile, String> {
    let deadline = Instant::now() + SPAWN_TIMEOUT * 2;
    loop {
        if let Some(existing) = probe_existing().await {
            return Ok(existing);
        }
        if let Some(_lock) = take_start_lock(true) {
            if let Some(existing) = probe_existing().await {
                return Ok(existing);
            }
            return spawn_daemon().await;
        }
        if Instant::now() > deadline {
            return Err(
                "Another Vexdom Bridge was starting and never came up — see \"vexdom logs\"".into(),
            );
        }
        tokio::time::sleep(POLL_INTERVAL).await;
    }
}

/// What a client inherits that must not reach the Bridge it starts.
const NOT_INHERITED: [&str; 4] = [
    "VEXDOM_AGENT_RUN",
    "CLAUDECODE",
    "CLAUDE_CODE_ENTRYPOINT",
    "NODE_NO_WARNINGS",
];

/// One launchd started is waited for before another is spawned.
async fn spawn_daemon() -> Result<Lockfile, String> {
    let exe = std::env::current_exe().map_err(|error| error.to_string())?;
    if cfg!(target_os = "macos") && crate::quarantine::runs_quarantined() {
        let env: std::collections::BTreeMap<String, String> = std::env::vars()
            .filter(|(key, _)| !NOT_INHERITED.contains(&key.as_str()))
            .collect();
        if crate::quarantine::start_bridge_job(&exe, &env).await {
            log::log("no daemon reachable; starting one through launchd");
            if let Some(started) = wait_for_daemon(SPAWN_TIMEOUT).await {
                return Ok(started);
            }
            log::log(&format!(
                "the Bridge launchd started did not answer within {}ms; spawning one",
                SPAWN_TIMEOUT.as_millis()
            ));
        } else {
            log::log("no daemon reachable; spawning one");
        }
    } else {
        log::log("no daemon reachable; spawning one");
    }
    let mut command = Command::new(exe);
    command
        .arg("daemon")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    for name in NOT_INHERITED {
        command.env_remove(name);
    }
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(&mut command, 0);
    command
        .spawn()
        .map_err(|error| format!("cannot start Vexdom Bridge: {error}"))?;
    came_up().await
}

async fn wait_for_daemon(timeout: Duration) -> Option<Lockfile> {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        tokio::time::sleep(POLL_INTERVAL).await;
        if let Some(started) = probe_existing().await {
            return Some(started);
        }
    }
    None
}

async fn came_up() -> Result<Lockfile, String> {
    wait_for_daemon(SPAWN_TIMEOUT).await.ok_or_else(|| {
        format!(
            "The Vexdom daemon did not come up within {}ms — see the log with \"vexdom logs\"",
            SPAWN_TIMEOUT.as_millis()
        )
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RunningDaemon {
    pub pid: u32,
    pub port: u16,
}

/// Every Bridge actually answering on a Vexdom port.
pub async fn running_daemons() -> Vec<RunningDaemon> {
    let mut found: Vec<RunningDaemon> = Vec::new();
    for port in paths::daemon_ports().unwrap_or_default() {
        if let Some(pid) = healthy_pid(port).await
            && !found.iter().any(|daemon| daemon.pid == pid)
        {
            found.push(RunningDaemon { pid, port });
        }
    }
    found
}

pub struct StopResult {
    pub stopped: Vec<RunningDaemon>,
    pub stubborn: Vec<RunningDaemon>,
}

fn kill(pid: u32, hard: bool) {
    if let Ok(pid) = i32::try_from(pid) {
        let signal = if hard {
            nix::sys::signal::Signal::SIGKILL
        } else {
            nix::sys::signal::Signal::SIGTERM
        };
        let _ = nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid), signal);
    }
}

async fn wait_for_exit(daemons: &[RunningDaemon], timeout: Duration) -> Vec<RunningDaemon> {
    let deadline = Instant::now() + timeout;
    let mut alive: Vec<RunningDaemon> = daemons
        .iter()
        .copied()
        .filter(|d| is_running(d.pid))
        .collect();
    while !alive.is_empty() && Instant::now() < deadline {
        tokio::time::sleep(POLL_INTERVAL).await;
        alive.retain(|daemon| is_running(daemon.pid));
    }
    alive
}

/// Asks every Bridge to stop (the caller may first have sent `Control.stop`), then SIGTERM,
/// then SIGKILL after `grace`; holds the browser wake-up so a paired browser does not restart it.
pub async fn stop_daemons(grace: Duration) -> StopResult {
    let _ = lockfile::hold_wake();
    let daemons = running_daemons().await;
    for daemon in &daemons {
        kill(daemon.pid, false);
    }
    let remaining = wait_for_exit(&daemons, grace).await;
    for daemon in &remaining {
        kill(daemon.pid, true);
    }
    let stubborn = wait_for_exit(&remaining, Duration::from_secs(2)).await;
    let _ = std::fs::remove_file(paths::lockfile());
    StopResult {
        stopped: daemons
            .into_iter()
            .filter(|daemon| !stubborn.contains(daemon))
            .collect(),
        stubborn,
    }
}

#[cfg(test)]
mod tests {
    use std::process::Command;
    use std::time::Duration;

    use super::is_running;

    #[test]
    fn a_child_that_exited_but_was_not_reaped_is_not_running() {
        let mut child = Command::new("sh").args(["-c", "exit 0"]).spawn().unwrap();
        std::thread::sleep(Duration::from_millis(200));
        assert!(!is_running(child.id()), "a zombie is not running");
        child.wait().unwrap();
        assert!(is_running(std::process::id()));
    }
}
