//! Lets the browser reach Vexdom Bridge (`native-host.ts`). Chrome, Edge, Brave, Firefox and their
//! forks each launch a program registered as a native messaging host when the extension asks for it.
//! The registration lists the exact extension origins allowed to launch it: the unpacked folder and
//! whatever has paired, recomputed on every setup, every pairing and every Bridge start.
//!
//! A browser does not always read its own folder: Brave and Opera read Chrome's on macOS. So
//! Chrome's and Mozilla's locations are written whether or not those browsers are installed, and
//! each installed browser's own on top.

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use vexdom_core::proto::{FIREFOX_ADDON_ID, NATIVE_HOST_NAME};
use vexdom_core::{auth_store, config, log, paths};

use crate::quarantine;

/// macOS first; Linux is kept building. Windows is not supported (ADR-026).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    Mac,
    Linux,
}

impl Platform {
    pub fn current() -> Self {
        if cfg!(target_os = "macos") {
            Self::Mac
        } else {
            Self::Linux
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Family {
    Chromium,
    Firefox,
}

impl Family {
    /// A file the browser itself writes into its profile folder, which a baseline manifest does not make.
    fn profile_mark(self) -> &'static str {
        match self {
            Self::Chromium => "Local State",
            Self::Firefox => "profiles.ini",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostTarget {
    pub browser: String,
    pub family: Family,
    /// Any of these present means the browser is installed.
    pub installs: Vec<PathBuf>,
    /// Every folder this browser reads a manifest from, which is not always its own.
    pub manifest_dirs: Vec<PathBuf>,
    /// Written whether or not this browser is installed, because other browsers read it too.
    pub baseline: bool,
}

const CHROME: &str = "Google/Chrome";
const MOZILLA: &str = "Mozilla";

struct MacBrowser {
    browser: &'static str,
    family: Family,
    profile: &'static str,
    apps: &'static [&'static str],
    reads: &'static [&'static str],
    baseline: bool,
}

const fn mac(
    browser: &'static str,
    profile: &'static str,
    apps: &'static [&'static str],
    reads: &'static [&'static str],
) -> MacBrowser {
    MacBrowser {
        browser,
        family: Family::Chromium,
        profile,
        apps,
        reads,
        baseline: false,
    }
}

const fn mac_firefox(
    browser: &'static str,
    profile: &'static str,
    apps: &'static [&'static str],
    reads: &'static [&'static str],
) -> MacBrowser {
    MacBrowser {
        browser,
        family: Family::Firefox,
        profile,
        apps,
        reads,
        baseline: false,
    }
}

const MAC_BROWSERS: [MacBrowser; 24] = [
    MacBrowser {
        baseline: true,
        ..mac("Chrome", CHROME, &["Google Chrome.app"], &[CHROME])
    },
    mac(
        "Chrome Beta",
        "Google/Chrome Beta",
        &["Google Chrome Beta.app"],
        &["Google/Chrome Beta"],
    ),
    mac(
        "Chrome Dev",
        "Google/Chrome Dev",
        &["Google Chrome Dev.app"],
        &["Google/Chrome Dev"],
    ),
    mac(
        "Chrome Canary",
        "Google/Chrome Canary",
        &["Google Chrome Canary.app"],
        &["Google/Chrome Canary"],
    ),
    mac(
        "Chrome for Testing",
        "Google/Chrome for Testing",
        &["Google Chrome for Testing.app"],
        &["Google/Chrome for Testing"],
    ),
    mac("Chromium", "Chromium", &["Chromium.app"], &["Chromium"]),
    mac(
        "Edge",
        "Microsoft Edge",
        &["Microsoft Edge.app"],
        &["Microsoft Edge"],
    ),
    mac(
        "Edge Beta",
        "Microsoft Edge Beta",
        &["Microsoft Edge Beta.app"],
        &["Microsoft Edge Beta"],
    ),
    mac(
        "Edge Dev",
        "Microsoft Edge Dev",
        &["Microsoft Edge Dev.app"],
        &["Microsoft Edge Dev"],
    ),
    mac(
        "Edge Canary",
        "Microsoft Edge Canary",
        &["Microsoft Edge Canary.app"],
        &["Microsoft Edge Canary"],
    ),
    // brave-core points both host folders at Chrome's (brave_main_delegate.cc); its own is kept for when it stops.
    mac(
        "Brave",
        "BraveSoftware/Brave-Browser",
        &["Brave Browser.app"],
        &[CHROME, "BraveSoftware/Brave-Browser"],
    ),
    mac(
        "Brave Beta",
        "BraveSoftware/Brave-Browser-Beta",
        &["Brave Browser Beta.app"],
        &[CHROME, "BraveSoftware/Brave-Browser-Beta"],
    ),
    mac(
        "Brave Nightly",
        "BraveSoftware/Brave-Browser-Nightly",
        &["Brave Browser Nightly.app"],
        &[CHROME, "BraveSoftware/Brave-Browser-Nightly"],
    ),
    mac("Vivaldi", "Vivaldi", &["Vivaldi.app"], &["Vivaldi"]),
    mac(
        "Arc",
        "Arc/User Data",
        &["Arc.app"],
        &["Arc/User Data", CHROME],
    ),
    mac(
        "Opera",
        "com.operasoftware.Opera",
        &["Opera.app"],
        &[CHROME],
    ),
    mac(
        "Opera GX",
        "com.operasoftware.OperaGX",
        &["Opera GX.app"],
        &[CHROME],
    ),
    mac("Comet", "Comet", &["Comet.app"], &[CHROME]),
    mac("Thorium", "Thorium", &["Thorium.app"], &["Thorium"]),
    MacBrowser {
        baseline: true,
        ..mac_firefox(
            "Firefox",
            "Firefox",
            &[
                "Firefox.app",
                "Firefox Developer Edition.app",
                "Firefox Nightly.app",
            ],
            &[MOZILLA],
        )
    },
    mac_firefox("Zen", "zen", &["Zen.app", "Zen Browser.app"], &[MOZILLA]),
    mac_firefox("LibreWolf", "librewolf", &["LibreWolf.app"], &[MOZILLA]),
    mac_firefox("Floorp", "Floorp", &["Floorp.app"], &[MOZILLA]),
    // Waterfox reads Mozilla's folder only from 6.7.6 on.
    mac_firefox(
        "Waterfox",
        "Waterfox",
        &["Waterfox.app"],
        &["Waterfox", MOZILLA],
    ),
];

const LINUX_BROWSERS: [(&str, Family, &str, &str); 6] = [
    (
        "Chrome",
        Family::Chromium,
        ".config/google-chrome",
        ".config/google-chrome/NativeMessagingHosts",
    ),
    (
        "Chromium",
        Family::Chromium,
        ".config/chromium",
        ".config/chromium/NativeMessagingHosts",
    ),
    (
        "Edge",
        Family::Chromium,
        ".config/microsoft-edge",
        ".config/microsoft-edge/NativeMessagingHosts",
    ),
    (
        "Brave",
        Family::Chromium,
        ".config/BraveSoftware/Brave-Browser",
        ".config/BraveSoftware/Brave-Browser/NativeMessagingHosts",
    ),
    (
        "Vivaldi",
        Family::Chromium,
        ".config/vivaldi",
        ".config/vivaldi/NativeMessagingHosts",
    ),
    (
        "Firefox",
        Family::Firefox,
        ".mozilla",
        ".mozilla/native-messaging-hosts",
    ),
];

/// A macOS app that names its own Chromium user-data folder in Info.plist, as forks do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChromiumFork {
    pub browser: String,
    pub app: PathBuf,
    pub data_dir: String,
}

fn mac_apps(home: &Path) -> [PathBuf; 2] {
    [PathBuf::from("/Applications"), home.join("Applications")]
}

pub fn host_targets(platform: Platform, home: &Path, forks: &[ChromiumFork]) -> Vec<HostTarget> {
    match platform {
        Platform::Mac => {
            let support = home.join("Library").join("Application Support");
            let hosts_in = |dir: &str| support.join(dir).join("NativeMessagingHosts");
            let mut targets: Vec<HostTarget> = MAC_BROWSERS
                .iter()
                .map(|entry| {
                    let mut installs = vec![
                        support
                            .join(entry.profile)
                            .join(entry.family.profile_mark()),
                    ];
                    for app in entry.apps {
                        installs.extend(mac_apps(home).iter().map(|root| root.join(app)));
                    }
                    HostTarget {
                        browser: entry.browser.into(),
                        family: entry.family,
                        installs,
                        manifest_dirs: entry.reads.iter().map(|dir| hosts_in(dir)).collect(),
                        baseline: entry.baseline,
                    }
                })
                .collect();
            let covered: Vec<&str> = MAC_BROWSERS
                .iter()
                .flat_map(|entry| entry.apps.iter().copied())
                .collect();
            for fork in forks {
                let name = fork
                    .app
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("");
                if covered.contains(&name) {
                    continue;
                }
                targets.push(HostTarget {
                    browser: fork.browser.clone(),
                    family: Family::Chromium,
                    installs: vec![fork.app.clone()],
                    manifest_dirs: vec![hosts_in(&fork.data_dir)],
                    baseline: false,
                });
            }
            targets
        }
        Platform::Linux => LINUX_BROWSERS
            .iter()
            .map(|(browser, family, dir, manifests)| HostTarget {
                browser: (*browser).into(),
                family: *family,
                installs: vec![home.join(dir)],
                manifest_dirs: vec![home.join(manifests)],
                baseline: false,
            })
            .collect(),
    }
}

/// Chromium derives a fork's host folder from CrProductDirName (chrome_paths_mac.mm), so a fork no
/// table names still gets one.
pub fn chromium_forks(home: &Path) -> Vec<ChromiumFork> {
    let mut forks = Vec::new();
    for root in mac_apps(home) {
        let Ok(entries) = std::fs::read_dir(&root) else {
            continue;
        };
        let mut names: Vec<String> = entries
            .filter_map(Result::ok)
            .filter_map(|entry| entry.file_name().into_string().ok())
            .filter(|name| name.ends_with(".app"))
            .collect();
        names.sort();
        for name in names {
            let app = root.join(&name);
            if let Some(data_dir) = product_dir(&app.join("Contents").join("Info.plist")) {
                forks.push(ChromiumFork {
                    browser: name.trim_end_matches(".app").into(),
                    app,
                    data_dir,
                });
            }
        }
    }
    forks
}

fn product_dir(plist: &Path) -> Option<String> {
    let raw = std::fs::read(plist).ok()?;
    let marker = b"CrProductDirName";
    if !raw.windows(marker.len()).any(|window| window == marker) {
        return None;
    }
    let named = if raw.starts_with(b"bplist") {
        let out = Command::new("plutil")
            .args(["-extract", "CrProductDirName", "raw", "-o", "-"])
            .arg(plist)
            .stderr(Stdio::null())
            .output()
            .ok()
            .filter(|out| out.status.success())?;
        String::from_utf8(out.stdout).ok()?
    } else {
        let text = String::from_utf8_lossy(&raw);
        let pattern =
            regex::Regex::new(r"<key>CrProductDirName</key>\s*<string>([^<]+)</string>").ok()?;
        pattern.captures(&text)?.get(1)?.as_str().to_owned()
    };
    let dir = named.trim();
    (!dir.is_empty() && !dir.starts_with('/') && !dir.split('/').any(|part| part == ".."))
        .then(|| dir.to_owned())
}

/// Chrome names an unpacked extension after its folder: the SHA-256 of the path, spelled in a–p.
pub fn unpacked_extension_origin(dir: &str) -> String {
    let id: String = Sha256::digest(dir.as_bytes())
        .iter()
        .take(16)
        .flat_map(|byte| [byte >> 4, byte & 0x0f])
        .map(|nibble| char::from(b'a' + nibble))
        .collect();
    format!("chrome-extension://{id}/")
}

fn as_origin(origin: &str) -> String {
    if origin.ends_with('/') {
        origin.to_owned()
    } else {
        format!("{origin}/")
    }
}

fn configured_extension_dir() -> PathBuf {
    let config = config::read_agent_config();
    paths::extension_dir(config.extension_dir().map(PathBuf::from).as_deref())
}

pub fn allowed_origins() -> Vec<String> {
    let unpacked = unpacked_extension_origin(&configured_extension_dir().to_string_lossy());
    let mut set: BTreeSet<String> = BTreeSet::new();
    set.insert(as_origin(&unpacked));
    for session in auth_store::list_sessions() {
        if session.origin.starts_with("chrome-extension://") {
            set.insert(as_origin(&session.origin));
        }
    }
    set.into_iter().collect()
}

pub fn host_manifest(family: Family, launcher: &str, origins: &[String]) -> Value {
    let mut manifest = json!({
        "name": NATIVE_HOST_NAME,
        "description": "Starts and connects to Vexdom Bridge when the browser needs it",
        "path": launcher,
        "type": "stdio",
    });
    match family {
        Family::Firefox => manifest["allowed_extensions"] = json!([FIREFOX_ADDON_ID]),
        Family::Chromium => manifest["allowed_origins"] = json!(origins),
    }
    manifest
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LauncherPins {
    /// The `vexdom` binary the launcher runs.
    pub exe: String,
    pub path_env: String,
}

/// A host must write nothing but its framed reply: Firefox up to 157 on macOS can wait forever on a
/// host that writes to stderr (bug 1981177).
pub fn launcher_script(pins: &LauncherPins) -> String {
    [
        "#!/bin/sh".to_owned(),
        format!("PATH={}", shell_quote(&pins.path_env)),
        "export PATH".to_owned(),
        format!("BIN={}", shell_quote(&pins.exe)),
        "exec \"$BIN\" native-host \"$@\"".to_owned(),
        String::new(),
    ]
    .join("\n")
}

/// What a launcher was pinned to, read back from the script. `None` for one this binary did not
/// write, such as the Node launcher of a Bridge before ADR-025.
pub fn launcher_pins(script: &str) -> Option<LauncherPins> {
    let quoted = |name: &str| {
        let pattern = regex::Regex::new(&format!(r"(?m)^{name}='((?:[^']|'\\'')*)'$")).ok()?;
        Some(
            pattern
                .captures(script)?
                .get(1)?
                .as_str()
                .replace("'\\''", "'"),
        )
    };
    Some(LauncherPins {
        exe: quoted("BIN")?,
        path_env: quoted("PATH")?,
    })
}

pub fn native_host_dir() -> PathBuf {
    paths::state_dir().join("native-host")
}

pub fn launcher_path() -> PathBuf {
    native_host_dir().join("vexdom-native-host")
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostInstall {
    pub launcher: PathBuf,
    pub browsers: Vec<String>,
    /// Set when nothing was registered because this Bridge's state lives outside the home folder.
    pub skipped: Option<String>,
}

/// The user's browsers read one set of manifests and keys. A Bridge whose state lives outside the
/// home folder, a scratch `VEXDOM_HOME` above all, must not point them at its own launcher, or the
/// user's browsers would wake and pair with a throwaway Bridge.
pub fn registers_hosts_in(state: &str, home: &str, forced: bool) -> bool {
    if forced {
        return true;
    }
    // macOS disks are case-insensitive by default.
    let fold = |value: &str| {
        if cfg!(target_os = "macos") {
            value.to_lowercase()
        } else {
            value.to_owned()
        }
    };
    let home = fold(home);
    let state = fold(state);
    let home = home.trim_end_matches('/');
    state.len() > home.len() + 1 && state.starts_with(home) && state[home.len()..].starts_with('/')
}

pub fn registers_hosts() -> bool {
    registers_hosts_in(
        &paths::state_dir().to_string_lossy(),
        &paths::home_dir().to_string_lossy(),
        std::env::var("VEXDOM_REGISTER_HOST").is_ok_and(|value| value == "1"),
    )
}

fn machine(platform: Platform) -> Vec<HostTarget> {
    let home = paths::home_dir();
    let forks = if platform == Platform::Mac {
        chromium_forks(&home)
    } else {
        Vec::new()
    };
    host_targets(platform, &home, &forks)
}

fn installed(target: &HostTarget) -> bool {
    target.installs.iter().any(|path| path.exists())
}

/// Leaves an unchanged file alone, and never leaves a half-written one where a browser reads it.
pub fn write_if_changed(file: &Path, content: &str) -> std::io::Result<bool> {
    if std::fs::read_to_string(file).is_ok_and(|current| current == content) {
        return Ok(false);
    }
    let temporary = PathBuf::from(format!("{}.{}.tmp", file.display(), std::process::id()));
    std::fs::write(&temporary, content)?;
    std::fs::rename(&temporary, file)?;
    Ok(true)
}

fn register(launcher: &Path, platform: Platform) -> Vec<String> {
    let targets = machine(platform);
    let origins = allowed_origins();
    let launcher = launcher.to_string_lossy();
    let chosen: Vec<&HostTarget> = targets
        .iter()
        .filter(|target| target.baseline || installed(target))
        .collect();
    let mut done: HashMap<String, bool> = HashMap::new();
    let mut write =
        |location: String, family: Family, put: &dyn Fn(&str) -> std::io::Result<()>| {
            if let Some(outcome) = done.get(&location) {
                return *outcome;
            }
            let manifest = format!(
                "{}\n",
                serde_json::to_string_pretty(&host_manifest(family, &launcher, &origins))
                    .unwrap_or_default()
            );
            let outcome = match put(&manifest) {
                Ok(()) => true,
                Err(error) => {
                    log::log(&format!(
                        "native host: could not register at {location}: {error}"
                    ));
                    false
                }
            };
            done.insert(location, outcome);
            outcome
        };
    for target in &chosen {
        for dir in &target.manifest_dirs {
            write(
                dir.to_string_lossy().into_owned(),
                target.family,
                &|manifest| {
                    std::fs::create_dir_all(dir)?;
                    write_if_changed(&dir.join(format!("{NATIVE_HOST_NAME}.json")), manifest)
                        .map(|_| ())
                },
            );
        }
    }
    chosen
        .iter()
        .filter(|target| installed(target))
        .filter(|target| {
            target.manifest_dirs.iter().any(|dir| {
                done.get(dir.to_string_lossy().as_ref())
                    .copied()
                    .unwrap_or(false)
            })
        })
        .map(|target| target.browser.clone())
        .collect()
}

const SKIPPED: &str = "this Bridge keeps its state outside your home folder, so it leaves your browsers registered with the one that does not";

fn write_launcher(pins: &LauncherPins) -> std::io::Result<()> {
    let launcher = launcher_path();
    if write_if_changed(&launcher, &launcher_script(pins))? {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            std::fs::set_permissions(&launcher, std::fs::Permissions::from_mode(0o755))?;
        }
    }
    Ok(())
}

/// `vexdom setup`: writes the launcher pinned to `exe` and registers it with every browser here.
pub fn install_native_host(exe: &Path) -> HostInstall {
    let platform = Platform::current();
    let launcher = launcher_path();
    if !registers_hosts() {
        return HostInstall {
            launcher,
            browsers: Vec::new(),
            skipped: Some(SKIPPED.into()),
        };
    }
    if let Err(error) = paths::ensure_private_dir(&native_host_dir()).and_then(|()| {
        write_launcher(&LauncherPins {
            exe: exe.to_string_lossy().into_owned(),
            path_env: std::env::var("PATH").unwrap_or_default(),
        })
    }) {
        log::log(&format!(
            "native host: could not write the launcher: {error}"
        ));
    }
    let browsers = register(&launcher, platform);
    HostInstall {
        launcher,
        browsers,
        skipped: None,
    }
}

/// Re-registers with what is installed now, letting in whatever has paired since. A launcher whose
/// binary has gone (a moved install), or one an older Bridge wrote for Node, is repinned to `exe`.
pub fn refresh_native_host(exe: &Path) {
    let platform = Platform::current();
    let launcher = launcher_path();
    if !launcher.exists() || !registers_hosts() {
        return;
    }
    let script = std::fs::read_to_string(&launcher).unwrap_or_default();
    let pinned = launcher_pins(&script);
    let current = exe.to_string_lossy().into_owned();
    let healed = match &pinned {
        Some(pins) if Path::new(&pins.exe).exists() || !exe.exists() => pins.clone(),
        Some(pins) => LauncherPins {
            exe: current,
            path_env: pins.path_env.clone(),
        },
        None => LauncherPins {
            exe: current,
            path_env: std::env::var("PATH").unwrap_or_default(),
        },
    };
    if let Err(error) = write_launcher(&healed) {
        log::log(&format!(
            "native host: could not rewrite the launcher: {error}"
        ));
    }
    let repinned = pinned.as_ref().is_none_or(|pins| pins.exe != healed.exe);
    if repinned {
        log::log(&format!(
            "native host: repinned the launcher to {}",
            healed.exe
        ));
    }
    register(&launcher, platform);
}

pub fn remove_native_host() -> Vec<String> {
    if !registers_hosts() {
        return Vec::new();
    }
    let platform = Platform::current();
    let targets = machine(platform);
    let mut removed = Vec::new();
    if platform == Platform::Mac && quarantine::remove_bridge_job() {
        removed.push(format!("the {} launchd job", quarantine::bridge_job()));
    }
    let dirs: BTreeSet<&PathBuf> = targets
        .iter()
        .flat_map(|target| target.manifest_dirs.iter())
        .collect();
    for dir in dirs {
        let manifest = dir.join(format!("{NATIVE_HOST_NAME}.json"));
        if manifest.exists() && std::fs::remove_file(&manifest).is_ok() {
            removed.push(manifest.to_string_lossy().into_owned());
        }
    }
    let dir = native_host_dir();
    if dir.exists() {
        let _ = std::fs::remove_dir_all(&dir);
        removed.push(dir.to_string_lossy().into_owned());
    }
    removed
}

/// The installed browsers that can start the Bridge, read back from where each one looks.
pub fn registered_browsers() -> Vec<String> {
    let platform = Platform::current();
    if !launcher_path().exists() {
        return Vec::new();
    }
    machine(platform)
        .into_iter()
        .filter(installed)
        .filter(|target| {
            target
                .manifest_dirs
                .iter()
                .any(|dir| dir.join(format!("{NATIVE_HOST_NAME}.json")).exists())
        })
        .map(|target| target.browser)
        .collect()
}

/// Whether every manifest this Bridge wrote still lets in the unpacked copy at the configured
/// folder; one written for another folder does not.
pub fn registration_current() -> bool {
    let platform = Platform::current();
    let files: Vec<PathBuf> = machine(platform)
        .iter()
        .filter(|target| target.family == Family::Chromium)
        .flat_map(|target| {
            target
                .manifest_dirs
                .iter()
                .map(|dir| dir.join(format!("{NATIVE_HOST_NAME}.json")))
        })
        .collect();
    let unpacked = unpacked_extension_origin(&configured_extension_dir().to_string_lossy());
    files.iter().filter(|file| file.exists()).all(|file| {
        std::fs::read_to_string(file)
            .ok()
            .and_then(|text| serde_json::from_str::<Value>(&text).ok())
            .and_then(|manifest| {
                manifest
                    .get("allowed_origins")
                    .and_then(Value::as_array)
                    .cloned()
            })
            .is_some_and(|origins| {
                origins
                    .iter()
                    .any(|origin| origin.as_str() == Some(&unpacked))
            })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unpacked_origin_matches_chrome() {
        // Vector computed with the TypeScript implementation (node:crypto).
        assert_eq!(
            unpacked_extension_origin("/Users/me/vexdom/extension/chrome-mv3"),
            "chrome-extension://clgokacphkkcjcaldpadponiabapmohp/"
        );
    }

    #[test]
    fn launcher_round_trips_its_pins() {
        let pins = LauncherPins {
            exe: "/opt/it's here/vexdom".into(),
            path_env: "/usr/bin:/bin".into(),
        };
        let script = launcher_script(&pins);
        assert_eq!(launcher_pins(&script), Some(pins));
        let node = "#!/bin/sh\nPATH='/usr/bin'\nNODE='/usr/local/bin/node'\nCLI='/x/cli.js'\nexec \"$NODE\" \"$CLI\" native-host \"$@\"\n";
        assert_eq!(launcher_pins(node), None);
    }

    #[test]
    fn registers_only_inside_home() {
        assert!(registers_hosts_in("/Users/me/.vexdom", "/Users/me", false));
        assert!(!registers_hosts_in("/tmp/scratch", "/Users/me", false));
        assert!(!registers_hosts_in("/Users/me", "/Users/me", false));
        assert!(!registers_hosts_in(
            "/Users/meow/.vexdom",
            "/Users/me",
            false
        ));
        assert!(registers_hosts_in("/tmp/scratch", "/Users/me", true));
    }

    #[test]
    fn targets_per_platform() {
        let home = Path::new("/Users/me");
        let mac = host_targets(Platform::Mac, home, &[]);
        assert_eq!(mac.len(), 24);
        let brave = mac.iter().find(|target| target.browser == "Brave").unwrap();
        assert_eq!(
            brave.manifest_dirs[0],
            home.join("Library/Application Support/Google/Chrome/NativeMessagingHosts")
        );
        let linux = host_targets(Platform::Linux, Path::new("/home/me"), &[]);
        assert_eq!(linux.len(), 6);
        let manifest = host_manifest(Family::Firefox, "/l", &[]);
        assert_eq!(manifest["allowed_extensions"][0], FIREFOX_ADDON_ID);
    }
}
