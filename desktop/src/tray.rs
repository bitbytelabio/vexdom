//! The menu-bar item, the app's main surface: the switch-and-lamp mark as a template image (macOS
//! tints it) over a native menu with the Bridge's state and facts, Start or Stop and Restart,
//! browsers asking to pair and actions waiting for approval, the agent's model, the pairing mode,
//! notices, appearance and the app's switches, Settings…, updates, About and Quit. The menu is laid
//! out as plain data (`entries`) so it is tested off the main thread, and rebuilt only when what it
//! shows changes.
//!
//! The Dock icon shows only when Settings says so (`showInDock`, off by default); otherwise the app
//! is an accessory, in the menu bar alone, even while its window is open. An unbundled run
//! (`cargo run`) gets the app icon from `brand/` (`uv run brand/build.py`) when it shows in the Dock.

use std::sync::Arc;

use objc2::MainThreadMarker;
use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy};
use tray_icon::menu::{
    CheckMenuItem, IsMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu,
};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};

use crate::model::{PAIRING_MODES, UpdateState};
use crate::settings::Appearance;

const MENU_BAR_TEMPLATE: &[u8] = include_bytes!("../../brand/MenuBarTemplate@2x.png");
const APP_ICON: &[u8] = include_bytes!("../../brand/AppIcon-1024.png");

/// What a menu click asks for. Its menu id is `verb` or `verb:rest` (`id`, `parse`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Pick {
    Power,
    Restart,
    /// Allow the pair request with this id, named as suggested.
    Allow(String),
    /// Allow it after naming it in the window.
    AllowAndName(String),
    Deny(String),
    Confirm {
        id: String,
        allow: bool,
        remember: bool,
    },
    /// `None` is the CLI's default model.
    Model(Option<String>),
    /// "auto", "ask" or "code".
    Pairing(String),
    PairCode,
    /// The notice with this id.
    Notice(String),
    Appearance(Appearance),
    /// Flip the current value.
    StartOnLaunch,
    OpenAtLogin,
    ShowInDock,
    Settings,
    CheckUpdate,
    InstallUpdate,
    About,
    Quit,
}

impl Pick {
    pub fn id(&self) -> String {
        match self {
            Self::Power => "power".to_owned(),
            Self::Restart => "restart".to_owned(),
            Self::Allow(id) => format!("allow:{id}"),
            Self::AllowAndName(id) => format!("allow-name:{id}"),
            Self::Deny(id) => format!("deny:{id}"),
            Self::Confirm {
                id,
                allow: true,
                remember: true,
            } => format!("confirm-always:{id}"),
            Self::Confirm {
                id, allow: true, ..
            } => format!("confirm-allow:{id}"),
            Self::Confirm { id, .. } => format!("confirm-deny:{id}"),
            Self::Model(model) => format!("model:{}", model.as_deref().unwrap_or("")),
            Self::Pairing(mode) => format!("pairing:{mode}"),
            Self::PairCode => "pair-code".to_owned(),
            Self::Notice(id) => format!("notice:{id}"),
            Self::Appearance(appearance) => format!("appearance:{}", appearance.id()),
            Self::StartOnLaunch => "start-on-launch".to_owned(),
            Self::OpenAtLogin => "open-at-login".to_owned(),
            Self::ShowInDock => "show-in-dock".to_owned(),
            Self::Settings => "settings".to_owned(),
            Self::CheckUpdate => "check-update".to_owned(),
            Self::InstallUpdate => "install-update".to_owned(),
            Self::About => "about".to_owned(),
            Self::Quit => "quit".to_owned(),
        }
    }

    pub fn parse(id: &str) -> Option<Self> {
        let confirm = |rest: &str, allow: bool, remember: bool| Self::Confirm {
            id: rest.to_owned(),
            allow,
            remember,
        };
        let Some((verb, rest)) = id.split_once(':') else {
            return Some(match id {
                "power" => Self::Power,
                "restart" => Self::Restart,
                "pair-code" => Self::PairCode,
                "start-on-launch" => Self::StartOnLaunch,
                "open-at-login" => Self::OpenAtLogin,
                "show-in-dock" => Self::ShowInDock,
                "settings" => Self::Settings,
                "check-update" => Self::CheckUpdate,
                "install-update" => Self::InstallUpdate,
                "about" => Self::About,
                "quit" => Self::Quit,
                _ => return None,
            });
        };
        Some(match verb {
            "allow" => Self::Allow(rest.to_owned()),
            "allow-name" => Self::AllowAndName(rest.to_owned()),
            "deny" => Self::Deny(rest.to_owned()),
            "confirm-allow" => confirm(rest, true, false),
            "confirm-always" => confirm(rest, true, true),
            "confirm-deny" => confirm(rest, false, false),
            "model" => Self::Model((!rest.is_empty()).then(|| rest.to_owned())),
            "pairing" => Self::Pairing(rest.to_owned()),
            "notice" => Self::Notice(rest.to_owned()),
            "appearance" => Self::Appearance(Appearance::parse(rest)?),
            _ => return None,
        })
    }
}

/// An action waiting for approval, as the menu shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MenuConfirmation {
    pub id: String,
    pub question: String,
    pub summary: String,
    /// Empty when it acts on no site.
    pub site: String,
}

/// Everything the menu shows, as plain data; the window builds it and the menu is rebuilt only
/// when it changes.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MenuState {
    /// The headline, or who is asking.
    pub line: String,
    pub facts: Vec<(&'static str, String)>,
    /// (id, name, suggested label).
    pub requests: Vec<(String, String, String)>,
    pub confirmations: Vec<MenuConfirmation>,
    pub power: String,
    pub power_enabled: bool,
    pub restart_enabled: bool,
    /// (current model, models); `None` while there is no agent to show.
    pub agent: Option<(String, Vec<String>)>,
    /// The pairing mode; `None` while the Bridge is off.
    pub pairing: Option<String>,
    /// (id, title), newest first.
    pub notices: Vec<(String, String)>,
    pub appearance: Appearance,
    pub start_on_launch: bool,
    /// `None` when there is no login item (an unbundled run).
    pub open_at_login: Option<bool>,
    pub show_in_dock: bool,
    pub update: UpdateState,
}

/// One menu entry, before it becomes a native item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Entry {
    /// A line that cannot be clicked.
    Text(String),
    Action {
        pick: Pick,
        text: String,
        enabled: bool,
    },
    Check {
        pick: Pick,
        text: String,
        checked: bool,
    },
    Sub {
        text: String,
        entries: Vec<Entry>,
    },
    Separator,
}

fn action(pick: Pick, text: impl Into<String>) -> Entry {
    Entry::Action {
        pick,
        text: text.into(),
        enabled: true,
    }
}

fn check(pick: Pick, text: impl Into<String>, checked: bool) -> Entry {
    Entry::Check {
        pick,
        text: text.into(),
        checked,
    }
}

/// The menu, top to bottom: status, what waits, power, the Bridge's choices, preferences, the
/// app, Quit; sections apart by a separator, an empty one left out.
pub fn entries(state: &MenuState) -> Vec<Entry> {
    let mut status = vec![Entry::Text(state.line.clone())];
    status.extend(
        state
            .facts
            .iter()
            .map(|(name, value)| Entry::Text(format!("{name}: {value}"))),
    );

    let mut waiting: Vec<Entry> = state
        .requests
        .iter()
        .map(|(id, name, suggested)| {
            let allow = if suggested.is_empty() {
                "Allow".to_owned()
            } else {
                format!("Allow as “{suggested}”")
            };
            Entry::Sub {
                text: format!("{name} wants to connect"),
                entries: vec![
                    action(Pick::Allow(id.clone()), allow),
                    action(Pick::AllowAndName(id.clone()), "Allow and Name…"),
                    action(Pick::Deny(id.clone()), "Deny"),
                ],
            }
        })
        .collect();
    for confirmation in &state.confirmations {
        let answer = |allow, remember| Pick::Confirm {
            id: confirmation.id.clone(),
            allow,
            remember,
        };
        let mut entries = Vec::new();
        if !confirmation.summary.is_empty() {
            entries.push(Entry::Text(confirmation.summary.clone()));
        }
        entries.push(action(answer(true, false), "Allow"));
        if !confirmation.site.is_empty() {
            entries.push(action(
                answer(true, true),
                format!("Always Allow on {}", confirmation.site),
            ));
        }
        entries.push(action(answer(false, false), "Deny"));
        waiting.push(Entry::Sub {
            text: confirmation.question.clone(),
            entries,
        });
    }

    let power = vec![
        Entry::Action {
            pick: Pick::Power,
            text: state.power.clone(),
            enabled: state.power_enabled,
        },
        Entry::Action {
            pick: Pick::Restart,
            text: "Restart Vexdom".to_owned(),
            enabled: state.restart_enabled,
        },
    ];

    let mut bridge = Vec::new();
    if let Some((current, models)) = &state.agent {
        let mut entries = vec![check(Pick::Model(None), "Default", current.is_empty())];
        entries.extend(
            models
                .iter()
                .map(|model| check(Pick::Model(Some(model.clone())), model, current == model)),
        );
        bridge.push(Entry::Sub {
            text: "Model".to_owned(),
            entries,
        });
    }
    if let Some(mode) = &state.pairing {
        let mut entries: Vec<Entry> = PAIRING_MODES
            .iter()
            .map(|(value, label)| check(Pick::Pairing((*value).to_owned()), *label, mode == value))
            .collect();
        entries.push(Entry::Separator);
        entries.push(action(Pick::PairCode, "Get a Pairing Code…"));
        bridge.push(Entry::Sub {
            text: "Pairing".to_owned(),
            entries,
        });
    }
    if !state.notices.is_empty() {
        bridge.push(Entry::Sub {
            text: "Notices".to_owned(),
            entries: state
                .notices
                .iter()
                .map(|(id, title)| action(Pick::Notice(id.clone()), title))
                .collect(),
        });
    }

    let mut preferences = vec![
        Entry::Sub {
            text: "Appearance".to_owned(),
            entries: Appearance::ALL
                .into_iter()
                .map(|appearance| {
                    check(
                        Pick::Appearance(appearance),
                        appearance.label(),
                        appearance == state.appearance,
                    )
                })
                .collect(),
        },
        check(
            Pick::StartOnLaunch,
            "Start Vexdom When the App Opens",
            state.start_on_launch,
        ),
    ];
    if let Some(on) = state.open_at_login {
        preferences.push(check(Pick::OpenAtLogin, "Open at Login", on));
    }
    preferences.push(check(Pick::ShowInDock, "Show in Dock", state.show_in_dock));

    let update = match &state.update {
        UpdateState::Available(version) => {
            action(Pick::InstallUpdate, format!("Update to Vexdom {version}"))
        }
        UpdateState::Checking => Entry::Text("Checking for Updates…".to_owned()),
        UpdateState::Downloading(version) => Entry::Text(format!("Downloading Vexdom {version}…")),
        UpdateState::Verifying(version) => Entry::Text(format!("Verifying Vexdom {version}…")),
        UpdateState::Relaunching(version) => {
            Entry::Text(format!("Relaunching into Vexdom {version}…"))
        }
        UpdateState::Idle | UpdateState::Latest | UpdateState::Failed(_) => {
            action(Pick::CheckUpdate, "Check for Updates…")
        }
    };
    let app = vec![
        action(Pick::Settings, "Settings…"),
        update,
        action(Pick::About, "About Vexdom"),
    ];

    let quit = vec![action(Pick::Quit, "Quit Vexdom")];

    let mut menu = Vec::new();
    for section in [status, waiting, power, bridge, preferences, app, quit] {
        if section.is_empty() {
            continue;
        }
        if !menu.is_empty() {
            menu.push(Entry::Separator);
        }
        menu.extend(section);
    }
    menu
}

/// muda reads `&` as a mnemonic marker and drops it; `&&` is a literal one.
fn title(text: &str) -> String {
    text.replace('&', "&&")
}

/// The native items for `entries`.
fn build(entries: &[Entry]) -> Result<Vec<Box<dyn IsMenuItem>>, String> {
    entries
        .iter()
        .map(|entry| -> Result<Box<dyn IsMenuItem>, String> {
            Ok(match entry {
                Entry::Text(text) => Box::new(MenuItem::new(title(text), false, None)),
                Entry::Action {
                    pick,
                    text,
                    enabled,
                } => Box::new(MenuItem::with_id(pick.id(), title(text), *enabled, None)),
                Entry::Check {
                    pick,
                    text,
                    checked,
                } => Box::new(CheckMenuItem::with_id(
                    pick.id(),
                    title(text),
                    true,
                    *checked,
                    None,
                )),
                Entry::Sub { text, entries } => {
                    let items = build(entries)?;
                    let refs: Vec<&dyn IsMenuItem> = items.iter().map(AsRef::as_ref).collect();
                    Box::new(
                        Submenu::with_items(title(text), true, &refs)
                            .map_err(|error| error.to_string())?,
                    )
                }
                Entry::Separator => Box::new(PredefinedMenuItem::separator()),
            })
        })
        .collect()
}

fn menu(state: &MenuState) -> Result<Menu, String> {
    let items = build(&entries(state))?;
    let refs: Vec<&dyn IsMenuItem> = items.iter().map(AsRef::as_ref).collect();
    let menu = Menu::new();
    menu.append_items(&refs)
        .map_err(|error| error.to_string())?;
    Ok(menu)
}

pub struct Tray {
    icon: TrayIcon,
    /// What the menu shows now; `None` forces the next `show` to rebuild it.
    last: Option<MenuState>,
}

impl Tray {
    /// Must run on the main thread, after the app has started. `on_pick` runs on the main thread
    /// as each item is clicked.
    pub fn try_new(on_pick: impl Fn(Pick) + Send + Sync + 'static) -> Result<Self, String> {
        let picture =
            image::load_from_memory_with_format(MENU_BAR_TEMPLATE, image::ImageFormat::Png)
                .map_err(|error| error.to_string())?
                .into_rgba8();
        let (width, height) = picture.dimensions();
        let icon = Icon::from_rgba(picture.into_raw(), width, height)
            .map_err(|error| error.to_string())?;

        MenuEvent::set_event_handler(Some(Arc::new(move |event: MenuEvent| {
            if let Some(pick) = Pick::parse(event.id.as_ref()) {
                on_pick(pick);
            }
        })));

        let icon = TrayIconBuilder::new()
            .with_icon_templated(icon)
            .with_tooltip("Vexdom")
            .with_menu(Box::new(menu(&MenuState::default())?))
            .build()
            .map_err(|error| error.to_string())?;
        Ok(Self { icon, last: None })
    }

    /// Rebuilds the menu when `state` differs from what it shows.
    pub fn show(&mut self, state: MenuState) {
        if self.last.as_ref() == Some(&state) {
            return;
        }
        match menu(&state) {
            Ok(menu) => self.icon.set_menu(Some(Box::new(menu))),
            Err(error) => {
                eprintln!("vexdom-desktop: menu-bar menu not rebuilt ({error})");
                return;
            }
        }
        self.last = Some(state);
    }

    /// A click flips a check item's mark by itself; the next `show` redraws it from the state
    /// even when that did not change (the pick failed, or picked what was already chosen).
    pub fn forget(&mut self) {
        self.last = None;
    }
}

/// Brings the app back after the window was closed (closing hides the app).
pub fn reveal() {
    if let Some(main_thread) = MainThreadMarker::new() {
        NSApplication::sharedApplication(main_thread).unhide(None);
    }
}

/// Shows the app in the Dock and the app switcher, or keeps it in the menu bar alone.
pub fn set_dock_visible(visible: bool) {
    let Some(main_thread) = MainThreadMarker::new() else {
        return;
    };
    NSApplication::sharedApplication(main_thread).setActivationPolicy(if visible {
        NSApplicationActivationPolicy::Regular
    } else {
        NSApplicationActivationPolicy::Accessory
    });
    if visible {
        set_dock_icon();
    }
}

/// Names the app notifications are posted as, before the first one. Left unset, notify-rust looks
/// the name up with an AppleScript (`id of application "use_default"`) that opens a "Where is
/// use_default?" chooser, and AppKit traps when that runs off the main thread. A bundled app
/// posts as itself; `cargo run` posts as Finder, the library's own fallback.
pub fn set_notification_sender() {
    use objc2_foundation::NSBundle;

    let bundled = NSBundle::mainBundle()
        .bundleIdentifier()
        .map(|id| id.to_string());
    let sender = bundled.as_deref().unwrap_or("com.apple.Finder");
    if let Err(error) = notify_rust::set_application(sender) {
        eprintln!("vexdom-desktop: notifications post as the default sender ({error})");
    }
}

/// Gives an unbundled run (`cargo run`) the app icon in the Dock and the app switcher; a bundled
/// `Vexdom.app` shows its own icon (`Assets.car`, `AppIcon.icns`) and is left alone.
fn set_dock_icon() {
    use objc2::AllocAnyThread;
    use objc2_app_kit::NSImage;
    use objc2_foundation::{NSBundle, NSData};

    if NSBundle::mainBundle().bundleIdentifier().is_some() {
        return;
    }
    let Some(main_thread) = MainThreadMarker::new() else {
        return;
    };
    let data = NSData::with_bytes(APP_ICON);
    if let Some(image) = NSImage::initWithData(NSImage::alloc(), &data) {
        let app = NSApplication::sharedApplication(main_thread);
        // SAFETY: on the main thread (checked above), with an image AppKit just decoded.
        unsafe { app.setApplicationIconImage(Some(&image)) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stopped() -> MenuState {
        MenuState {
            line: "Vexdom is off".to_owned(),
            power: "Start Vexdom".to_owned(),
            power_enabled: true,
            ..MenuState::default()
        }
    }

    /// Every entry, submenus flattened in.
    fn flat(entries: &[Entry]) -> Vec<&Entry> {
        entries
            .iter()
            .flat_map(|entry| match entry {
                Entry::Sub { entries, .. } => {
                    let mut all = vec![entry];
                    all.extend(flat(entries));
                    all
                }
                _ => vec![entry],
            })
            .collect()
    }

    fn sub<'a>(entries: &'a [Entry], name: &str) -> Option<&'a [Entry]> {
        entries.iter().find_map(|entry| match entry {
            Entry::Sub { text, entries } if text == name => Some(entries.as_slice()),
            _ => None,
        })
    }

    #[test]
    fn every_pick_round_trips_through_its_menu_id() {
        let picks = [
            Pick::Power,
            Pick::Restart,
            Pick::Allow("r:1".to_owned()),
            Pick::AllowAndName("r1".to_owned()),
            Pick::Deny("r1".to_owned()),
            Pick::Confirm {
                id: "c:9:x".to_owned(),
                allow: true,
                remember: false,
            },
            Pick::Confirm {
                id: "c1".to_owned(),
                allow: true,
                remember: true,
            },
            Pick::Confirm {
                id: "c1".to_owned(),
                allow: false,
                remember: false,
            },
            Pick::Model(None),
            Pick::Model(Some("claude-opus-4:1m".to_owned())),
            Pick::Pairing("ask".to_owned()),
            Pick::PairCode,
            Pick::Notice("n:7".to_owned()),
            Pick::Appearance(Appearance::Dark),
            Pick::StartOnLaunch,
            Pick::OpenAtLogin,
            Pick::ShowInDock,
            Pick::Settings,
            Pick::CheckUpdate,
            Pick::InstallUpdate,
            Pick::About,
            Pick::Quit,
        ];
        for pick in picks {
            assert_eq!(Pick::parse(&pick.id()), Some(pick.clone()), "{}", pick.id());
        }
        assert_eq!(Pick::parse("bogus"), None);
        assert_eq!(Pick::parse("allow"), None);
        assert_eq!(Pick::parse("appearance:sepia"), None);
    }

    #[test]
    fn a_stopped_bridge_offers_start_and_no_bridge_choices() {
        let menu = entries(&stopped());
        assert!(sub(&menu, "Model").is_none());
        assert!(sub(&menu, "Pairing").is_none());
        assert!(menu.contains(&Entry::Action {
            pick: Pick::Restart,
            text: "Restart Vexdom".to_owned(),
            enabled: false,
        }));
        assert!(menu.contains(&action(Pick::Power, "Start Vexdom")));
        assert_eq!(menu.first(), Some(&Entry::Text("Vexdom is off".to_owned())));
        assert_eq!(menu.last(), Some(&action(Pick::Quit, "Quit Vexdom")));
        assert!(
            !menu
                .windows(2)
                .any(|pair| pair == [Entry::Separator, Entry::Separator])
        );
    }

    #[test]
    fn a_request_and_a_confirmation_are_answered_from_the_menu() {
        let state = MenuState {
            requests: vec![("r1".to_owned(), "Chrome".to_owned(), "Work".to_owned())],
            confirmations: vec![MenuConfirmation {
                id: "c1".to_owned(),
                question: "Allow click on github.com?".to_owned(),
                summary: String::new(),
                site: "github.com".to_owned(),
            }],
            ..stopped()
        };
        let menu = entries(&state);
        let request = sub(&menu, "Chrome wants to connect").expect("the request");
        assert_eq!(
            request.first(),
            Some(&action(Pick::Allow("r1".to_owned()), "Allow as “Work”"))
        );
        let confirmation = sub(&menu, "Allow click on github.com?").expect("the confirmation");
        assert!(confirmation.contains(&action(
            Pick::Confirm {
                id: "c1".to_owned(),
                allow: true,
                remember: true,
            },
            "Always Allow on github.com",
        )));
    }

    #[test]
    fn an_available_update_is_installed_from_the_menu() {
        let state = MenuState {
            update: UpdateState::Available("0.11.0".to_owned()),
            ..stopped()
        };
        assert!(entries(&state).contains(&action(Pick::InstallUpdate, "Update to Vexdom 0.11.0")));
    }

    #[test]
    fn open_at_login_shows_only_with_a_login_item() {
        let has_login = |state: &MenuState| {
            flat(&entries(state)).into_iter().any(|entry| {
                matches!(
                    entry,
                    Entry::Check {
                        pick: Pick::OpenAtLogin,
                        ..
                    }
                )
            })
        };
        assert!(!has_login(&stopped()));
        assert!(has_login(&MenuState {
            open_at_login: Some(true),
            ..stopped()
        }));
    }
}
