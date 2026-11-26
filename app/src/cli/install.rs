//! Copies the packaged extension into the folder the browser loads unpacked (`install.ts`).
//!
//! Not a directory swap. Renaming the old tree aside leaves the path briefly non-existent, and a
//! browser that starts in that window disables the extension and drops it from the profile. Renaming
//! over a *file* is atomic and stays safe while another process holds the old inode open.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

const STAMP: &str = ".vexdom-install.json";

/// Records what `install` last wrote, so `status` can report installed-versus-loaded.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallStamp {
    pub version: String,
    pub installed_at: String,
    pub source: String,
    pub files: usize,
}

#[derive(Debug, Clone)]
pub struct InstallResult {
    pub version: String,
    pub changed: usize,
    pub already_current: bool,
}

#[derive(Debug)]
pub struct InstallError {
    pub message: String,
    pub hint: Option<&'static str>,
}

pub fn read_stamp(dir: &Path) -> Option<InstallStamp> {
    serde_json::from_slice(&std::fs::read(dir.join(STAMP)).ok()?).ok()
}

/// The extension build this binary ships or sits beside: `<exe>/../extension/chrome-mv3` in a
/// package, `Contents/Resources/extension/chrome-mv3` from `Contents/MacOS/vexdom` in
/// `Vexdom.app`, `<repo>/packages/extension/dist/chrome-mv3` from `target/<profile>/vexdom` in a
/// source checkout. A symlink to the binary (`~/.local/bin/vexdom`) resolves to the real file
/// first: `current_exe` returns the link on macOS.
fn packaged_extension() -> Option<(PathBuf, &'static str)> {
    let exe = std::env::current_exe().ok()?;
    let exe = std::fs::canonicalize(&exe).unwrap_or(exe);
    let here = exe.parent()?;
    let candidates = [
        (
            here.join("..").join("extension").join("chrome-mv3"),
            "package",
        ),
        (
            here.join("..")
                .join("Resources")
                .join("extension")
                .join("chrome-mv3"),
            "app",
        ),
        (
            here.join("..")
                .join("..")
                .join("packages")
                .join("extension")
                .join("dist")
                .join("chrome-mv3"),
            "repo",
        ),
    ];
    candidates
        .into_iter()
        .find(|(dir, _)| dir.join("manifest.json").exists())
}

/// Every file under `dir`, relative to it.
fn walk(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut pending = vec![PathBuf::new()];
    while let Some(relative) = pending.pop() {
        let Ok(entries) = std::fs::read_dir(dir.join(&relative)) else {
            continue;
        };
        for entry in entries.filter_map(Result::ok) {
            let path = relative.join(entry.file_name());
            if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
                pending.push(path);
            } else {
                out.push(path);
            }
        }
    }
    out.sort();
    out
}

fn same_content(a: &Path, b: &Path) -> bool {
    let (Ok(left), Ok(right)) = (std::fs::metadata(a), std::fs::metadata(b)) else {
        return false;
    };
    if left.len() != right.len() {
        return false;
    }
    match (std::fs::read(a), std::fs::read(b)) {
        (Ok(left), Ok(right)) => Sha256::digest(left) == Sha256::digest(right),
        _ => false,
    }
}

fn is_leftover(path: &Path) -> bool {
    let name = path.to_string_lossy();
    name.rsplit_once(".tmp-")
        .is_some_and(|(_, pid)| !pid.is_empty() && pid.bytes().all(|b| b.is_ascii_digit()))
}

fn copy_one(from: &Path, to: &Path) -> std::io::Result<()> {
    if let Some(parent) = to.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = PathBuf::from(format!("{}.tmp-{}", to.display(), std::process::id()));
    let copied = std::fs::copy(from, &tmp).and_then(|_| {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o644))?;
        }
        std::fs::rename(&tmp, to)
    });
    if copied.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    copied
}

pub fn install(dir: &Path, force: bool) -> Result<InstallResult, InstallError> {
    // Files the new build dropped are pruned below, so a folder that holds anything but an earlier
    // copy of the extension is never written into: `--dir ~/Documents` would lose the documents.
    let holds_other_files = read_stamp(dir).is_none()
        && std::fs::read_dir(dir).is_ok_and(|mut entries| entries.next().is_some());
    if holds_other_files {
        return Err(InstallError {
            message: format!(
                "{} already holds files that are not Vexdom's",
                dir.display()
            ),
            hint: Some("Pick an empty folder, or one an earlier `vexdom setup` wrote, with --dir."),
        });
    }
    let Some((packaged, source)) = packaged_extension() else {
        return Err(InstallError {
            message: "this build carries no extension payload".into(),
            hint: Some("Reinstall vexdom, or run `bun run build` if you are in a source checkout."),
        });
    };
    let manifest: Value = std::fs::read(packaged.join("manifest.json"))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or(Value::Null);
    let version = manifest
        .get("version")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned();
    if !force && read_stamp(dir).is_some_and(|stamp| stamp.version == version) {
        return Ok(InstallResult {
            version,
            changed: 0,
            already_current: true,
        });
    }
    let io = |error: std::io::Error| InstallError {
        message: error.to_string(),
        hint: None,
    };
    let sources = walk(&packaged);
    std::fs::create_dir_all(dir).map_err(io)?;
    // Leftovers from a run that died mid-copy.
    for stale in walk(dir).into_iter().filter(|path| is_leftover(path)) {
        let _ = std::fs::remove_file(dir.join(stale));
    }
    // The manifest goes last: an old manifest naming a superset of the files present loads; a new
    // one naming files not there yet is a broken extension.
    let manifest_path = PathBuf::from("manifest.json");
    let ordered = sources
        .iter()
        .filter(|path| **path != manifest_path)
        .chain(std::iter::once(&manifest_path));
    let mut changed = 0;
    for relative in ordered {
        let (from, to) = (packaged.join(relative), dir.join(relative));
        if !force && same_content(&from, &to) {
            continue;
        }
        if let Err(error) = copy_one(&from, &to) {
            return Err(
                if error.kind() == std::io::ErrorKind::PermissionDenied
                    || error.raw_os_error() == Some(16)
                {
                    InstallError {
                        message: format!("the browser is holding {} open", relative.display()),
                        hint: Some(
                            "Disable the Vexdom card at chrome://extensions (or quit the browser), then run `vexdom update` again.",
                        ),
                    }
                } else {
                    io(error)
                },
            );
        }
        changed += 1;
    }
    // Anything the new build dropped. After the manifest, for the same reason.
    for relative in walk(dir) {
        if !sources.contains(&relative) && relative != Path::new(STAMP) {
            let _ = std::fs::remove_file(dir.join(relative));
        }
    }
    let stamp = InstallStamp {
        version: version.clone(),
        installed_at: vexdom_core::log::iso_now(),
        source: source.into(),
        files: sources.len(),
    };
    let text = format!(
        "{}\n",
        serde_json::to_string_pretty(&stamp).unwrap_or_default()
    );
    std::fs::write(dir.join(STAMP), text).map_err(io)?;
    Ok(InstallResult {
        version,
        changed,
        already_current: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn never_writes_into_a_folder_of_other_files() {
        let dir = std::env::temp_dir().join(format!("vexdom-install-other-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("a scratch folder");
        std::fs::write(dir.join("notes.txt"), "mine").expect("a user's file");
        let refused = install(&dir, false).expect_err("a folder of other files is refused");
        assert!(
            refused.message.contains("not Vexdom's"),
            "{}",
            refused.message
        );
        assert!(
            dir.join("notes.txt").exists(),
            "the user's file is left alone"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn leftovers_are_tmp_pid_files() {
        assert!(is_leftover(Path::new("a/b.js.tmp-123")));
        assert!(!is_leftover(Path::new("a/b.js")));
        assert!(!is_leftover(Path::new("a/b.tmp-x")));
    }
}
