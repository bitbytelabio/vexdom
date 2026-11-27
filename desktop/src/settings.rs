//! The app's own settings: `~/.vexdom/desktop.json`, `{appearance, startBridgeOnLaunch,
//! finishUpdateOnLaunch, showInDock}`. Read at launch; a change reads the file, changes one key and writes the
//! whole file through a temporary one renamed over it, so the window and an update in flight each
//! write only their own key and a crash never leaves half a file. Kept apart from GPUI so it is
//! tested without a window.

use std::io;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

/// Light or dark: whatever macOS shows, or one of them always.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Appearance {
    #[default]
    System,
    Light,
    Dark,
}

impl Appearance {
    pub const ALL: [Self; 3] = [Self::System, Self::Light, Self::Dark];

    pub fn id(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::Light => "light",
            Self::Dark => "dark",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::System => "System",
            Self::Light => "Light",
            Self::Dark => "Dark",
        }
    }

    pub(crate) fn parse(id: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|appearance| appearance.id() == id)
    }

    /// Whether the window is dark, given whether macOS is.
    pub fn is_dark(self, system_dark: bool) -> bool {
        match self {
            Self::System => system_dark,
            Self::Light => false,
            Self::Dark => true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Settings {
    pub appearance: Appearance,
    /// Start the Bridge when the app opens and finds it off.
    pub start_bridge_on_launch: bool,
    /// A new Vexdom.app was put in place: finish it (`vexdom setup`) when it opens.
    pub finish_update_on_launch: bool,
    /// Show the app's icon in the Dock; off keeps it in the menu bar only.
    pub show_in_dock: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            appearance: Appearance::System,
            start_bridge_on_launch: true,
            finish_update_on_launch: false,
            show_in_dock: false,
        }
    }
}

impl Settings {
    /// The file's JSON; a key that is missing or of another type keeps its default.
    pub fn from_json(value: &Value) -> Self {
        let defaults = Self::default();
        let flag =
            |key: &str, default: bool| value.get(key).and_then(Value::as_bool).unwrap_or(default);
        Self {
            appearance: value
                .get("appearance")
                .and_then(Value::as_str)
                .and_then(Appearance::parse)
                .unwrap_or(defaults.appearance),
            start_bridge_on_launch: flag("startBridgeOnLaunch", defaults.start_bridge_on_launch),
            finish_update_on_launch: flag("finishUpdateOnLaunch", defaults.finish_update_on_launch),
            show_in_dock: flag("showInDock", defaults.show_in_dock),
        }
    }

    pub fn to_json(self) -> Value {
        json!({
            "appearance": self.appearance.id(),
            "startBridgeOnLaunch": self.start_bridge_on_launch,
            "finishUpdateOnLaunch": self.finish_update_on_launch,
            "showInDock": self.show_in_dock,
        })
    }
}

/// `~/.vexdom/desktop.json`; `VEXDOM_HOME` moves it with the rest of the state.
pub fn path() -> PathBuf {
    vexdom_core::paths::state_dir().join("desktop.json")
}

pub fn load() -> Settings {
    read(&path())
}

/// Reads the file, applies `change` and writes the result back.
pub fn edit(change: impl FnOnce(&mut Settings)) -> io::Result<Settings> {
    edit_file(&path(), change)
}

/// What `file` holds; the defaults when it is missing or does not read.
pub fn read(file: &Path) -> Settings {
    std::fs::read_to_string(file)
        .ok()
        .and_then(|text| serde_json::from_str::<Value>(&text).ok())
        .map_or_else(Settings::default, |value| Settings::from_json(&value))
}

/// Writes `settings` whole: into a temporary file beside `file` (0600, in a 0700 folder like the
/// rest of the state), then renamed over it.
pub fn write(file: &Path, settings: Settings) -> io::Result<()> {
    let mut text = serde_json::to_string_pretty(&settings.to_json()).map_err(io::Error::other)?;
    text.push('\n');
    let temporary = file.with_extension(format!("json.{}.tmp", std::process::id()));
    vexdom_core::paths::write_private(&temporary, text.as_bytes())?;
    std::fs::rename(&temporary, file).inspect_err(|_| {
        let _ = std::fs::remove_file(&temporary);
    })
}

pub fn edit_file(file: &Path, change: impl FnOnce(&mut Settings)) -> io::Result<Settings> {
    let mut settings = read(file);
    change(&mut settings);
    write(file, settings)?;
    Ok(settings)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "vexdom-desktop-settings-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        dir.join("desktop.json")
    }

    #[test]
    fn defaults_fill_what_is_missing_or_mistyped() {
        assert_eq!(read(&scratch("missing")), Settings::default());
        assert_eq!(
            Settings::default(),
            Settings {
                appearance: Appearance::System,
                start_bridge_on_launch: true,
                finish_update_on_launch: false,
                show_in_dock: false,
            }
        );
        let mixed = Settings::from_json(
            &json!({"appearance": "sepia", "startBridgeOnLaunch": "no", "finishUpdateOnLaunch": true, "showInDock": 1}),
        );
        assert_eq!(
            mixed,
            Settings {
                finish_update_on_launch: true,
                ..Settings::default()
            }
        );
        assert_eq!(Settings::from_json(&json!([1, 2])), Settings::default());
    }

    #[test]
    fn settings_round_trip_through_the_file() {
        let file = scratch("round-trip");
        let chosen = Settings {
            appearance: Appearance::Dark,
            start_bridge_on_launch: false,
            finish_update_on_launch: true,
            show_in_dock: true,
        };
        write(&file, chosen).expect("written");
        assert_eq!(read(&file), chosen);
        let text = std::fs::read_to_string(&file).expect("read");
        assert!(text.contains("\"startBridgeOnLaunch\": false"));
        // No temporary file is left beside it.
        let left: Vec<_> = std::fs::read_dir(file.parent().expect("a folder"))
            .expect("listed")
            .filter_map(Result::ok)
            .map(|entry| entry.file_name())
            .collect();
        assert_eq!(left, ["desktop.json"]);
        let _ = std::fs::remove_dir_all(file.parent().expect("a folder"));
    }

    #[test]
    fn an_edit_changes_one_key_and_keeps_the_rest() {
        let file = scratch("edit");
        edit_file(&file, |settings| settings.appearance = Appearance::Light).expect("edited");
        let after =
            edit_file(&file, |settings| settings.finish_update_on_launch = true).expect("edited");
        assert_eq!(after.appearance, Appearance::Light);
        assert!(after.finish_update_on_launch && after.start_bridge_on_launch);
        assert_eq!(read(&file), after);
        let _ = std::fs::remove_dir_all(file.parent().expect("a folder"));
    }

    #[test]
    fn an_appearance_other_than_system_overrides_macos() {
        assert!(Appearance::System.is_dark(true));
        assert!(!Appearance::System.is_dark(false));
        assert!(!Appearance::Light.is_dark(true));
        assert!(Appearance::Dark.is_dark(false));
        assert_eq!(Appearance::parse("dark"), Some(Appearance::Dark));
    }
}
