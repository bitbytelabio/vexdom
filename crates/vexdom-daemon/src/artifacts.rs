//! Files the Bridge writes for the user (ADR-030): a PDF, a HAR, a trace. The extension hands the
//! bytes over `Page.artifact`; the file goes to the absolute `path` the caller gave, else to
//! `~/vexdom/captures/<stem>-<unix_ms>.<ext>`. Never into the Bridge's state folder.

use std::io::Write as _;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use vexdom_core::paths;
use vexdom_core::result::ActionError;

use crate::link::Reply;

/// Actions whose answer is a file.
pub const ARTIFACT_ACTIONS: &[&str] = &["page.pdf", "page.harStop", "page.traceStop"];

/// The most one artifact may be, in bytes.
pub const ARTIFACT_MAX_BYTES: usize = 48 * 1024 * 1024;

fn invalid(message: impl Into<String>) -> ActionError {
    ActionError {
        code: "INVALID_INPUT".into(),
        message: message.into(),
    }
}

/// An artifact action that writes a file (`page.harStop` with `inline` returns the HAR instead).
pub fn is_artifact(action: &str, input: &Value) -> bool {
    ARTIFACT_ACTIONS.contains(&action) && input.get("inline") != Some(&Value::Bool(true))
}

/// The caller's `path`, checked before anything runs: absolute, and outside the state folder.
pub fn requested_path(input: &Value) -> Result<Option<PathBuf>, ActionError> {
    let Some(path) = input
        .get("path")
        .and_then(Value::as_str)
        .filter(|p| !p.is_empty())
    else {
        return Ok(None);
    };
    let path = PathBuf::from(path);
    if !path.is_absolute() {
        return Err(invalid("path must be absolute"));
    }
    if inside_state_dir(&path) {
        return Err(invalid(format!(
            "{} is inside the Bridge's own folder; choose another place",
            path.display()
        )));
    }
    Ok(Some(path))
}

/// Symlinks resolved on the nearest existing ancestor, so a link cannot reach the state folder.
fn inside_state_dir(path: &Path) -> bool {
    let state = std::fs::canonicalize(paths::state_dir()).unwrap_or_else(|_| paths::state_dir());
    let mut existing = path;
    let mut rest = Vec::new();
    while !existing.exists() {
        let (Some(parent), Some(name)) = (existing.parent(), existing.file_name()) else {
            break;
        };
        rest.push(name.to_owned());
        existing = parent;
    }
    let mut real = std::fs::canonicalize(existing).unwrap_or_else(|_| existing.to_path_buf());
    for name in rest.into_iter().rev() {
        real.push(name);
    }
    real.starts_with(&state)
}

fn default_path(result: &Value) -> PathBuf {
    let file = result.get("file");
    let stem = file
        .and_then(|f| f.get("stem"))
        .and_then(Value::as_str)
        .unwrap_or("capture");
    let ext = file
        .and_then(|f| f.get("ext"))
        .and_then(Value::as_str)
        .unwrap_or("bin");
    paths::user_dir()
        .join("captures")
        .join(format!("{stem}-{}.{ext}", vexdom_core::log::now_ms()))
}

#[cfg(unix)]
fn private_dir(dir: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::DirBuilderExt as _;
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)
}

#[cfg(not(unix))]
fn private_dir(dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)
}

fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        private_dir(parent)?;
    }
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    options.open(path)?.write_all(bytes)
}

/// Writes the bytes an artifact action returned and answers `{...result, path, bytes}`.
pub fn write_artifact(requested: Option<PathBuf>, mut reply: Reply) -> Reply {
    let Ok(result) = &mut reply.result else {
        return reply;
    };
    let data = reply.image.take().unwrap_or_default();
    if data.len() > ARTIFACT_MAX_BYTES {
        return Err(ActionError {
            code: "ARTIFACT_TOO_LARGE".into(),
            message: format!(
                "The file came to {} bytes; the most the Bridge writes is {ARTIFACT_MAX_BYTES}.",
                data.len()
            ),
        })
        .into();
    }
    let path = requested.unwrap_or_else(|| default_path(result));
    if let Err(error) = write_private(&path, &data) {
        return Err(ActionError {
            code: "ACTION_FAILED".into(),
            message: format!("Could not write {}: {error}", path.display()),
        })
        .into();
    }
    let mut out = match result.take() {
        Value::Object(object) => object,
        _ => serde_json::Map::new(),
    };
    out.shift_remove("file");
    out.insert("path".into(), json!(path.to_string_lossy()));
    out.insert("bytes".into(), json!(data.len()));
    Ok(Value::Object(out)).into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_path_must_be_absolute_and_outside_the_state_folder() {
        assert_eq!(
            requested_path(&json!({"path": "rel/x.pdf"}))
                .unwrap_err()
                .message,
            "path must be absolute"
        );
        let inside = paths::state_dir().join("x.pdf");
        assert!(requested_path(&json!({"path": inside})).is_err());
        assert_eq!(requested_path(&json!({})).unwrap(), None);
        assert!(is_artifact("page.pdf", &json!({})));
        assert!(!is_artifact("page.harStop", &json!({"inline": true})));
    }

    #[test]
    fn writes_the_bytes_and_reports_where() {
        let dir = std::env::temp_dir().join(format!("vexdom-artifact-{}", std::process::id()));
        let path = dir.join("page.pdf");
        let reply = Reply {
            result: Ok(json!({"pages": 1, "file": {"stem": "page", "ext": "pdf"}})),
            image: Some(b"%PDF-1.7".to_vec()),
        };
        let written = write_artifact(Some(path.clone()), reply).result.unwrap();
        assert_eq!(written["bytes"], 8);
        assert_eq!(written["pages"], 1);
        assert!(written.get("file").is_none());
        assert_eq!(std::fs::read(&path).unwrap(), b"%PDF-1.7");
        let _ = std::fs::remove_dir_all(dir);
    }
}
