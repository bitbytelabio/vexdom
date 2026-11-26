//! `auth.json`: pairing codes, sessions, approvals and blocks (`src/daemon/auth-store.ts`). Read on
//! every use; written as pretty JSON, mode 0600.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::{log, paths, rand};
use vexdom_pure::text::utf16_prefix;

const CODE_LENGTH: usize = 8;
/// A grant travels through native messaging and is never typed, so it carries ~126 bits.
const GRANT_LENGTH: usize = 26;
const PAIRING_TTL_MS: i64 = 10 * 60 * 1000;
/// Kept so a revoked or denied browser stays out; the oldest are let go past this many.
const BLOCKED_KEPT: usize = 200;
/// The app, a terminal and a second terminal can each be showing one.
const LIVE_PAIRINGS: usize = 3;

fn auth_path() -> PathBuf {
    paths::state_dir().join("auth.json")
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Session {
    pub key: String,
    pub origin: String,
    /// Absent on a session paired before the extension sent one; the first reconnect claims it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub install_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub browser: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// `code` | `auto` | `approved`; absent on sessions from before automatic pairing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub paired_by: Option<String>,
    pub extension_version: String,
    pub paired_at: String,
    pub last_seen_at: String,
}

impl Session {
    /// `sessionId`: the install id, or the origin for a session that predates it.
    pub fn id(&self) -> &str {
        self.install_id.as_deref().unwrap_or(&self.origin)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Approval {
    pub install_id: String,
    pub origin: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub browser: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    pub at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BlockedBrowser {
    pub install_id: String,
    pub origin: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub browser: Option<String>,
    pub at: String,
    /// `revoked` | `denied`.
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Pairing {
    code: String,
    expires_at: i64,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct AuthFile {
    #[serde(default)]
    pairings: Vec<Pairing>,
    #[serde(default)]
    sessions: Vec<Session>,
    #[serde(default)]
    approved: Vec<Approval>,
    #[serde(default)]
    blocked: Vec<BlockedBrowser>,
}

fn read() -> AuthFile {
    std::fs::read(auth_path())
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

fn write(auth: &AuthFile) {
    if let Err(error) = paths::write_private_json(&auth_path(), auth) {
        log::log_detail("could not write auth.json", &error.to_string());
    }
}

/// Who connects: what the extension said in its hello, plus its socket origin.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Install {
    pub install_id: String,
    pub origin: String,
    pub extension_version: String,
    pub browser: Option<String>,
}

/// In the typed codes' alphabet, so the extension's handshake treats it as any code.
pub fn mint_grant_code() -> String {
    rand::code(GRANT_LENGTH)
}

/// A new pairing code; at most [`LIVE_PAIRINGS`] stay live.
pub fn create_pairing() -> (String, i64) {
    let mut auth = read();
    let code = rand::code(CODE_LENGTH);
    let now = log::now_ms();
    let expires_at = now + PAIRING_TTL_MS;
    let mut live: Vec<Pairing> = auth
        .pairings
        .into_iter()
        .filter(|pairing| pairing.expires_at > now)
        .collect();
    let keep = LIVE_PAIRINGS - 1;
    if live.len() > keep {
        live.drain(..live.len() - keep);
    }
    live.push(Pairing {
        code: code.clone(),
        expires_at,
    });
    auth.pairings = live;
    write(&auth);
    (code, expires_at)
}

pub fn pending_pairings() -> Vec<String> {
    let now = log::now_ms();
    read()
        .pairings
        .into_iter()
        .filter(|pairing| pairing.expires_at > now)
        .map(|pairing| pairing.code)
        .collect()
}

pub fn consume_pairing(code: &str) {
    let mut auth = read();
    auth.pairings.retain(|pairing| pairing.code != code);
    write(&auth);
}

pub fn has_pending_pairing() -> bool {
    let now = log::now_ms();
    read()
        .pairings
        .iter()
        .any(|pairing| pairing.expires_at > now)
}

pub fn create_session(install: &Install, paired_by: &str) -> Session {
    let mut auth = read();
    let now = log::iso_now();
    let label = auth
        .approved
        .iter()
        .find(|approval| approval.install_id == install.install_id)
        .and_then(|approval| approval.label.clone())
        .filter(|label| !label.is_empty());
    let session = Session {
        key: rand::base64url(32),
        origin: install.origin.clone(),
        install_id: Some(install.install_id.clone()),
        browser: install.browser.clone(),
        label,
        paired_by: Some(paired_by.to_owned()),
        extension_version: install.extension_version.clone(),
        paired_at: now.clone(),
        last_seen_at: now,
    };
    auth.sessions
        .retain(|existing| existing.install_id.as_deref() != Some(&install.install_id));
    auth.sessions.push(session.clone());
    auth.blocked
        .retain(|block| block.install_id != install.install_id);
    write(&auth);
    session
}

/// The sessions this install may hold the key to: its own, else origin-only sessions that predate
/// install ids (the proof settles which).
pub fn session_candidates(install_id: &str, origin: &str) -> Vec<Session> {
    let sessions = read().sessions;
    let own: Vec<Session> = sessions
        .iter()
        .filter(|candidate| candidate.install_id.as_deref() == Some(install_id))
        .cloned()
        .collect();
    if !own.is_empty() {
        return own;
    }
    sessions
        .into_iter()
        .filter(|candidate| candidate.install_id.is_none() && candidate.origin == origin)
        .collect()
}

pub fn claim_session(key: &str, install: &Install) {
    let mut auth = read();
    let Some(session) = auth
        .sessions
        .iter_mut()
        .find(|candidate| candidate.key == key)
    else {
        return;
    };
    session.install_id = Some(install.install_id.clone());
    session.extension_version = install.extension_version.clone();
    session.browser = install.browser.clone();
    session.last_seen_at = log::iso_now();
    write(&auth);
}

pub fn list_sessions() -> Vec<Session> {
    read().sessions
}

/// Unpairs the matching browsers and blocks them from pairing again by themselves.
pub fn revoke_sessions(mut predicate: impl FnMut(&Session) -> bool) -> usize {
    let mut auth = read();
    let (revoked, keep): (Vec<Session>, Vec<Session>) = auth
        .sessions
        .drain(..)
        .partition(|session| predicate(session));
    let at = log::iso_now();
    let blocks = revoked
        .iter()
        .filter_map(|session| {
            Some(BlockedBrowser {
                install_id: session.install_id.clone()?,
                origin: session.origin.clone(),
                browser: session.browser.clone(),
                at: at.clone(),
                reason: "revoked".into(),
            })
        })
        .collect();
    with_blocks(&mut auth, blocks);
    auth.sessions = keep;
    write(&auth);
    revoked.len()
}

fn with_blocks(auth: &mut AuthFile, blocks: Vec<BlockedBrowser>) {
    let ids: Vec<String> = blocks
        .iter()
        .map(|block| block.install_id.clone())
        .collect();
    auth.approved
        .retain(|approval| !ids.contains(&approval.install_id));
    auth.blocked
        .retain(|block| !ids.contains(&block.install_id));
    auth.blocked.extend(blocks);
    if auth.blocked.len() > BLOCKED_KEPT {
        let cut = auth.blocked.len() - BLOCKED_KEPT;
        auth.blocked.drain(..cut);
    }
}

pub fn block_install(install_id: &str, origin: &str, browser: Option<&str>, reason: &str) {
    let mut auth = read();
    with_blocks(
        &mut auth,
        vec![BlockedBrowser {
            install_id: install_id.to_owned(),
            origin: origin.to_owned(),
            browser: browser.map(str::to_owned),
            at: log::iso_now(),
            reason: reason.to_owned(),
        }],
    );
    write(&auth);
}

/// True when there was a block to lift.
pub fn unblock_install(install_id: &str) -> bool {
    let mut auth = read();
    let before = auth.blocked.len();
    auth.blocked.retain(|block| block.install_id != install_id);
    if auth.blocked.len() == before {
        return false;
    }
    write(&auth);
    true
}

pub fn block_of(install_id: &str) -> Option<BlockedBrowser> {
    read()
        .blocked
        .into_iter()
        .find(|block| block.install_id == install_id)
}

pub fn list_blocked() -> Vec<BlockedBrowser> {
    read().blocked
}

pub fn approve_install(install_id: &str, origin: &str, browser: Option<&str>, label: Option<&str>) {
    let mut auth = read();
    auth.approved.retain(|kept| kept.install_id != install_id);
    auth.approved.push(Approval {
        install_id: install_id.to_owned(),
        origin: origin.to_owned(),
        browser: browser.map(str::to_owned),
        label: label.map(str::to_owned),
        at: log::iso_now(),
    });
    auth.blocked.retain(|block| block.install_id != install_id);
    write(&auth);
}

pub fn is_approved(install_id: &str) -> bool {
    read()
        .approved
        .iter()
        .any(|approval| approval.install_id == install_id)
}

/// True when that browser was paired; an empty name clears it. Control characters are dropped and
/// the name is cut to 40 UTF-16 units.
pub fn label_session(id: &str, label: &str) -> bool {
    let mut auth = read();
    let Some(session) = auth
        .sessions
        .iter_mut()
        .find(|candidate| candidate.id() == id)
    else {
        return false;
    };
    let cleaned: String = label.chars().filter(|c| (*c as u32) > 0x1f).collect();
    let named = utf16_prefix(cleaned.trim(), 40).to_owned();
    session.label = (!named.is_empty()).then_some(named);
    write(&auth);
    true
}

/// `text.slice(0, n)` on UTF-16 units (a split surrogate pair is dropped).
pub fn clear_auth() {
    let _ = std::fs::remove_file(auth_path());
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::sync::{Mutex, MutexGuard};

    static HOME: Mutex<()> = Mutex::new(());

    /// Points `VEXDOM_HOME` at a fresh folder for one test; tests that touch state files serialize.
    pub(crate) fn scratch_home(name: &str) -> MutexGuard<'static, ()> {
        let guard = HOME.lock().unwrap_or_else(|poison| poison.into_inner());
        let dir = std::env::temp_dir().join(format!("vexdom-core-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        set_home(&dir);
        guard
    }

    #[allow(unsafe_code)]
    fn set_home(dir: &std::path::Path) {
        // SAFETY: every test that reads or writes VEXDOM_HOME holds the HOME mutex.
        unsafe { std::env::set_var("VEXDOM_HOME", dir) };
    }

    fn install(id: &str) -> Install {
        Install {
            install_id: id.into(),
            origin: "chrome-extension://abc".into(),
            extension_version: "0.9.2".into(),
            browser: Some("Chrome".into()),
        }
    }

    #[test]
    fn pairings_keep_three_live() {
        let _home = scratch_home("pairings");
        let codes: Vec<String> = (0..4).map(|_| create_pairing().0).collect();
        assert_eq!(pending_pairings(), codes[1..].to_vec());
        assert!(has_pending_pairing());
        consume_pairing(&codes[2]);
        assert_eq!(pending_pairings(), vec![codes[1].clone(), codes[3].clone()]);
    }

    #[test]
    fn sessions_revoke_block_and_label() {
        let _home = scratch_home("sessions");
        approve_install("install-aaaaaaaaaaaa", "o", None, Some("Work"));
        let session = create_session(&install("install-aaaaaaaaaaaa"), "approved");
        assert_eq!(session.label.as_deref(), Some("Work"));
        assert_eq!(session.key.len(), 43);
        assert_eq!(session_candidates("install-aaaaaaaaaaaa", "x").len(), 1);
        assert!(label_session("install-aaaaaaaaaaaa", "  Home\u{7}  "));
        assert_eq!(list_sessions()[0].label.as_deref(), Some("Home"));
        assert!(label_session("install-aaaaaaaaaaaa", ""));
        assert_eq!(list_sessions()[0].label, None);
        assert!(!label_session("nope", "x"));
        assert_eq!(revoke_sessions(|_| true), 1);
        let blocked = block_of("install-aaaaaaaaaaaa").unwrap();
        assert_eq!(blocked.reason, "revoked");
        assert!(!is_approved("install-aaaaaaaaaaaa"));
        assert!(unblock_install("install-aaaaaaaaaaaa"));
        assert!(!unblock_install("install-aaaaaaaaaaaa"));
        let text = std::fs::read_to_string(auth_path()).unwrap();
        assert!(text.starts_with("{\n  \"pairings\": [],\n  \"sessions\": [],"));
    }

    #[test]
    fn legacy_sessions_are_candidates_by_origin() {
        let _home = scratch_home("legacy");
        paths::write_private_json(
            &auth_path(),
            &serde_json::json!({"sessions": [{"key": "k", "origin": "chrome-extension://abc",
                "extensionVersion": "0.1.0", "pairedAt": "a", "lastSeenAt": "b"}]}),
        )
        .unwrap();
        let candidates = session_candidates("install-bbbbbbbbbbbb", "chrome-extension://abc");
        assert_eq!(candidates.len(), 1);
        claim_session("k", &install("install-bbbbbbbbbbbb"));
        assert_eq!(
            list_sessions()[0].install_id.as_deref(),
            Some("install-bbbbbbbbbbbb")
        );
    }
}
