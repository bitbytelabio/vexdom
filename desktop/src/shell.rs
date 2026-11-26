//! The PATH a terminal would have (browsentic `Shell.swift`). An app opened from the Finder gets
//! launchd's PATH, with neither Homebrew nor `~/.local/bin` on it, so every child the app starts —
//! the `vexdom` CLI, the Bridge that CLI starts, npm — gets the login shell's PATH instead. Also
//! linking `vexdom` onto that PATH (browsentic `CommandLink`).

use std::ffi::{OsStr, OsString};
use std::io::Read as _;
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

/// How long the login shell gets to say its PATH.
const ASK_LIMIT: Duration = Duration::from_secs(2);

/// Brackets the PATH in the shell's output, past whatever its startup files print.
const MARKER: &str = "__VEXDOM_PATH__";

/// The name the command line is linked under.
const NAME: &str = "vexdom";

/// The login shell's PATH, asked once (`$SHELL -l -c`); the usual folders when it does not answer
/// within two seconds.
pub fn login_path() -> &'static [PathBuf] {
    static LOGIN: OnceLock<Vec<PathBuf>> = OnceLock::new();
    LOGIN.get_or_init(|| ask_login_shell().unwrap_or_else(fallback))
}

/// [`login_path`] as one `PATH` value.
pub fn path_var() -> OsString {
    std::env::join_paths(login_path()).unwrap_or_default()
}

/// A command for `program` with the login PATH, reading nothing from stdin.
pub fn command(program: impl AsRef<OsStr>) -> tokio::process::Command {
    let mut command = tokio::process::Command::new(program);
    command.env("PATH", path_var()).stdin(Stdio::null());
    command
}

/// The first `name` on the login PATH that can run.
pub fn which(name: &str) -> Option<PathBuf> {
    login_path()
        .iter()
        .map(|dir| dir.join(name))
        .find(|candidate| runnable(candidate))
}

fn runnable(path: &Path) -> bool {
    std::fs::metadata(path)
        .is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
}

fn ask_login_shell() -> Option<Vec<PathBuf>> {
    let shell = std::env::var_os("SHELL")
        .filter(|shell| !shell.is_empty())
        .unwrap_or_else(|| "/bin/zsh".into());
    let mut child = Command::new(shell)
        .arg("-l")
        .arg("-c")
        .arg(format!("printf '{MARKER}%s{MARKER}' \"$PATH\""))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let deadline = Instant::now() + ASK_LIMIT;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(20)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
    let mut out = String::new();
    child.stdout.take()?.read_to_string(&mut out).ok()?;
    marked_path(&out)
}

/// The folders between the markers, each once, in order; `None` without the markers or folders.
fn marked_path(output: &str) -> Option<Vec<PathBuf>> {
    let mut parts = output.split(MARKER);
    parts.next()?;
    let path = parts.next()?;
    parts.next()?;
    let mut dirs: Vec<PathBuf> = Vec::new();
    for dir in std::env::split_paths(path) {
        if !dir.as_os_str().is_empty() && !dirs.contains(&dir) {
            dirs.push(dir);
        }
    }
    (!dirs.is_empty()).then_some(dirs)
}

fn fallback() -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = ["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin", "/bin"]
        .into_iter()
        .map(PathBuf::from)
        .collect();
    dirs.push(vexdom_core::paths::home_dir().join(".local/bin"));
    dirs
}

/// Where `vexdom` may be linked, in order: `~/.local/bin`, `/opt/homebrew/bin`, `/usr/local/bin`,
/// each only if the login PATH has it.
pub fn link_dirs(home: &Path, login: &[PathBuf]) -> Vec<PathBuf> {
    [
        home.join(".local/bin"),
        PathBuf::from("/opt/homebrew/bin"),
        PathBuf::from("/usr/local/bin"),
    ]
    .into_iter()
    .filter(|dir| login.contains(dir))
    .collect()
}

/// The folder on `login` whose `vexdom` is a link to `target`.
pub fn linked_in(login: &[PathBuf], target: &Path) -> Option<PathBuf> {
    login
        .iter()
        .find(|dir| std::fs::read_link(dir.join(NAME)).is_ok_and(|to| to == target))
        .cloned()
}

/// Links `target` as `vexdom` into the first of `dirs` that takes it: a link already there is
/// replaced, a file there is somebody else's `vexdom` and stops it.
pub fn link(target: &Path, dirs: &[PathBuf]) -> Result<PathBuf, String> {
    let mut refused = None;
    for dir in dirs {
        let link = dir.join(NAME);
        if let Ok(meta) = std::fs::symlink_metadata(&link) {
            if !meta.file_type().is_symlink() {
                return Err(format!(
                    "{} is another vexdom. Remove it, then link again.",
                    link.display()
                ));
            }
            if let Err(error) = std::fs::remove_file(&link) {
                refused = Some(error);
                continue;
            }
        }
        match std::os::unix::fs::symlink(target, &link) {
            Ok(()) => return Ok(dir.clone()),
            Err(error) => refused = Some(error),
        }
    }
    Err(match refused {
        Some(error) => format!("No folder on your PATH takes the link ({error})."),
        None => "None of ~/.local/bin, /opt/homebrew/bin or /usr/local/bin is on your PATH. Add ~/.local/bin to it in your shell's profile.".to_owned(),
    })
}

/// Removes the links to `target` from the folders on `login`; anything else is left alone.
pub fn unlink(target: &Path, login: &[PathBuf]) -> usize {
    let ours = login
        .iter()
        .map(|dir| dir.join(NAME))
        .filter(|link| std::fs::read_link(link).is_ok_and(|to| to == target));
    let mut removed = 0;
    for link in ours {
        if std::fs::remove_file(&link).is_ok() {
            removed += 1;
        }
    }
    removed
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "vexdom-desktop-shell-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("a scratch folder");
        dir
    }

    #[test]
    fn the_path_is_read_between_the_markers() {
        let said =
            format!("Welcome!\n{MARKER}/opt/homebrew/bin:/usr/bin::/opt/homebrew/bin{MARKER}");
        assert_eq!(
            marked_path(&said),
            Some(vec![
                PathBuf::from("/opt/homebrew/bin"),
                PathBuf::from("/usr/bin")
            ])
        );
        assert_eq!(marked_path("no markers"), None);
        assert_eq!(marked_path(&format!("{MARKER}/usr/bin")), None);
        assert_eq!(marked_path(&format!("{MARKER}{MARKER}")), None);
    }

    #[test]
    fn links_go_to_the_first_usual_folder_on_the_path() {
        let home = Path::new("/Users/ada");
        let login = [
            PathBuf::from("/usr/local/bin"),
            PathBuf::from("/Users/ada/.local/bin"),
            PathBuf::from("/usr/bin"),
        ];
        assert_eq!(
            link_dirs(home, &login),
            [
                PathBuf::from("/Users/ada/.local/bin"),
                PathBuf::from("/usr/local/bin")
            ]
        );
        assert!(link_dirs(home, &[PathBuf::from("/usr/bin")]).is_empty());
    }

    #[test]
    fn linking_skips_a_folder_it_cannot_write_and_unlinking_takes_only_ours() {
        let root = scratch("link");
        let locked = root.join("locked");
        let open = root.join("open");
        std::fs::create_dir_all(&locked).expect("made");
        std::fs::create_dir_all(&open).expect("made");
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o555)).expect("locked");
        let target = root.join("Vexdom.app/Contents/MacOS/vexdom");
        let dirs = [locked.clone(), open.clone()];
        assert_eq!(link(&target, &dirs), Ok(open.clone()));
        assert_eq!(linked_in(&dirs, &target), Some(open.clone()));
        // Linking again replaces our own link.
        assert_eq!(link(&target, &dirs), Ok(open.clone()));
        // Someone else's link stays.
        let other = root.join("other");
        std::fs::create_dir_all(&other).expect("made");
        std::os::unix::fs::symlink("/usr/bin/true", other.join(NAME)).expect("linked");
        let login = [locked.clone(), open.clone(), other.clone()];
        assert_eq!(unlink(&target, &login), 1);
        assert_eq!(linked_in(&login, &target), None);
        assert!(std::fs::symlink_metadata(other.join(NAME)).is_ok());
        // A file in the way is somebody else's vexdom.
        std::fs::write(open.join(NAME), b"#!/bin/sh\n").expect("written");
        assert!(
            link(&target, std::slice::from_ref(&open))
                .unwrap_err()
                .contains("is another vexdom")
        );
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755))
            .expect("unlocked");
        let _ = std::fs::remove_dir_all(&root);
    }
}
