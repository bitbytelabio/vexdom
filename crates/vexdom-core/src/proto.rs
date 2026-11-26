//! Protocol constants shared with the extension: `src/lib/actions/protocol.ts`, `reserved.ts`,
//! `src/lib/notices.ts` and `src/lib/pairing.ts`.

/// The wire protocol this Bridge speaks (ADR-025: Cap'n Proto from 27 on).
pub const SOCKET_PROTOCOL_VERSION: u32 = 27;
/// The oldest extension the Bridge lets in; older extensions speak JSON and cannot connect.
pub const MIN_EXTENSION_PROTOCOL: u32 = 27;
/// The run id of a tool call that arrives from outside any agent run.
pub const EXTERNAL_RUN_ID: &str = "external";
/// The ports the extension walks; `VEXDOM_PORTS` moves the daemon's own (see `paths`).
pub const DAEMON_PORTS: [u16; 3] = [8785, 8786, 8787];
/// The native messaging host the extension asks to start the Bridge.
pub const NATIVE_HOST_NAME: &str = "com.vexdom.bridge";
/// Actions with this prefix are reserved for the extension's own use and refused from outside.
pub const RESERVED_PREFIX: &str = "vexdom.";
/// The first protocol whose extension can show a notice the Bridge asks it to open.
pub const NOTICE_PROTOCOL: u32 = 24;
/// The longest a native-host `grant` waits on a person deciding an ask-mode request.
/// How long the Bridge holds an external action for the user's approval (ADR-032).
pub const CONFIRM_WAIT_MS: u64 = 120_000;
pub const MAX_PAIR_WAIT_MS: u32 = 25_000;
/// The Firefox add-on's id (`src/lib/stores.ts`).
pub const FIREFOX_ADDON_ID: &str = "vexdom@nguyenthdat.github.io";

/// `isInstallId`: `^[A-Za-z0-9_-]{16,64}$`.
pub fn is_install_id(value: &str) -> bool {
    (16..=64).contains(&value.len())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn install_ids() {
        assert!(is_install_id("abcdefghijklmnop"));
        assert!(is_install_id(&"a_-9".repeat(16)));
        assert!(!is_install_id("abcdefghijklmno"));
        assert!(!is_install_id(&"a".repeat(65)));
        assert!(!is_install_id("abcdefghijklmnop!"));
        assert!(!is_install_id("ébcdefghijklmnop"));
    }
}
