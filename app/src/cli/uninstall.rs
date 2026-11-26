//! What removing Vexdom means on this machine, resolved rather than assumed (`uninstall.ts`): an
//! install can have been pointed elsewhere with `setup --dir`, and skills, screenshots and downloads
//! each take a config override.

use std::path::{Path, PathBuf};

use vexdom_core::{config, downloads, paths};

#[derive(Debug, Clone)]
pub struct Removal {
    pub label: &'static str,
    pub path: PathBuf,
    pub holds: &'static str,
    /// Entries under `path` to leave behind.
    pub keep: Vec<String>,
}

pub struct UninstallPlan {
    pub removals: Vec<Removal>,
    /// Configured directories outside the two roots. Named, never deleted: the user put them there.
    pub elsewhere: Vec<Removal>,
}

fn removal(label: &'static str, path: PathBuf, holds: &'static str) -> Removal {
    Removal {
        label,
        path,
        holds,
        keep: Vec::new(),
    }
}

pub fn plan_uninstall(keep_skills: bool) -> UninstallPlan {
    let agent = config::read_agent_config();
    let extension = paths::extension_dir(agent.extension_dir().map(PathBuf::from).as_deref());
    let (state, user) = (paths::state_dir(), paths::user_dir());
    let inside_roots = |path: &Path| path.starts_with(&state) || path.starts_with(&user);
    let keep = if keep_skills {
        vec!["skills".to_owned()]
    } else {
        Vec::new()
    };

    let mut removals = Vec::new();
    // Only a folder `setup` wrote (it holds the install stamp) is Vexdom's to remove.
    if !inside_roots(&extension) && super::install::read_stamp(&extension).is_some() {
        removals.push(removal(
            "extension",
            extension,
            "the unpacked build Chrome loads",
        ));
    }
    removals.push(Removal {
        keep: keep.clone(),
        ..removal(
            "state",
            state.clone(),
            "pairing keys, config, approvals, logs",
        )
    });
    removals.push(Removal {
        keep,
        ..removal(
            "files",
            user.clone(),
            "the extension, skills, site maps, screenshots, captured downloads",
        )
    });

    let outside = [
        (
            "skills",
            downloads::uploaded_skills_dir(),
            "site maps and uploaded skills",
        ),
        (
            "screenshots",
            downloads::screenshot_dir(),
            "captures taken with save: true",
        ),
        (
            "downloads",
            downloads::download_dir(),
            "files captured from pages",
        ),
    ];
    let elsewhere = outside
        .into_iter()
        .filter(|(_, path, _)| !inside_roots(path) && path.exists())
        .map(|(label, path, holds)| removal(label, path, holds))
        .collect();
    UninstallPlan {
        removals: removals
            .into_iter()
            .filter(|removal| removal.path.exists())
            .collect(),
        elsewhere,
    }
}

pub struct RemovalOutcome {
    pub removal: Removal,
    pub removed: bool,
    /// What was left behind, so the report can say "emptied" rather than claim a clean sweep.
    pub kept: Vec<String>,
    pub error: Option<String>,
}

/// A scanner (Spotlight, an antivirus) can hold a file it has just seen for a moment, so a delete is
/// tried again.
fn remove_with_retry(path: &Path) -> std::io::Result<()> {
    let mut attempt = 0;
    loop {
        let removed = if path.is_dir() && !path.is_symlink() {
            std::fs::remove_dir_all(path)
        } else {
            std::fs::remove_file(path)
        };
        match removed {
            Ok(()) => return Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) if attempt >= 3 => return Err(error),
            Err(_) => {
                attempt += 1;
                std::thread::sleep(std::time::Duration::from_millis(200));
            }
        }
    }
}

/// A folder reached through a link is somewhere the user chose, so only the link goes. One entry
/// that will not go leaves the rest to be deleted anyway: the pairing keys among them.
pub fn remove_all(removals: Vec<Removal>) -> Vec<RemovalOutcome> {
    removals
        .into_iter()
        .map(|removal| {
            if removal.path.is_symlink() {
                let result = std::fs::remove_file(&removal.path);
                return RemovalOutcome {
                    removed: result.is_ok(),
                    kept: Vec::new(),
                    error: result.err().map(|e| e.to_string()),
                    removal,
                };
            }
            let spare: Vec<&String> = removal.keep.iter().collect();
            let entries = match std::fs::read_dir(&removal.path) {
                Ok(entries) => entries,
                Err(error) => {
                    return RemovalOutcome {
                        removed: false,
                        kept: Vec::new(),
                        error: Some(error.to_string()),
                        removal,
                    };
                }
            };
            let mut kept = Vec::new();
            let mut refused = Vec::new();
            for entry in entries.filter_map(Result::ok) {
                let name = entry.file_name().to_string_lossy().into_owned();
                if spare.contains(&&name) {
                    kept.push(name);
                    continue;
                }
                if let Err(error) = remove_with_retry(&entry.path()) {
                    refused.push(error.to_string());
                }
            }
            if !refused.is_empty() {
                return RemovalOutcome {
                    removed: false,
                    kept,
                    error: Some(refused.join("; ")),
                    removal,
                };
            }
            if kept.is_empty()
                && let Err(error) = remove_with_retry(&removal.path)
            {
                return RemovalOutcome {
                    removed: false,
                    kept,
                    error: Some(error.to_string()),
                    removal,
                };
            }
            RemovalOutcome {
                removed: true,
                kept,
                error: None,
                removal,
            }
        })
        .collect()
}
