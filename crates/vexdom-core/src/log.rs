//! Lines in the Bridge log, `src/daemon/log.ts`: `<ISO> <message>[ <detail>]`, rotated to
//! `daemon.log.1` past 2 MiB, echoed to stderr under `VEXDOM_DEBUG`.

use std::fs::{self, OpenOptions};
use std::io::Write;

use crate::paths;

const MAX_BYTES: u64 = 2 * 1024 * 1024;

/// `new Date().toISOString()`: UTC, milliseconds, `Z`.
pub fn iso_now() -> String {
    iso(jiff::Timestamp::now())
}

/// A timestamp as `Date.prototype.toISOString` prints it.
pub fn iso(at: jiff::Timestamp) -> String {
    at.strftime("%Y-%m-%dT%H:%M:%S%.3fZ").to_string()
}

/// Milliseconds since the epoch, `Date.now()`.
pub fn now_ms() -> i64 {
    jiff::Timestamp::now().as_millisecond()
}

pub fn log(message: &str) {
    write_line(&format!("{} {message}\n", iso_now()));
}

/// `log(message, detail)` with a detail already formatted (an error's `Name: message`, a string,
/// or JSON).
pub fn log_detail(message: &str, detail: &str) {
    write_line(&format!("{} {message} {detail}\n", iso_now()));
}

fn write_line(line: &str) {
    let dir = paths::state_dir();
    if paths::ensure_private_dir(&dir).is_ok() {
        let path = paths::log_path();
        if fs::metadata(&path).is_ok_and(|meta| meta.len() > MAX_BYTES) {
            let mut rotated = path.clone().into_os_string();
            rotated.push(".1");
            let _ = fs::rename(&path, rotated);
        }
        if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
            let _ = file.write_all(line.as_bytes());
        }
    }
    if std::env::var_os("VEXDOM_DEBUG").is_some_and(|value| !value.is_empty()) {
        let _ = std::io::stderr().write_all(line.as_bytes());
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn iso_matches_js() {
        let at: jiff::Timestamp = "2026-11-25T08:09:10.5Z".parse().unwrap();
        assert_eq!(super::iso(at), "2026-11-25T08:09:10.500Z");
    }
}
