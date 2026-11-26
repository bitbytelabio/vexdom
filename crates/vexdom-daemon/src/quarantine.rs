//! macOS: a browser runs its native hosts quarantined, and every process a quarantined one starts
//! inherits the flag, so a Bridge spawned from the host writes quarantined files and Gatekeeper
//! refuses what an agent unpacks. A launchd job starts the Bridge outside that chain
//! (`quarantine.ts`).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use sha2::{Digest, Sha256};
use vexdom_core::{log, paths};

const QUARANTINE: &str = "com.apple.quarantine";
const USER_APPROVED: u32 = 0x40;
const BOOTSTRAP_RETRY: Duration = Duration::from_millis(300);

/// One per Bridge home, so that a scratch Bridge never boots out the real one.
pub fn bridge_job() -> String {
    let digest = Sha256::digest(paths::state_dir().to_string_lossy().as_bytes());
    let hex: String = digest
        .iter()
        .take(4)
        .map(|byte| format!("{byte:02x}"))
        .collect();
    format!("com.vexdom.bridge.{hex}")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum JobState {
    Running,
    Idle,
    Absent,
}

fn launchctl(args: &[&str]) -> Result<String, String> {
    let out = Command::new("/bin/launchctl")
        .args(args)
        .stdin(Stdio::null())
        .output()
        .map_err(|error| error.to_string())?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        let stderr = String::from_utf8_lossy(&out.stderr).trim().to_owned();
        Err(if stderr.is_empty() {
            format!("launchctl {} exited with {}", args[0], out.status)
        } else {
            stderr
        })
    }
}

fn domain() -> String {
    let uid = nix::unistd::getuid().as_raw();
    format!("gui/{uid}")
}

fn state(label: &str) -> JobState {
    match launchctl(&["print", &format!("{}/{label}", domain())]) {
        Ok(out) if out.lines().any(|line| line == "\tstate = running") => JobState::Running,
        Ok(_) => JobState::Idle,
        Err(_) => JobState::Absent,
    }
}

fn escape_xml(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn plist_string(value: &str) -> String {
    format!("<string>{}</string>", escape_xml(value))
}

/// The browser's own launchd service and bundle, which this job is not.
fn names_the_browser(key: &str) -> bool {
    key.starts_with("XPC_") || key == "__CFBundleIdentifier"
}

/// Runs once, as it loads, with the host's environment, and leaves what it started running when it
/// exits, as the detached spawn it stands in for does. Interactive spares it the CPU and I/O
/// throttling launchd gives a job of no declared type.
pub fn bridge_job_plist(exe: &Path, env: &BTreeMap<String, String>) -> String {
    let mut lines = vec![
        r#"<?xml version="1.0" encoding="UTF-8"?>"#.to_owned(),
        r#"<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">"#.to_owned(),
        r#"<plist version="1.0">"#.to_owned(),
        "<dict>".to_owned(),
        format!("  <key>Label</key>{}", plist_string(&bridge_job())),
        format!(
            "  <key>ProgramArguments</key><array>{}{}</array>",
            plist_string(&exe.to_string_lossy()),
            plist_string("daemon")
        ),
        "  <key>EnvironmentVariables</key>".to_owned(),
        "  <dict>".to_owned(),
    ];
    for (key, value) in env.iter().filter(|(key, _)| !names_the_browser(key)) {
        lines.push(format!(
            "    <key>{}</key>{}",
            escape_xml(key),
            plist_string(value)
        ));
    }
    lines.extend(
        [
            "  </dict>",
            "  <key>RunAtLoad</key><true/>",
            "  <key>AbandonProcessGroup</key><true/>",
            "  <key>ProcessType</key><string>Interactive</string>",
            "</dict>",
            "</plist>",
            "",
        ]
        .map(str::to_owned),
    );
    lines.join("\n")
}

/// Starts the Bridge as a launchd job. A job launchd is still running is a Bridge on its way up, or
/// one too busy to answer, so it is waited for and never booted out. False when launchd would not
/// take the job, and the caller spawns the Bridge itself.
pub async fn start_bridge_job(exe: &Path, env: &BTreeMap<String, String>) -> bool {
    let dir = paths::state_dir().join("native-host");
    let file = dir.join("bridge-job.plist");
    let label = bridge_job();
    let started = async {
        match state(&label) {
            JobState::Running => return Ok(()),
            JobState::Idle => {
                launchctl(&["bootout", &format!("{}/{label}", domain())]).map(|_| ())?
            }
            JobState::Absent => {}
        }
        paths::ensure_private_dir(&dir).map_err(|error| error.to_string())?;
        paths::write_private(&file, bridge_job_plist(exe, env).as_bytes())
            .map_err(|error| error.to_string())?;
        let plist = file.to_string_lossy().into_owned();
        if launchctl(&["bootstrap", &domain(), &plist]).is_err() {
            // launchd can still be taking the old job down when the new one arrives.
            tokio::time::sleep(BOOTSTRAP_RETRY).await;
            launchctl(&["bootstrap", &domain(), &plist])?;
        }
        Ok::<(), String>(())
    }
    .await;
    let _ = std::fs::remove_file(&file);
    match started {
        Ok(()) => true,
        Err(error) => {
            log::log(&format!(
                "native host: could not start the Bridge through launchd: {error}"
            ));
            false
        }
    }
}

/// On uninstall: launchd keeps a job, and the environment it carries, until logout.
pub fn remove_bridge_job() -> bool {
    let label = bridge_job();
    state(&label) != JobState::Absent
        && launchctl(&["bootout", &format!("{}/{label}", domain())]).is_ok()
}

fn quarantine_of(file: &Path) -> Option<String> {
    let out = Command::new("/usr/bin/xattr")
        .args(["-p", QUARANTINE])
        .arg(file)
        .stderr(Stdio::null())
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_owned())
}

/// Whether what this process writes comes out quarantined, passed on by whatever started it.
pub fn runs_quarantined() -> bool {
    let dir = paths::state_dir();
    let probe = dir.join(format!(".quarantine-probe-{}", std::process::id()));
    let quarantined = paths::ensure_private_dir(&dir).is_ok()
        && std::fs::write(&probe, b"").is_ok()
        && quarantine_of(&probe).is_some();
    let _ = std::fs::remove_file(&probe);
    quarantined
}

/// The user let it run with Open Anyway, which Gatekeeper records in the flag instead of removing it.
fn approved(quarantine: &str) -> bool {
    let flags = quarantine.split(';').next().unwrap_or("");
    u32::from_str_radix(flags, 16).is_ok_and(|flags| flags & USER_APPROVED != 0)
}

/// Bun-built CLIs, OpenCode among them, unpack their native libraries into the temporary folder as
/// `.bun-<uid>-<hash>.<ext>` and reuse that copy on every later start. One unpacked under a
/// quarantined Bridge stays quarantined, and Gatekeeper refuses it with a dialog at each start until
/// it is gone.
pub fn drop_quarantined_bun_libraries(dir: &Path, uid: u32) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let prefix = format!(".bun-{uid}-");
    let mut dropped = Vec::new();
    for entry in entries.filter_map(Result::ok) {
        if !entry.file_name().to_string_lossy().starts_with(&prefix) {
            continue;
        }
        let file = entry.path();
        let quarantined = quarantine_of(&file).is_some_and(|flag| !approved(&flag));
        if quarantined && std::fs::remove_file(&file).is_ok() {
            dropped.push(file);
        }
    }
    dropped
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plist_runs_the_daemon_without_the_browser_env() {
        let env = BTreeMap::from([
            ("HOME".to_owned(), "/Users/a&b".to_owned()),
            ("XPC_SERVICE_NAME".to_owned(), "x".to_owned()),
            ("__CFBundleIdentifier".to_owned(), "y".to_owned()),
        ]);
        let plist = bridge_job_plist(Path::new("/opt/vexdom"), &env);
        assert!(
            plist.contains("<array><string>/opt/vexdom</string><string>daemon</string></array>")
        );
        assert!(plist.contains("<key>HOME</key><string>/Users/a&amp;b</string>"));
        assert!(!plist.contains("XPC_SERVICE_NAME") && !plist.contains("__CFBundleIdentifier"));
    }

    #[test]
    fn user_approval_flag() {
        assert!(approved("01c1;6650a1b2;Chrome;"));
        assert!(!approved("0081;6650a1b2;Chrome;"));
        assert!(!approved("garbage"));
    }
}
