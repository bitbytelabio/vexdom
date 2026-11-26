//! Automatic pairing (`src/lib/pairing.ts`) and the desk that decides a native host's request for a
//! grant (`src/daemon/pairing-desk.ts`). Grants and requests live in memory only; approvals and
//! blocks persist in `auth.json`.

use std::collections::HashMap;

use serde::Serialize;

use crate::{auth_store, log, rand};

pub const PAIRING_MODES: [&str; 3] = ["auto", "ask", "code"];
pub const DEFAULT_PAIRING_MODE: &str = "auto";
pub const PAIR_REASONS: [&str; 7] = [
    "install",
    "update",
    "startup",
    "alarm",
    "open",
    "gesture",
    "unknown-session",
];
/// A Firefox extension's socket origin is a random UUID per install, so its grant is bound to the scheme.
pub const FIREFOX_CALLER: &str = "moz-extension://";
/// The ops a host that pairs answers.
pub const HOST_OPS: [&str; 3] = ["ensure", "hello", "pair"];

pub fn is_pairing_mode(value: &str) -> bool {
    PAIRING_MODES.contains(&value)
}

pub fn is_pair_reason(value: &str) -> bool {
    PAIR_REASONS.contains(&value)
}

/// `sourceOf` (`src/lib/stores.ts`): the Firefox add-on, or an unpacked Chromium copy.
pub fn source_of(origin: &str) -> &'static str {
    if origin.starts_with("moz-extension://") {
        "firefox"
    } else {
        "unpacked"
    }
}

/// `SOURCE_LABEL`.
pub fn source_label(source: &str) -> &'static str {
    if source == "firefox" {
        "Firefox add-on"
    } else {
        "unpacked"
    }
}

const GRANT_TTL_MS: i64 = 120_000;
/// A grant with less left than this is replaced rather than handed out again.
const GRANT_REUSE_MS: i64 = 30_000;
const REQUEST_TTL_MS: i64 = 10 * 60_000;
const RETRY_AFTER_MS: i64 = 3_000;
/// How soon after a deny a click in the browser may ask again.
const REASK_AFTER_DENY_MS: i64 = 60_000;

pub const REFUSED_OFF: &str =
    "Automatic pairing is off on this computer. Pair with a code from \"vexdom pair\" or the app.";
pub const REFUSED_REVOKED: &str =
    "This computer unpaired this browser. Press Reconnect, or pair with a code.";
pub const REFUSED_DENIED: &str =
    "This computer turned this browser down. Press Reconnect to ask again, or pair with a code.";

fn refused_words(reason: &str) -> &'static str {
    match reason {
        "revoked" => REFUSED_REVOKED,
        "denied" => REFUSED_DENIED,
        _ => REFUSED_OFF,
    }
}

/// Who is asking, as the native host vouches for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Asker {
    pub install_id: String,
    /// `chrome-extension://<id>/` for Chromium, [`FIREFOX_CALLER`] for Firefox.
    pub caller: String,
    pub browser: Option<String>,
    pub extension_version: String,
    pub gesture: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Grant {
    pub code: String,
    pub port: u16,
    pub expires_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PairWait {
    pub request: String,
    pub approver: String,
    pub retry_after_ms: i64,
}

/// What the Bridge answers a host that asks for a grant (`GrantOutcome`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum GrantOutcome {
    Grant { grant: Grant, approver: String },
    Wait { wait: PairWait },
    Refused { refused: String, error: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PairRequestSummary {
    pub id: String,
    pub install_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub browser: Option<String>,
    pub source: String,
    pub extension_version: String,
    pub first_at: i64,
    pub last_ask_at: i64,
    /// Proves a `vexdom://` link came from the notification the app itself posted.
    pub nonce: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeldGrant {
    pub code: String,
    pub caller: String,
    pub expires_at: i64,
    /// `auto` | `approved`.
    pub paired_by: String,
}

struct PendingRequest {
    summary: PairRequestSummary,
    caller: String,
}

/// What the desk needs from the daemon.
pub trait DeskHost {
    fn port(&self) -> u16;
    fn mode(&self) -> String;
    fn approver(&self) -> String;
    /// The pending list changed.
    fn changed(&self);
    /// That browser's request was decided or answered: whoever waits on it may ask again.
    fn wake(&self, install_id: &str);
    fn now(&self) -> i64 {
        log::now_ms()
    }
}

fn as_origin(origin: &str) -> String {
    if origin.ends_with('/') {
        origin.to_owned()
    } else {
        format!("{origin}/")
    }
}

pub fn caller_matches(caller: &str, origin: &str) -> bool {
    if caller == FIREFOX_CALLER {
        origin.starts_with(FIREFOX_CALLER)
    } else {
        as_origin(origin) == caller
    }
}

fn label(browser: Option<&str>, caller: &str, install_id: &str) -> String {
    let short: String = install_id.chars().take(8).collect();
    format!("{} ({short})", browser.unwrap_or(caller))
}

fn parse_iso_ms(at: &str) -> Option<i64> {
    at.parse::<jiff::Timestamp>()
        .ok()
        .map(|time| time.as_millisecond())
}

pub struct PairingDesk<H: DeskHost> {
    host: H,
    grants: HashMap<String, HeldGrant>,
    /// Insertion-ordered, as the TS `Map`.
    requests: Vec<(String, PendingRequest)>,
}

impl<H: DeskHost> PairingDesk<H> {
    pub fn new(host: H) -> Self {
        Self {
            host,
            grants: HashMap::new(),
            requests: Vec::new(),
        }
    }

    pub fn host(&self) -> &H {
        &self.host
    }

    fn mint(&mut self, asker: &Asker, paired_by: &str) -> Grant {
        let now = self.host.now();
        if let Some(held) = self.grants.get(&asker.install_id)
            && held.caller == asker.caller
            && held.paired_by == paired_by
            && held.expires_at - now > GRANT_REUSE_MS
        {
            return Grant {
                code: held.code.clone(),
                port: self.host.port(),
                expires_at: held.expires_at,
            };
        }
        let fresh = HeldGrant {
            code: auth_store::mint_grant_code(),
            caller: asker.caller.clone(),
            expires_at: now + GRANT_TTL_MS,
            paired_by: paired_by.to_owned(),
        };
        log::log(&format!(
            "granted pairing to {} [{paired_by}]",
            label(asker.browser.as_deref(), &asker.caller, &asker.install_id)
        ));
        let grant = Grant {
            code: fresh.code.clone(),
            port: self.host.port(),
            expires_at: fresh.expires_at,
        };
        self.grants.insert(asker.install_id.clone(), fresh);
        grant
    }

    fn wait_on(&self, id: &str) -> PairWait {
        PairWait {
            request: id.to_owned(),
            approver: self.host.approver(),
            retry_after_ms: RETRY_AFTER_MS,
        }
    }

    fn request(&mut self, asker: &Asker) -> PairWait {
        let now = self.host.now();
        if let Some((_, known)) = self
            .requests
            .iter_mut()
            .find(|(id, _)| *id == asker.install_id)
            && now - known.summary.last_ask_at < REQUEST_TTL_MS
        {
            known.summary.last_ask_at = now;
            let id = known.summary.id.clone();
            return self.wait_on(&id);
        }
        let created = PendingRequest {
            summary: PairRequestSummary {
                id: rand::base64url(6),
                install_id: asker.install_id.clone(),
                browser: asker.browser.clone(),
                source: source_of(&asker.caller).to_owned(),
                extension_version: asker.extension_version.clone(),
                first_at: now,
                last_ask_at: now,
                nonce: rand::base64url(16),
            },
            caller: asker.caller.clone(),
        };
        let id = created.summary.id.clone();
        self.requests.retain(|(key, _)| *key != asker.install_id);
        self.requests.push((asker.install_id.clone(), created));
        log::log(&format!(
            "pairing request from {}",
            label(asker.browser.as_deref(), &asker.caller, &asker.install_id)
        ));
        self.host.changed();
        self.wait_on(&id)
    }

    pub fn ask(&mut self, asker: &Asker) -> GrantOutcome {
        let mode = self.host.mode();
        if mode == "code" {
            return GrantOutcome::Refused {
                refused: "off".into(),
                error: REFUSED_OFF.into(),
            };
        }
        let who = label(asker.browser.as_deref(), &asker.caller, &asker.install_id);
        let block = auth_store::block_of(&asker.install_id);
        if let Some(block) = &block {
            let reask_too_soon = block.reason == "denied"
                && parse_iso_ms(&block.at)
                    .is_none_or(|at| self.host.now() - at < REASK_AFTER_DENY_MS);
            if !asker.gesture || (mode == "ask" && reask_too_soon) {
                log::log(&format!("refused pairing for {who} ({})", block.reason));
                return GrantOutcome::Refused {
                    refused: block.reason.clone(),
                    error: refused_words(&block.reason).into(),
                };
            }
            if mode == "auto" {
                auth_store::unblock_install(&asker.install_id);
                log::log(&format!(
                    "{who} reconnected from the browser, lifting the {} block",
                    block.reason
                ));
            }
        }
        if mode == "ask" && (block.is_some() || !auth_store::is_approved(&asker.install_id)) {
            return GrantOutcome::Wait {
                wait: self.request(asker),
            };
        }
        let grant = self.mint(asker, if mode == "ask" { "approved" } else { "auto" });
        GrantOutcome::Grant {
            grant,
            approver: self.host.approver(),
        }
    }

    /// A grant is checked again when it is redeemed.
    fn still_granted(&self, install_id: &str) -> bool {
        let mode = self.host.mode();
        if mode == "code" || auth_store::block_of(install_id).is_some() {
            return false;
        }
        mode == "auto" || auth_store::is_approved(install_id)
    }

    /// Allow or deny a request (by request id or install id) or a blocked browser.
    pub fn decide(&mut self, reference: &str, allow: bool, name: Option<&str>) -> bool {
        let asked = self
            .requests
            .iter()
            .find(|(_, request)| {
                request.summary.id == reference || request.summary.install_id == reference
            })
            .map(|(_, request)| (request.summary.clone(), request.caller.clone()));
        let install_id = asked.as_ref().map_or(reference.to_owned(), |(summary, _)| {
            summary.install_id.clone()
        });
        let block = auth_store::block_of(&install_id);
        let (origin, browser) = match (&asked, &block) {
            (Some((summary, caller)), _) => (caller.clone(), summary.browser.clone()),
            (None, Some(block)) => (block.origin.clone(), block.browser.clone()),
            (None, None) => return false,
        };
        let browser = browser.or_else(|| block.as_ref().and_then(|b| b.browser.clone()));
        if allow {
            let named = name
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .map(|name| vexdom_pure::text::utf16_prefix(name, 40).to_owned());
            auth_store::approve_install(&install_id, &origin, browser.as_deref(), named.as_deref());
        } else {
            auth_store::block_install(&install_id, &origin, browser.as_deref(), "denied");
        }
        log::log(&format!(
            "{} {}",
            if allow { "approved" } else { "denied" },
            label(browser.as_deref(), &origin, &install_id)
        ));
        self.requests.retain(|(key, _)| *key != install_id);
        self.host.wake(&install_id);
        self.host.changed();
        true
    }

    pub fn pending(&mut self) -> Vec<PairRequestSummary> {
        let now = self.host.now();
        self.requests
            .retain(|(_, request)| now - request.summary.last_ask_at < REQUEST_TTL_MS);
        self.requests
            .iter()
            .map(|(_, request)| request.summary.clone())
            .collect()
    }

    /// The live grant that browser may redeem from that socket origin.
    pub fn grant_for(&mut self, install_id: &str, origin: &str) -> Option<HeldGrant> {
        let held = self.grants.get(install_id)?.clone();
        if held.expires_at <= self.host.now() {
            return None;
        }
        let short: String = install_id.chars().take(8).collect();
        if !self.still_granted(install_id) {
            self.grants.remove(install_id);
            log::log(&format!(
                "dropped the grant for {short}: the pairing mode, an approval or a block changed since"
            ));
            return None;
        }
        if caller_matches(&held.caller, origin) {
            return Some(held);
        }
        log::log(&format!(
            "a grant for {short} was offered from {origin}, not the origin it was given to; refused"
        ));
        None
    }

    /// That browser paired: the grant is spent and its request answered.
    pub fn consume(&mut self, install_id: &str) {
        self.grants.remove(install_id);
        let before = self.requests.len();
        self.requests.retain(|(key, _)| key != install_id);
        if self.requests.len() == before {
            return;
        }
        self.host.wake(install_id);
        self.host.changed();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth_store::tests::scratch_home;
    use std::cell::{Cell, RefCell};

    #[derive(Default)]
    struct Host {
        mode: RefCell<String>,
        changed: Cell<u32>,
        woken: RefCell<Vec<String>>,
    }

    impl DeskHost for &Host {
        fn port(&self) -> u16 {
            8785
        }
        fn mode(&self) -> String {
            self.mode.borrow().clone()
        }
        fn approver(&self) -> String {
            "app".into()
        }
        fn changed(&self) {
            self.changed.set(self.changed.get() + 1);
        }
        fn wake(&self, install_id: &str) {
            self.woken.borrow_mut().push(install_id.into());
        }
    }

    fn asker(gesture: bool) -> Asker {
        Asker {
            install_id: "install-cccccccccccc".into(),
            caller: "chrome-extension://abc/".into(),
            browser: Some("Chrome".into()),
            extension_version: "0.9.2".into(),
            gesture,
        }
    }

    #[test]
    fn auto_grants_and_reuses() {
        let _home = scratch_home("desk-auto");
        let host = Host {
            mode: RefCell::new("auto".into()),
            ..Host::default()
        };
        let mut desk = PairingDesk::new(&host);
        let GrantOutcome::Grant { grant, approver } = desk.ask(&asker(false)) else {
            panic!("expected a grant");
        };
        assert_eq!(
            (grant.code.len(), grant.port, approver.as_str()),
            (26, 8785, "app")
        );
        let GrantOutcome::Grant { grant: again, .. } = desk.ask(&asker(false)) else {
            panic!("expected a grant");
        };
        assert_eq!(again.code, grant.code);
        assert!(
            desk.grant_for("install-cccccccccccc", "chrome-extension://abc")
                .is_some()
        );
        assert!(
            desk.grant_for("install-cccccccccccc", "chrome-extension://xyz")
                .is_none()
        );
        *host.mode.borrow_mut() = "code".into();
        assert!(
            desk.grant_for("install-cccccccccccc", "chrome-extension://abc")
                .is_none()
        );
        assert_eq!(
            desk.ask(&asker(false)),
            GrantOutcome::Refused {
                refused: "off".into(),
                error: REFUSED_OFF.into()
            }
        );
    }

    #[test]
    fn ask_mode_waits_then_decides() {
        let _home = scratch_home("desk-ask");
        let host = Host {
            mode: RefCell::new("ask".into()),
            ..Host::default()
        };
        let mut desk = PairingDesk::new(&host);
        let GrantOutcome::Wait { wait } = desk.ask(&asker(false)) else {
            panic!("expected a wait");
        };
        assert_eq!(wait.retry_after_ms, 3000);
        assert_eq!(host.changed.get(), 1);
        assert_eq!(desk.pending().len(), 1);
        assert!(desk.decide(&wait.request, false, None));
        assert_eq!(host.woken.borrow().as_slice(), ["install-cccccccccccc"]);
        // Denied a moment ago: even a click is refused in ask mode.
        assert!(
            matches!(desk.ask(&asker(true)), GrantOutcome::Refused { refused, .. } if refused == "denied")
        );
        assert!(desk.decide("install-cccccccccccc", true, Some("  Laptop ")));
        assert!(matches!(
            desk.ask(&asker(false)),
            GrantOutcome::Grant { .. }
        ));
        assert!(!desk.decide("unknown", true, None));
    }

    #[test]
    fn auto_mode_gesture_lifts_a_block() {
        let _home = scratch_home("desk-block");
        let host = Host {
            mode: RefCell::new("auto".into()),
            ..Host::default()
        };
        auth_store::block_install(
            "install-cccccccccccc",
            "chrome-extension://abc",
            None,
            "revoked",
        );
        let mut desk = PairingDesk::new(&host);
        assert!(
            matches!(desk.ask(&asker(false)), GrantOutcome::Refused { refused, .. } if refused == "revoked")
        );
        assert!(matches!(desk.ask(&asker(true)), GrantOutcome::Grant { .. }));
        assert!(auth_store::block_of("install-cccccccccccc").is_none());
    }

    #[test]
    fn callers() {
        assert!(caller_matches("moz-extension://", "moz-extension://uuid"));
        assert!(caller_matches(
            "chrome-extension://abc/",
            "chrome-extension://abc"
        ));
        assert!(!caller_matches(
            "chrome-extension://abc/",
            "chrome-extension://abd"
        ));
    }
}
