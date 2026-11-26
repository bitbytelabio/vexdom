//! The lockfile clients find the Bridge by: `src/daemon/lockfile.ts`.

use std::fs;
use std::io;
use std::path::Path;

use serde::{Deserialize, Serialize};

/// Field order is the TS literal's (`daemon.ts:224-230`), so the file reads the same.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Lockfile {
    pub pid: u32,
    pub port: u16,
    pub token: String,
    #[serde(rename = "protocolVersion")]
    pub protocol_version: u32,
    #[serde(rename = "daemonVersion")]
    pub daemon_version: String,
}

/// Any lockfile that cannot be read or parsed counts as none.
pub fn read(path: &Path) -> Option<Lockfile> {
    serde_json::from_slice(&fs::read(path).ok()?).ok()
}

/// Pretty JSON and a newline, in a 0700 directory, as a 0600 file (`writeLockfile`).
pub fn write(path: &Path, lock: &Lockfile) -> io::Result<()> {
    crate::paths::write_private_json(path, lock)
}

/// `holdWake`: the `stopped` marker, holding an ISO timestamp.
pub fn hold_wake() -> io::Result<()> {
    crate::paths::write_private(
        &crate::paths::stopped_path(),
        format!("{}\n", crate::log::iso_now()).as_bytes(),
    )
}

/// `releaseWake`: a starting daemon clears the marker.
pub fn release_wake() {
    let _ = fs::remove_file(crate::paths::stopped_path());
}

/// `wakeHeld`.
pub fn wake_held() -> bool {
    crate::paths::stopped_path().exists()
}

/// Removes the file only while it still names `pid`, so a newer Bridge's lockfile survives.
pub fn remove_if_ours(path: &Path, pid: u32) {
    if read(path).is_some_and(|lock| lock.pid == pid) {
        let _ = fs::remove_file(path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_the_ts_layout() {
        let dir = std::env::temp_dir().join(format!("vexdom-core-lock-{}", std::process::id()));
        let path = dir.join("state").join("daemon.json");
        let lock = Lockfile {
            pid: 42,
            port: 8785,
            token: "t".into(),
            protocol_version: 26,
            daemon_version: "0.9.2".into(),
        };
        write(&path, &lock).unwrap();
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "{\n  \"pid\": 42,\n  \"port\": 8785,\n  \"token\": \"t\",\n  \"protocolVersion\": 26,\n  \"daemonVersion\": \"0.9.2\"\n}\n"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
            assert_eq!(
                fs::metadata(path.parent().unwrap())
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o700
            );
        }
        assert_eq!(read(&path), Some(lock));
        remove_if_ours(&path, 7);
        assert!(path.exists());
        remove_if_ours(&path, 42);
        assert!(!path.exists());
        let _ = fs::remove_dir_all(dir);
    }
}
