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
