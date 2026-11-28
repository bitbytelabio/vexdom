//! Pure Bridge logic shared by the daemon, the CLI and the MCP server (ADR-025): protocol constants,
//! handshake crypto, the files under the state directory, guardrails and the tool manifest. No
//! async runtime; file I/O only through `std::fs`.

pub mod agents;
pub mod approvals;
pub mod auth_store;
pub mod config;
pub mod downloads;
pub mod guardrails;
pub mod handshake;
pub mod lockfile;
pub mod log;
pub mod notices;
pub mod pairing;
pub mod paths;
pub mod plugins;
pub mod preferences;
pub mod profile;
pub mod project_config;
pub mod proto;
pub mod rand;
pub mod result;
pub mod schedules;
pub mod skills;
pub mod tools;

/// Installs ring as rustls' process-wide crypto provider. reqwest is built without one
/// (`rustls-no-provider`), so call this before building any reqwest client.
pub fn install_tls_provider() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        // Err means a provider is already installed, which is just as good.
        let _ = rustls::crypto::ring::default_provider().install_default();
    });
}

#[cfg(test)]
mod tests {
    #[test]
    fn install_tls_provider_sets_a_process_default() {
        super::install_tls_provider();
        assert!(
            rustls::crypto::CryptoProvider::get_default().is_some(),
            "reqwest clients built after this would panic for want of a rustls provider"
        );
    }
}
