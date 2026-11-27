//! "Open at login" (`SMAppService.mainAppService`). Only a bundled Vexdom.app has a service to
//! register; `cargo run` has none, and Settings hides the switch there.
//! Off macOS there is none: `state` is `None`.

#[cfg(target_os = "macos")]
use objc2_foundation::NSBundle;
#[cfg(target_os = "macos")]
use objc2_service_management::{SMAppService, SMAppServiceStatus};

/// Where the app stands with macOS's login items.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoginState {
    On,
    Off,
    /// Registered, but switched off in System Settings → General → Login Items.
    NeedsApproval,
}

/// The app's state; `None` when it runs unbundled.
#[cfg(target_os = "macos")]
pub fn state() -> Option<LoginState> {
    NSBundle::mainBundle().bundleIdentifier()?;
    // SAFETY: the main app's own service, created and read on this thread.
    let status = unsafe { SMAppService::mainAppService().status() };
    Some(match status {
        SMAppServiceStatus::Enabled => LoginState::On,
        SMAppServiceStatus::RequiresApproval => LoginState::NeedsApproval,
        _ => LoginState::Off,
    })
}

/// Registers the app to open at login, or unregisters it; macOS's reason when it refuses.
#[cfg(target_os = "macos")]
pub fn set(on: bool) -> Result<(), String> {
    // SAFETY: as in `state`; both calls only take the service they are sent to.
    let result = unsafe {
        let service = SMAppService::mainAppService();
        if on {
            service.registerAndReturnError()
        } else {
            service.unregisterAndReturnError()
        }
    };
    result.map_err(|error| error.localizedDescription().to_string())
}

/// Off macOS there is no login item; `None` hides the switch, as for an unbundled run.
#[cfg(not(target_os = "macos"))]
pub fn state() -> Option<LoginState> {
    None
}

#[cfg(not(target_os = "macos"))]
pub fn set(_on: bool) -> Result<(), String> {
    Err("Open at login is only on macOS.".to_owned())
}

#[cfg(all(test, not(target_os = "macos")))]
mod tests {
    use super::*;

    #[test]
    fn without_macos_there_is_no_login_item_to_offer() {
        assert_eq!(state(), None);
        assert!(set(true).is_err());
    }
}
