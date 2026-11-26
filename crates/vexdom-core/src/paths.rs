//! Where the Bridge keeps its state, and which ports it walks: `src/daemon/{paths,lockfile,ports}.ts`.

use std::env;
use std::path::{Path, PathBuf};

use crate::proto::DAEMON_PORTS;

/// `VEXDOM_HOME` when set, even empty (TS `??`), else `~/.vexdom`.
pub fn state_dir() -> PathBuf {
    match env::var_os("VEXDOM_HOME") {
        Some(dir) => PathBuf::from(dir),
        None => home_dir().join(".vexdom"),
    }
}

/// The visible half: skills, recordings, screenshots, the unpacked extension. Not relocatable.
pub fn user_dir() -> PathBuf {
    home_dir().join("vexdom")
}

pub fn home_dir() -> PathBuf {
    env::var_os("HOME").map(PathBuf::from).unwrap_or_default()
}

/// Where clients find the Bridge: `VEXDOM_LOCKFILE` when non-empty (TS `||`).
pub fn lockfile() -> PathBuf {
    match env::var_os("VEXDOM_LOCKFILE") {
        Some(path) if !path.is_empty() => PathBuf::from(path),
        _ => state_dir().join("daemon.json"),
    }
}

/// Saved browser states (`vexdom state save`): cookies and web storage, one JSON file each.
pub fn states_dir() -> PathBuf {
    state_dir().join("states")
}

pub fn log_path() -> PathBuf {
    state_dir().join("daemon.log")
}

/// Left by `vexdom stop`, cleared by the next daemon: while present the browser's wake-up leaves
/// the daemon down.
pub fn stopped_path() -> PathBuf {
    state_dir().join("stopped")
}

/// `vexdom setup`'s install folder for the unpacked extension. Never versioned: Chrome derives the
/// extension id from this path.
pub fn extension_dir(override_dir: Option<&Path>) -> PathBuf {
    override_dir.map_or_else(
        || user_dir().join("extension").join("chrome-mv3"),
        Path::to_path_buf,
    )
}

/// Records what `install` last wrote into an extension folder.
pub fn install_stamp_path(dir: &Path) -> PathBuf {
    dir.join(".vexdom-install.json")
}

/// A folder named in config.json: `~` and `~/…` start at the home folder,
/// and so does any other relative path.
pub fn expand_home(path: &str) -> PathBuf {
    if path == "~" {
        return home_dir();
    }
    if let Some(rest) = path.strip_prefix("~/") {
        return home_dir().join(rest);
    }
    let candidate = Path::new(path);
    if candidate.is_absolute() {
        candidate.to_path_buf()
    } else {
        home_dir().join(candidate)
    }
}

/// Creates `dir` (and parents) with mode 0700.
pub fn ensure_private_dir(dir: &Path) -> std::io::Result<()> {
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
    builder.create(dir)
}

/// `JSON.stringify(value, null, 2) + "\n"` written as a 0600 file, then chmod 0600, in a 0700
/// directory: how every JSON file under the state directory is written.
pub fn write_private_json<T: serde::Serialize + ?Sized>(
    path: &Path,
    value: &T,
) -> std::io::Result<()> {
    let mut text = serde_json::to_string_pretty(value).map_err(std::io::Error::other)?;
    text.push('\n');
    write_private(path, text.as_bytes())
}

/// Writes `bytes` as a 0600 file (then chmod 0600) in a 0700 directory.
pub fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write as _;
    if let Some(dir) = path.parent() {
        ensure_private_dir(dir)?;
    }
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    options.open(path)?.write_all(bytes)?;
    #[cfg(unix)]
    std::fs::set_permissions(path, std::os::unix::fs::PermissionsExt::from_mode(0o600))?;
    Ok(())
}

/// `VEXDOM_PORTS` as `ports.ts` reads it: unset or empty → the default ports; each entry is a JS
/// `Number()` of the trimmed text (so an empty entry is 0) and must be an integer 0–65535.
pub fn daemon_ports() -> Result<Vec<u16>, String> {
    let value = env::var("VEXDOM_PORTS").unwrap_or_default();
    parse_ports(&value)
}

fn parse_ports(value: &str) -> Result<Vec<u16>, String> {
    if value.is_empty() {
        return Ok(DAEMON_PORTS.to_vec());
    }
    value
        .split(',')
        .map(|entry| js_port(entry.trim()))
        .collect::<Option<Vec<u16>>>()
        .ok_or_else(|| {
            format!("VEXDOM_PORTS must be a comma-separated list of ports, not \"{value}\"")
        })
}

/// `Number(text)` restricted to what can pass `Number.isInteger(n) && 0 <= n <= 65535`.
fn js_port(text: &str) -> Option<u16> {
    let number = if text.is_empty() {
        0.0
    } else if let Some(radix) = [
        ("0x", 16),
        ("0X", 16),
        ("0o", 8),
        ("0O", 8),
        ("0b", 2),
        ("0B", 2),
    ]
    .iter()
    .find(|(prefix, _)| text.starts_with(prefix))
    {
        return u32::from_str_radix(&text[2..], radix.1)
            .ok()
            .and_then(|n| u16::try_from(n).ok());
    } else if text
        .chars()
        .all(|c| c.is_ascii_digit() || matches!(c, '+' | '-' | '.' | 'e' | 'E'))
    {
        text.parse::<f64>().ok()?
    } else {
        return None;
    };
    (number.fract() == 0.0 && (0.0..=65_535.0).contains(&number)).then_some(number as u16)
}

#[cfg(test)]
mod tests {
    use super::parse_ports;

    #[test]
    fn ports_parse_like_number() {
        assert_eq!(parse_ports(""), Ok(vec![8785, 8786, 8787]));
        assert_eq!(parse_ports("0"), Ok(vec![0]));
        assert_eq!(parse_ports(" 18790 , 18791"), Ok(vec![18790, 18791]));
        assert_eq!(parse_ports("8785,"), Ok(vec![8785, 0]));
        assert_eq!(parse_ports("1e3,0x10,+5"), Ok(vec![1000, 16, 5]));
        assert_eq!(parse_ports("-0"), Ok(vec![0]));
        for bad in ["abc", "65536", "1.5", "-1", "Infinity", "8785;8786"] {
            assert_eq!(
                parse_ports(bad),
                Err(format!(
                    "VEXDOM_PORTS must be a comma-separated list of ports, not \"{bad}\""
                ))
            );
        }
    }
}
