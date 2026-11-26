//! "Open at login" (`SMAppService.mainAppService`). Only a bundled Vexdom.app has a service to
//! register; `cargo run` has none, and Settings hides the switch there.

use objc2_foundation::NSBundle;
use objc2_service_management::{SMAppService, SMAppServiceStatus};

/// Where the app stands with macOS's login items.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoginState {
    On,
    Off,
    /// Registered, but switched off in System Settings → General → Login Items.
    NeedsApproval,
}

/// The app's state; `None` when it runs unbundled.
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
