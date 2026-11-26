//! The tool manifest as data (ADR-025): `assets/tools.<target>.json`, written by `bun run tools:emit`
//! from the extension's registry, plus tool naming (`src/lib/actions/{tool-names,reserved}.ts`).
//! Rust never computes the manifest hash; it embeds the one the registry produced.

use std::collections::BTreeMap;
use std::sync::LazyLock;

use serde::Deserialize;
use serde_json::Value;

use crate::proto::RESERVED_PREFIX;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ToolGroup {
    pub index: String,
    pub skills: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    pub hash: String,
    /// `ToolDescriptor[]`: `{name, description, inputSchema}`, in registry order.
    pub tools: Vec<Value>,
    /// Action → group, for grouped actions.
    pub groups: BTreeMap<String, String>,
    /// Group → its guide's index line and skills, in declaration order.
    pub tool_groups: serde_json::Map<String, Value>,
    pub chromium_only: Vec<String>,
}

impl Manifest {
    pub fn tool(&self, action: &str) -> Option<&Value> {
        self.tools
            .iter()
            .find(|tool| tool.get("name").and_then(Value::as_str) == Some(action))
    }

    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.tools
            .iter()
            .filter_map(|tool| tool.get("name").and_then(Value::as_str))
    }

    pub fn group_of(&self, action: &str) -> Option<&str> {
        self.groups.get(action).map(String::as_str)
    }

    /// Group names in declaration order.
    pub fn group_names(&self) -> Vec<&str> {
        self.tool_groups.keys().map(String::as_str).collect()
    }

    pub fn tool_group(&self, group: &str) -> Option<ToolGroup> {
        serde_json::from_value(self.tool_groups.get(group)?.clone()).ok()
    }
}

static CHROMIUM: LazyLock<Manifest> = LazyLock::new(|| {
    serde_json::from_str(include_str!("../assets/tools.chromium.json"))
        .expect("assets/tools.chromium.json is valid")
});
static FIREFOX: LazyLock<Manifest> = LazyLock::new(|| {
    serde_json::from_str(include_str!("../assets/tools.firefox.json"))
        .expect("assets/tools.firefox.json is valid")
});
static BRIDGE: LazyLock<Manifest> = LazyLock::new(|| {
    serde_json::from_str(include_str!("../assets/bridge-actions.json"))
        .expect("assets/bridge-actions.json is valid")
});

/// The bundled Chromium manifest: every action.
pub fn chromium() -> &'static Manifest {
    &CHROMIUM
}

/// The bundled Firefox manifest: no Chromium-only actions.
pub fn firefox() -> &'static Manifest {
    &FIREFOX
}

/// The actions the Bridge answers itself (`bridge.*`), offered next to the extension's.
pub fn bridge() -> &'static Manifest {
    &BRIDGE
}

/// The bundled manifest whose hash is `hash`, if any.
pub fn bundled(hash: &str) -> Option<&'static Manifest> {
    [chromium(), firefox()]
        .into_iter()
        .find(|manifest| manifest.hash == hash)
}

pub const SAVE_SITE_MAP_ACTION: &str = "vexdom.saveSiteMap";
pub const START_RECORDING_ACTION: &str = "vexdom.startRecording";
pub const STOP_RECORDING_ACTION: &str = "vexdom.stopRecording";
pub const READ_SITEMAP_ACTION: &str = "vexdom.readSitemap";
pub const FOCUS_SHOT_ACTION: &str = "vexdom.focusShot";
pub const READ_FILE_ACTION: &str = "vexdom.readFile";
pub const RESERVED_ACTIONS: [&str; 6] = [
    SAVE_SITE_MAP_ACTION,
    START_RECORDING_ACTION,
    STOP_RECORDING_ACTION,
    READ_SITEMAP_ACTION,
    FOCUS_SHOT_ACTION,
    READ_FILE_ACTION,
];

/// Extension actions the Bridge uses itself and never offers or forwards from a control client.
pub const DAEMON_INTERNAL_ACTIONS: [&str; 2] = ["page.readHtml", "page.snapshot"];

/// Actions that only read: `guide` marks them `read-only: yes`.
pub const READ_ONLY_ACTIONS: &[&str] = &[
    "page.getPageInfo",
    "page.snapshot",
    "page.readHtml",
    "page.extractText",
    "page.query",
    "page.readTheme",
    "page.auditContrast",
    "page.readConsole",
    "page.readNetwork",
    "page.monitorStatus",
    "page.timerStatus",
    "page.listSiteTools",
    "page.listFiles",
    "page.listDownloads",
    "page.listRecordings",
    "page.readRecording",
    "page.waitFor",
    "page.waitForElement",
    "page.siteToolResult",
    "page.vitals",
    "page.a11yAudit",
    "bridge.flowSteps",
    "bridge.flowList",
    "bridge.authList",
    "bridge.authShow",
];

/// An action named in full (`page.clickElement`), bare (`clickElement`), or as an MCP client or
/// an older catalog showed it (`mcp__vexdom__page_clickElement`, `page_clickElement`).
pub fn resolve_action(name: &str) -> Option<String> {
    let mut bare = name.trim();
    for prefix in [
        "mcp__vexdom__",
        "vexdom_",
        "vexdom.",
        "page_",
        "page.",
        "bridge.",
        "bridge_",
    ] {
        if let Some(rest) = bare.strip_prefix(prefix) {
            bare = rest;
        }
    }
    if bare.is_empty() {
        return None;
    }
    let page = format!("page.{bare}");
    if chromium().tool(&page).is_some() {
        return Some(page);
    }
    let bridged = format!("bridge.{bare}");
    if bridge().tool(&bridged).is_some() {
        return Some(bridged);
    }
    let reserved = format!("{RESERVED_PREFIX}{bare}");
    RESERVED_ACTIONS
        .contains(&reserved.as_str())
        .then_some(reserved)
}

/// An action without its namespace: `page.clickElement` → `clickElement`.
pub fn short_name(action: &str) -> &str {
    action
        .strip_prefix("page.")
        .or_else(|| action.strip_prefix("bridge."))
        .or_else(|| action.strip_prefix(RESERVED_PREFIX))
        .unwrap_or(action)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifests_load() {
        assert_eq!(chromium().hash.len(), 8);
        assert!(chromium().tools.len() > firefox().tools.len());
        assert!(bundled(&firefox().hash).is_some());
        assert_eq!(chromium().group_of("page.dragElement"), Some("drag"));
    }

    #[test]
    fn names() {
        for name in [
            "page.dragElement",
            "page_dragElement",
            "dragElement",
            "mcp__vexdom__page_dragElement",
        ] {
            assert_eq!(
                resolve_action(name).as_deref(),
                Some("page.dragElement"),
                "{name}"
            );
        }
        assert_eq!(resolve_action("nothing"), None);
        assert_eq!(
            resolve_action("saveSiteMap").as_deref(),
            Some(SAVE_SITE_MAP_ACTION)
        );
        assert_eq!(short_name("page.clickElement"), "clickElement");
        assert_eq!(short_name("vexdom.focusShot"), "focusShot");
    }

    /// `act` reads `do` and `batch` itself, so no action may take an input named either.
    #[test]
    fn no_action_declares_the_act_keys() {
        for manifest in [chromium(), firefox(), bridge()] {
            for tool in &manifest.tools {
                let properties = tool
                    .get("inputSchema")
                    .and_then(|schema| schema.get("properties"));
                for key in ["do", "batch"] {
                    assert!(
                        properties.and_then(|p| p.get(key)).is_none(),
                        "{} declares {key}",
                        tool["name"]
                    );
                }
            }
        }
    }
}
