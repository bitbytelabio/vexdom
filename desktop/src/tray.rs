//! The menu-bar item (the switch-and-lamp mark as a template image, so macOS tints it) with the
//! state, Start or Stop, Restart, Open Vexdom and Quit; and the Dock icon when the app runs unbundled
//! (`cargo run`). Both come from `brand/` (`uv run brand/build.py`).

use std::sync::Arc;

use objc2::MainThreadMarker;
use objc2_app_kit::NSApplication;
use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};

const MENU_BAR_TEMPLATE: &[u8] = include_bytes!("../../brand/MenuBarTemplate@2x.png");
const APP_ICON: &[u8] = include_bytes!("../../brand/AppIcon-1024.png");

/// What a menu click asks for.
pub enum Pick {
    Power,
    Restart,
    Open,
    Quit,
}

pub struct Tray {
    _icon: TrayIcon,
    state: MenuItem,
    power: MenuItem,
    restart: MenuItem,
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

        let state = MenuItem::new("Vexdom is off", false, None);
        let power = MenuItem::new("Start Vexdom", true, None);
        let restart = MenuItem::new("Restart Vexdom", false, None);
        let open = MenuItem::new("Open Vexdom", true, None);
        let quit = MenuItem::new("Quit Vexdom", true, None);
        let menu = Menu::new();
        menu.append_items(&[
            &state,
            &power,
            &restart,
            &PredefinedMenuItem::separator(),
            &open,
            &quit,
        ])
        .map_err(|error| error.to_string())?;

        let (power_id, restart_id, open_id, quit_id) = (
            power.id().clone(),
            restart.id().clone(),
            open.id().clone(),
            quit.id().clone(),
        );
        MenuEvent::set_event_handler(Some(Arc::new(move |event: MenuEvent| {
            let pick = if event.id == power_id {
                Pick::Power
            } else if event.id == restart_id {
                Pick::Restart
            } else if event.id == open_id {
                Pick::Open
            } else if event.id == quit_id {
                Pick::Quit
            } else {
                return;
            };
            on_pick(pick);
        })));

        let tray = TrayIconBuilder::new()
            .with_icon_templated(icon)
            .with_tooltip("Vexdom")
            .with_menu(Box::new(menu))
            .build()
            .map_err(|error| error.to_string())?;
        Ok(Self {
            _icon: tray,
            state,
            power,
            restart,
        })
    }

    /// Mirrors the window: its headline (or who is asking to connect) and its switch; Restart
    /// only while the Bridge runs.
    pub fn show(&self, line: &str, running: bool) {
        self.state.set_text(line);
        self.power.set_text(if running {
            "Stop Vexdom"
        } else {
            "Start Vexdom"
        });
        self.restart.set_enabled(running);
    }
}

/// Brings the app back after the window was closed (closing hides the app).
pub fn reveal() {
    if let Some(main_thread) = MainThreadMarker::new() {
        NSApplication::sharedApplication(main_thread).unhide(None);
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
pub fn set_dock_icon() {
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
