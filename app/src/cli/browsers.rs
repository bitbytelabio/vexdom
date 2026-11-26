//! The browsers setup offers, where each one gets the extension, how to find it on this computer,
//! and how to open a page in it (`browsers.ts`, `firefox-addon.ts`).

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use serde::Serialize;
use serde_json::Value;
use vexdom_core::pairing::source_label;
use vexdom_core::paths;
use vexdom_daemon::native_host::Platform;

use vexdom_client::Bridge;

pub const BROWSER_IDS: [&str; 8] = [
    "chrome", "edge", "brave", "arc", "vivaldi", "opera", "chromium", "firefox",
];

pub struct BrowserEntry {
    pub id: &'static str,
    pub label: &'static str,
    /// How the extension names this browser when it connects: its user-agent brand.
    brand: Brand,
    pub extensions_page: &'static str,
    mac: Option<&'static str>,
    linux: &'static [&'static str],
}

enum Brand {
    Contains(&'static str),
    Exactly(&'static str),
}

impl Brand {
    fn matches(&self, brand: &str) -> bool {
        let brand = brand.to_lowercase();
        match self {
            Self::Contains(word) => brand.contains(word),
            Self::Exactly(word) => brand == *word,
        }
    }
}

pub const BROWSERS: [BrowserEntry; 8] = [
    BrowserEntry {
        id: "chrome",
        label: "Chrome",
        brand: Brand::Contains("google chrome"),
        extensions_page: "chrome://extensions",
        mac: Some("Google Chrome.app"),
        linux: &["google-chrome", "google-chrome-stable"],
    },
    BrowserEntry {
        id: "edge",
        label: "Edge",
        brand: Brand::Contains("edge"),
        extensions_page: "edge://extensions",
        mac: Some("Microsoft Edge.app"),
        linux: &["microsoft-edge", "microsoft-edge-stable"],
    },
    BrowserEntry {
        id: "brave",
        label: "Brave",
        brand: Brand::Contains("brave"),
        extensions_page: "brave://extensions",
        mac: Some("Brave Browser.app"),
        linux: &["brave-browser", "brave"],
    },
    BrowserEntry {
        id: "arc",
        label: "Arc",
        brand: Brand::Exactly("arc"),
        extensions_page: "arc://extensions",
        mac: Some("Arc.app"),
        linux: &[],
    },
    BrowserEntry {
        id: "vivaldi",
        label: "Vivaldi",
        brand: Brand::Contains("vivaldi"),
        extensions_page: "vivaldi://extensions",
        mac: Some("Vivaldi.app"),
        linux: &["vivaldi", "vivaldi-stable"],
    },
    BrowserEntry {
        id: "opera",
        label: "Opera",
        brand: Brand::Contains("opera"),
        extensions_page: "opera://extensions",
        mac: Some("Opera.app"),
        linux: &["opera"],
    },
    BrowserEntry {
        id: "chromium",
        label: "Chromium",
        brand: Brand::Exactly("chromium"),
        extensions_page: "chrome://extensions",
        mac: Some("Chromium.app"),
        linux: &["chromium", "chromium-browser"],
    },
    BrowserEntry {
        id: "firefox",
        label: "Firefox",
        brand: Brand::Contains("firefox"),
        extensions_page: "about:addons",
        mac: Some("Firefox.app"),
        linux: &["firefox"],
    },
];

pub fn entry(id: &str) -> &'static BrowserEntry {
    BROWSERS
        .iter()
        .find(|entry| entry.id == id)
        .unwrap_or(&BROWSERS[0])
}

/// Which browser a connected extension is in, from the brand it reported.
pub fn browser_of(brand: &str) -> Option<&'static str> {
    if brand.is_empty() {
        return None;
    }
    BROWSERS
        .iter()
        .find(|entry| entry.brand.matches(brand))
        .map(|entry| entry.id)
}

/// There is no store listing: Firefox takes the signed add-on, every Chromium browser an unpacked copy.
pub fn source_for(id: &str) -> &'static str {
    if id == "firefox" {
        "firefox"
    } else {
        "unpacked"
    }
}

fn releases() -> String {
    std::env::var("VEXDOM_RELEASES")
        .unwrap_or_else(|_| "https://github.com/nguyenthdat/vexdom/releases".into())
}

pub fn releases_page() -> String {
    format!("{}/latest", releases())
}

/// The add-on addons.mozilla.org signed for this same version, hung under its GitHub release.
pub fn signed_addon_url(version: &str) -> String {
    format!(
        "{}/download/v{version}/vexdom-{version}-firefox.xpi",
        releases()
    )
}

/// Whether the release carries the signed file yet, or `None` when GitHub is unreachable. Mozilla
/// signs after the release exists, so for a few minutes after a tag the link is right but the file
/// is not there.
pub async fn signed_addon_attached(version: &str) -> Option<bool> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(4))
        .build()
        .ok()?;
    let response = client.head(signed_addon_url(version)).send().await.ok()?;
    Some(response.status().is_success())
}

/// A paired browser as the CLI shows it (`SessionSummary`).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Session {
    pub id: String,
    pub origin: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub browser: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub label: String,
    pub source: String,
    pub extension_version: String,
    pub paired_at: String,
    pub paired_by: String,
    pub last_seen_at: String,
    pub connected: bool,
}

pub async fn sessions(bridge: &Bridge) -> Result<Vec<Session>, String> {
    let reply = bridge
        .control
        .sessions_request()
        .send()
        .promise
        .await
        .map_err(|error| error.to_string())?;
    let list = reply
        .get()
        .and_then(|reply| reply.get_sessions())
        .map_err(|error| error.to_string())?;
    let text = |reader: capnp::Result<capnp::text::Reader<'_>>| vexdom_daemon::wire::text(reader);
    Ok(list
        .iter()
        .map(|session| {
            let source = text(session.get_source());
            Session {
                id: text(session.get_id()),
                origin: text(session.get_origin()),
                browser: text(session.get_browser()),
                label: text(session.get_label()),
                source: if source.is_empty() {
                    "unpacked".into()
                } else {
                    source
                },
                extension_version: text(session.get_extension_version()),
                paired_at: text(session.get_paired_at()),
                paired_by: text(session.get_paired_by()),
                last_seen_at: text(session.get_last_seen_at()),
                connected: session.get_connected(),
            }
        })
        .collect())
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RowSession {
    pub id: String,
    pub source: String,
    pub extension_version: String,
    pub connected: bool,
}

/// One browser as setup, `vexdom browsers` and the apps show it.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserRow {
    pub id: &'static str,
    pub label: &'static str,
    pub installed: bool,
    pub source: &'static str,
    pub store: &'static str,
    pub store_url: Option<String>,
    pub steps: Vec<&'static str>,
    pub extensions_page: &'static str,
    pub connected: bool,
    pub sessions: Vec<RowSession>,
}

pub fn browser_rows(sessions: &[Session], located: &[Located], version: &str) -> Vec<BrowserRow> {
    BROWSERS
        .iter()
        .map(|entry| {
            let own: Vec<&Session> = sessions.iter().filter(|session| browser_of(&session.browser) == Some(entry.id)).collect();
            let source = source_for(entry.id);
            BrowserRow {
                id: entry.id,
                label: entry.label,
                installed: located.iter().any(|found| found.id == entry.id),
                source,
                store: source_label(source),
                store_url: (source == "firefox").then(|| signed_addon_url(version)),
                steps: if entry.id == "firefox" {
                    vec!["Firefox asks whether to let github.com install software, then whether to add Vexdom. Say yes to both."]
                } else {
                    Vec::new()
                },
                extensions_page: entry.extensions_page,
                connected: own.iter().any(|session| session.connected),
                sessions: own
                    .iter()
                    .map(|session| RowSession {
                        id: session.id.clone(),
                        source: session.source.clone(),
                        extension_version: session.extension_version.clone(),
                        connected: session.connected,
                    })
                    .collect(),
            }
        })
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Located {
    pub id: &'static str,
    /// The app bundle, program or command that opens it.
    pub path: PathBuf,
}

pub fn locate_browsers() -> Vec<Located> {
    let platform = Platform::current();
    let home = paths::home_dir();
    BROWSERS
        .iter()
        .filter_map(|entry| {
            locate(entry, platform, &home).map(|path| Located { id: entry.id, path })
        })
        .collect()
}

fn locate(entry: &BrowserEntry, platform: Platform, home: &Path) -> Option<PathBuf> {
    match platform {
        Platform::Mac => {
            let app = entry.mac?;
            [
                PathBuf::from("/Applications").join(app),
                home.join("Applications").join(app),
            ]
            .into_iter()
            .find(|path| path.exists())
        }
        Platform::Linux => {
            let dirs = std::env::var_os("PATH")
                .map(|path| std::env::split_paths(&path).collect::<Vec<_>>())
                .unwrap_or_default();
            entry
                .linux
                .iter()
                .flat_map(|bin| dirs.iter().map(move |dir| dir.join(bin)))
                .find(|path| path.exists())
        }
    }
}

/// The command that opens `url`, in that browser or the default one.
pub fn launch(url: &str, browser: Option<&Located>, platform: Platform) -> (String, Vec<String>) {
    match (platform, browser) {
        (Platform::Mac, Some(found)) => (
            "open".into(),
            vec![
                "-a".into(),
                found.path.to_string_lossy().into_owned(),
                url.into(),
            ],
        ),
        (Platform::Mac, None) => ("open".into(), vec![url.into()]),
        (_, Some(found)) => (found.path.to_string_lossy().into_owned(), vec![url.into()]),
        (Platform::Linux, None) => ("xdg-open".into(), vec![url.into()]),
    }
}

/// Opens the page, never waiting on it.
pub fn open_page(url: &str, browser: Option<&Located>) {
    let (file, args) = launch(url, browser, Platform::current());
    let mut command = Command::new(file);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(&mut command, 0);
    let _ = command.spawn();
}

/// `{ running, browsers, unpacked }` for `vexdom browsers --json`.
pub fn browsers_json(
    running: bool,
    rows: &[BrowserRow],
    dir: &Path,
    version: Option<String>,
) -> Value {
    serde_json::json!({
        "running": running,
        "browsers": rows,
        "unpacked": {"dir": dir.to_string_lossy(), "version": version},
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn brands_name_their_browser() {
        assert_eq!(browser_of("Google Chrome"), Some("chrome"));
        assert_eq!(browser_of("Microsoft Edge"), Some("edge"));
        assert_eq!(browser_of("Arc"), Some("arc"));
        assert_eq!(browser_of("Chromium"), Some("chromium"));
        assert_eq!(browser_of("Firefox"), Some("firefox"));
        assert_eq!(browser_of(""), None);
        assert_eq!(browser_of("Netscape"), None);
    }

    #[test]
    fn opens_pages_like_the_platform() {
        let arc = Located {
            id: "arc",
            path: PathBuf::from("/Applications/Arc.app"),
        };
        assert_eq!(
            launch("u", Some(&arc), Platform::Mac),
            (
                "open".into(),
                vec!["-a".into(), "/Applications/Arc.app".into(), "u".into()]
            )
        );
        assert_eq!(launch("u", None, Platform::Linux).0, "xdg-open");
    }
}
