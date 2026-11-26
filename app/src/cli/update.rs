//! `vexdom update` (ADR-031): the latest signed release, installed over the copy that is running —
//! a release tarball's `bin/` and `extension/`, or `Vexdom.app` — then the Bridge restarted on it.
//! `--local` keeps the old behaviour: re-run setup with this binary.

use std::process::ExitCode;

use serde_json::json;
use vexdom_update::{Layout, Release};

use super::{Args, fail, setup};

pub(super) fn run(args: &Args) -> ExitCode {
    if args.flag("--local") {
        let mut rest = vec!["--no-pair".to_owned(), "--restart".to_owned()];
        rest.extend(
            args.all[1..]
                .iter()
                .filter(|arg| *arg != "--local")
                .cloned(),
        );
        return setup::setup(&rest);
    }
    let current = env!("CARGO_PKG_VERSION");
    let release = match vexdom_client::block_on(vexdom_update::latest()) {
        Ok(release) => release,
        Err(message) => return fail(&message),
    };
    if !vexdom_update::newer(&release.version, current) {
        if args.json() {
            println!(
                "{}",
                json!({"from": current, "to": current, "updated": false})
            );
        } else {
            println!("vexdom {current} is the latest");
        }
        return ExitCode::SUCCESS;
    }
    if args.flag("--check") {
        if args.json() {
            println!(
                "{}",
                json!({"from": current, "to": release.version, "available": true})
            );
        } else {
            println!(
                "vexdom {} is available (you have {current}); run vexdom update",
                release.version
            );
        }
        return ExitCode::SUCCESS;
    }
    let exe = match std::env::current_exe().and_then(std::fs::canonicalize) {
        Ok(exe) => exe,
        Err(error) => return fail(&format!("Cannot find this binary: {error}")),
    };
    let layout = vexdom_update::detect_layout(&exe);
    let installed = match &layout {
        Layout::Unmanaged(path) => Err(format!(
            "UPDATE_UNMANAGED: {} was not installed from a release; reinstall with install.sh or rebuild it",
            path.display()
        )),
        Layout::Tarball { root } => update_tarball(&release, root),
        Layout::App { bundle } => update_app(&release, bundle),
    };
    let (new_exe, waits_for_quit) = match installed {
        Ok(installed) => installed,
        Err(message) => return fail(&message),
    };
    // A swap that waits for the app to quit restarts the Bridge when the app reopens.
    let restart = !args.flag("--no-restart") && !waits_for_quit;
    let restarted = restart
        && std::process::Command::new(&new_exe)
            .args(["setup", "--no-pair", "--restart", "--no-open", "--no-wait"])
            .status()
            .is_ok_and(|status| status.success());
    let kind = match layout {
        Layout::Tarball { .. } => "tarball",
        Layout::App { .. } => "app",
        Layout::Unmanaged(_) => "unmanaged",
    };
    if args.json() {
        println!(
            "{}",
            json!({"from": current, "to": release.version, "layout": kind, "restarted": restarted})
        );
    } else {
        println!("Updated vexdom {current} → {}.", release.version);
        if restart && !restarted {
            println!("Restart the Bridge with: vexdom update --local");
        }
    }
    ExitCode::SUCCESS
}

/// A scratch folder for the download, removed when dropped.
struct Scratch(std::path::PathBuf);

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A fresh folder only this user can enter, so nobody swaps the download between its check and
/// its install.
fn scratch() -> Result<Scratch, String> {
    let dir = std::env::temp_dir().join(format!(
        "vexdom-update-{}-{}",
        std::process::id(),
        vexdom_core::rand::hex(8)
    ));
    let mut builder = std::fs::DirBuilder::new();
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
    builder
        .create(&dir)
        .map_err(|error| format!("Could not make {}: {error}", dir.display()))?;
    Ok(Scratch(dir))
}

/// The new binary, and whether it waits for the desktop app to quit before it is in place.
type Installed = (std::path::PathBuf, bool);

fn update_tarball(release: &Release, root: &std::path::Path) -> Result<Installed, String> {
    let asset = format!(
        "vexdom-{}-{}.tar.gz",
        release.version,
        vexdom_update::target_triple()
    );
    let dir = scratch()?;
    println!("Downloading {asset}");
    let archive = vexdom_client::block_on(vexdom_update::fetch_verified(release, &asset, &dir.0))?;
    vexdom_update::install_tarball(&archive, root)?;
    Ok((root.join("bin/vexdom"), false))
}

fn update_app(release: &Release, bundle: &std::path::Path) -> Result<Installed, String> {
    let asset = format!(
        "Vexdom-{}-{}.zip",
        release.version,
        vexdom_update::target_triple()
    );
    let dir = scratch()?;
    println!("Downloading {asset}");
    let zip = vexdom_client::block_on(vexdom_update::fetch_verified(release, &asset, &dir.0))?;
    let staged = vexdom_update::stage_app(&zip, bundle, &release.version)?;
    match running_desktop_app() {
        // The app swaps itself in once it quits; the CLI inside it keeps working until then.
        Some(pid) => {
            vexdom_update::swap_after_exit(pid, &staged, bundle)?;
            println!("Quit Vexdom to finish the update; it reopens on the new version.");
            Ok((bundle.join("Contents/MacOS/vexdom"), true))
        }
        None => {
            vexdom_update::swap_app(&staged, bundle)?;
            Ok((bundle.join("Contents/MacOS/vexdom"), false))
        }
    }
}

/// The running desktop app's pid, if it is open.
fn running_desktop_app() -> Option<u32> {
    let output = std::process::Command::new("pgrep")
        .args(["-f", "Vexdom.app/Contents/MacOS/vexdom-desktop"])
        .output()
        .ok()?;
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .next()?
        .trim()
        .parse()
        .ok()
}
