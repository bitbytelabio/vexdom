//! `vexdom-pure` for the extension (P9): the handshake crypto, secret detection, sealing and the
//! secret vault, intent routing, the blocked-sites list, schedule rules, profile and
//! recording-workflow validation, so the extension and the daemon run one
//! implementation. All synchronous. JSON documents cross as text in their TypeScript shapes
//! (ADR-025 payload rule); times are JavaScript epoch milliseconds; offsets are UTF-16 code units.

use std::cell::RefCell;

use boltffi::{data, export};
use jiff::tz::TimeZone;
use serde_json::{Value, json};
use vexdom_pure::seal::vault::{self, Vault};
use vexdom_pure::{blocked_sites, handshake, intent, profile, schedules, seal, secrets, workflow};

/// `Transcript` in `src/lib/actions/handshake.ts`.
#[data]
#[derive(Clone, Debug)]
pub struct HandshakeTranscript {
    pub protocol_version: u32,
    pub extension_version: String,
    pub manifest_hash: String,
    pub client_nonce: String,
    pub server_nonce: String,
}

/// `Welcome` in `src/lib/actions/handshake.ts`.
#[data]
#[derive(Clone, Debug)]
pub struct HandshakeWelcome {
    pub daemon_version: String,
    pub manifest_hash: String,
    pub manifest_in_sync: bool,
    pub sealed_session_key: Option<String>,
}

impl From<HandshakeTranscript> for handshake::Transcript {
    fn from(t: HandshakeTranscript) -> Self {
        Self {
            protocol_version: t.protocol_version,
            extension_version: t.extension_version,
            manifest_hash: t.manifest_hash,
            client_nonce: t.client_nonce,
            server_nonce: t.server_nonce,
        }
    }
}

#[export]
pub fn handshake_pairing_secret(code: String, t: HandshakeTranscript) -> String {
    handshake::pairing_secret(&code, &t.into())
}

/// `None` for an empty secret, which WebCrypto refuses as a key.
#[export]
pub fn handshake_client_proof(secret: String, t: HandshakeTranscript) -> Option<String> {
    handshake::client_proof(&secret, &t.into()).ok()
}

#[export]
pub fn handshake_server_proof(
    secret: String,
    t: HandshakeTranscript,
    w: HandshakeWelcome,
) -> Option<String> {
    handshake::server_proof(
        &secret,
        &t.into(),
        &w.daemon_version,
        &w.manifest_hash,
        w.manifest_in_sync,
        w.sealed_session_key.as_deref(),
    )
    .ok()
}

#[export]
pub fn handshake_seal_session_key(
    secret: String,
    t: HandshakeTranscript,
    key: String,
) -> Option<String> {
    handshake::seal_session_key(&secret, &t.into(), &key).ok()
}

#[export]
pub fn handshake_open_session_key(
    secret: String,
    t: HandshakeTranscript,
    sealed: String,
) -> Option<String> {
    handshake::open_session_key(&secret, &t.into(), &sealed)
}

/// An immune range for `secrets_find`, in UTF-16 code units.
#[data]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SecretRange {
    pub start: u32,
    pub end: u32,
}

/// A found secret, less its `value` (`text.slice(start, end)`), in UTF-16 code units.
#[data]
#[derive(Clone, Debug, PartialEq)]
pub struct SecretSpan {
    pub start: u32,
    pub end: u32,
    pub kind: String,
    pub shape: String,
    pub reveal_head: u32,
    pub reveal_tail: u32,
}

/// UTF-8 byte offset ↔ UTF-16 code unit offset, for one string.
struct Offsets {
    /// `(byte, unit)` at every char boundary, in order, ending at the string's end.
    marks: Vec<(usize, usize)>,
}

impl Offsets {
    fn new(text: &str) -> Self {
        let mut marks = Vec::with_capacity(text.len() + 1);
        let mut unit = 0;
        for (byte, c) in text.char_indices() {
            marks.push((byte, unit));
            unit += c.len_utf16();
        }
        marks.push((text.len(), unit));
        Self { marks }
    }

    fn unit(&self, byte: usize) -> u32 {
        let at = self.marks.partition_point(|(b, _)| *b < byte);
        self.marks.get(at).map_or(0, |(_, unit)| *unit as u32)
    }

    /// The byte offset of a UTF-16 offset; one inside a surrogate pair rounds up to the char's end.
    fn byte(&self, unit: u32) -> usize {
        let unit = unit as usize;
        let at = self.marks.partition_point(|(_, u)| *u < unit);
        self.marks
            .get(at)
            .or(self.marks.last())
            .map_or(0, |(byte, _)| *byte)
    }
}

#[export]
pub fn secrets_find(text: String, immune: Vec<SecretRange>) -> Vec<SecretSpan> {
    let offsets = Offsets::new(&text);
    let immune: Vec<(usize, usize)> = immune
        .iter()
        .map(|range| (offsets.byte(range.start), offsets.byte(range.end)))
        .collect();
    secrets::find_secrets(&text, &immune)
        .into_iter()
        .map(|span| SecretSpan {
            start: offsets.unit(span.start),
            end: offsets.unit(span.end),
            kind: span.kind.to_owned(),
            shape: span.shape.to_owned(),
            reveal_head: span.reveal.head as u32,
            reveal_tail: span.reveal.tail as u32,
        })
        .collect()
}

#[export]
pub fn secrets_kind_for_key(key: String) -> Option<String> {
    secrets::kind_for_key(&key).map(str::to_owned)
}

#[export]
pub fn secrets_looks_like_credential(value: String) -> bool {
    secrets::looks_like_credential(&value)
}

#[export]
pub fn secrets_not_a_placeholder(value: String) -> bool {
    secrets::not_a_placeholder(&value)
}

#[export]
pub fn secrets_case_flips(value: String) -> f64 {
    secrets::case_flips(&value)
}

#[export]
pub fn secrets_entropy(value: String) -> f64 {
    secrets::entropy(&value)
}

/// `Handle` in `src/lib/bridge/pure.ts`: a sealed handle found in text.
#[data]
#[derive(Clone, Debug, PartialEq)]
pub struct SealHandle {
    pub kind: String,
    pub id: String,
    pub origin: Option<String>,
    pub tag: String,
    pub text: String,
}

#[export]
pub fn seal_handles_in(text: String) -> Vec<SealHandle> {
    seal::handles_in(&text, None)
        .into_iter()
        .map(|handle| SealHandle {
            kind: handle.kind,
            id: handle.id,
            origin: handle.origin,
            tag: handle.tag,
            text: handle.text,
        })
        .collect()
}

/// A vault as `secret-vault.ts` stores it; text that is not one is an empty vault with `tag`.
fn read_vault(text: &str, tag: &str) -> (Vault, bool) {
    match serde_json::from_str::<Vault>(text) {
        Ok(vault) if !vault.tag.is_empty() => (vault, false),
        _ => (
            Vault {
                tag: tag.to_owned(),
                entries: Vec::new(),
            },
            true,
        ),
    }
}

fn vault_text(vault: &Vault) -> String {
    serde_json::to_string(vault).expect("a vault serializes")
}

/// `vault` is set only when it changed and must be written back.
#[data]
#[derive(Clone, Debug, PartialEq)]
pub struct VaultSealed {
    pub vault: Option<String>,
    pub value: String,
}

/// Seals a value read from a page (JSON text) with the vault's tag, minting ids from `id_seed`.
#[export]
pub fn vault_seal(
    vault: String,
    value: String,
    origin: Option<String>,
    now: f64,
    id_seed: String,
) -> VaultSealed {
    let (mut held, fresh) = read_vault(&vault, &id_seed);
    let (sealed, changed) =
        vault::seal_for_page(&mut held, &parse(&value), origin.as_deref(), now, &id_seed);
    VaultSealed {
        vault: (changed || fresh).then(|| vault_text(&held)),
        value: sealed.to_string(),
    }
}

/// `vault` is set only when an entry was touched and must be written back.
#[data]
#[derive(Clone, Debug, PartialEq)]
pub struct VaultReleased {
    pub vault: Option<String>,
    pub input: String,
    pub released: u32,
    pub refused: u32,
    pub unresolved: u32,
}

/// Releases the vault's handles in an action's input (JSON text) where the action may carry them.
#[export]
pub fn vault_release(vault: String, action: String, input: String, now: f64) -> VaultReleased {
    let (mut held, _) = read_vault(&vault, "");
    let input = serde_json::from_str::<Value>(&input).unwrap_or(Value::Null);
    let (release, touched) = vault::release_for_action(&mut held, &action, &input, now);
    VaultReleased {
        vault: touched.then(|| vault_text(&held)),
        input: release.input.to_string(),
        released: release.released as u32,
        refused: release.refused as u32,
        unresolved: release.unresolved as u32,
    }
}

/// An IANA name; one this build does not know is UTC.
fn zone(name: &str) -> TimeZone {
    TimeZone::get(name).unwrap_or(TimeZone::UTC)
}

/// Text that is not JSON reads as `null`, which every rule function treats as nothing.
fn parse(text: &str) -> Value {
    serde_json::from_str(text).unwrap_or(Value::Null)
}

fn parse_stored(text: Option<String>) -> Option<Value> {
    text.map(|text| parse(&text))
        .filter(|value| !value.is_null())
}

#[export]
pub fn schedule_describe_rule(rule: String, tz: String) -> String {
    schedules::describe_rule(&parse(&rule), &zone(&tz))
}

#[export]
pub fn schedule_describe_moment(at: f64, tz: String) -> String {
    schedules::describe_moment(at as i64, &zone(&tz))
}

#[export]
pub fn schedule_inside_window(at: f64, window: String, tz: String) -> bool {
    schedules::inside_window(at as i64, &parse(&window), &zone(&tz))
}

#[export]
pub fn schedule_next_after(rule: String, after: f64, tz: String) -> Option<f64> {
    schedules::next_after(&parse(&rule), after as i64, &zone(&tz)).map(|at| at as f64)
}

#[export]
pub fn schedule_upcoming(rule: String, from: f64, count: u32, tz: String) -> Vec<f64> {
    schedules::upcoming(&parse(&rule), from as i64, count as usize, &zone(&tz))
        .into_iter()
        .map(|at| at as f64)
        .collect()
}

/// `{"ok":true,"rule":…}` or `{"ok":false,"message":…}`.
#[export]
pub fn schedule_validate_rule(raw: String, now: f64, stored: Option<String>) -> String {
    let stored = parse_stored(stored);
    match schedules::validate_rule(&parse(&raw), now as i64, stored.as_ref()) {
        Ok(rule) => json!({"ok": true, "rule": rule}),
        Err(message) => json!({"ok": false, "message": message}),
    }
    .to_string()
}

/// `TaskValidation`: `{"ok":true,"draft":…}` or `{"ok":false,"field":…,"message":…}`.
#[export]
pub fn schedule_validate_task(input: String, now: f64, stored: Option<String>) -> String {
    let stored = parse_stored(stored);
    match schedules::validate_task(&parse(&input), now as i64, stored.as_ref()) {
        Ok(draft) => json!({"ok": true, "draft": draft}),
        Err((field, message)) => json!({"ok": false, "field": field, "message": message}),
    }
    .to_string()
}

#[export]
pub fn schedule_next_run_for(task: String, after: f64, tz: String) -> Option<f64> {
    schedules::next_run_for(&parse(&task), after as i64, &zone(&tz)).map(|at| at as f64)
}

#[export]
pub fn schedule_order_for(task: String, run_id: String) -> String {
    schedules::order_for(&parse(&task), &run_id).to_string()
}

#[export]
pub fn schedule_headline_of(text: String) -> Option<String> {
    schedules::headline_of(&text)
}

#[export]
pub fn profile_is_user(value: String) -> bool {
    profile::is_user_profile(&parse(&value))
}

#[export]
pub fn profile_as_user(value: String) -> String {
    json!(profile::as_user_profile(&parse(&value))).to_string()
}

/// A `UserProfile` as the settings page holds it, limits not checked; anything else as
/// `asUserProfile` reads it.
fn user_profile(text: &str) -> profile::UserProfile {
    serde_json::from_str(text).unwrap_or_else(|_| profile::as_user_profile(&parse(text)))
}

#[export]
pub fn profile_normalize(profile: String) -> String {
    json!(user_profile(&profile).normalized()).to_string()
}

#[export]
pub fn profile_is_empty(profile: String) -> bool {
    user_profile(&profile).is_empty()
}

/// `WorkflowValidation`: `{"ok":true,"workflow":…,"warnings":[…]}` or `{"ok":false,"message":…}`.
#[export]
pub fn workflow_validate(input: String, origin: String) -> String {
    match workflow::validate_recording_workflow(&parse(&input), &origin) {
        Ok(checked) => {
            json!({"ok": true, "workflow": checked.workflow, "warnings": checked.warnings})
        }
        Err(message) => json!({"ok": false, "message": message}),
    }
    .to_string()
}

/// The TypeScript `Routing` of one instruction, as JSON.
#[export]
pub fn intent_route(input: String) -> String {
    intent::route_intent(&input).to_string()
}

#[export]
pub fn intent_act_threshold() -> f64 {
    intent::ACT_THRESHOLD
}

/// `ParsedPattern`: `{"ok":true,"pattern":…,"source":…}` or `{"ok":false,"reason":…}`.
#[export]
pub fn blocked_sites_parse(entry: String) -> String {
    match blocked_sites::parse_pattern(&entry) {
        Ok((pattern, source)) => json!({"ok": true, "pattern": pattern, "source": source}),
        Err(reason) => json!({"ok": false, "reason": reason}),
    }
    .to_string()
}

#[export]
pub fn blocked_sites_describe(pattern: String) -> String {
    blocked_sites::describe_pattern(&pattern)
}

#[export]
pub fn blocked_sites_subject(url: String) -> Option<String> {
    blocked_sites::subject_of(&url)
}

thread_local! {
    /// The last list compiled, keyed by its JSON text: a list is compiled once per change, not once
    /// per action.
    static COMPILED: RefCell<Option<(String, Option<Vec<blocked_sites::Compiled>>)>> =
        const { RefCell::new(None) };
}

/// Runs `f` on the compiled list for `raw` (JSON text); `None` when it is not a list.
fn with_list<T>(raw: &str, f: impl FnOnce(Option<&[blocked_sites::Compiled]>) -> T) -> T {
    COMPILED.with(|cell| {
        let mut cached = cell.borrow_mut();
        if cached.as_ref().is_none_or(|(text, _)| text != raw) {
            *cached = Some((raw.to_owned(), blocked_sites::compile(&parse(raw))));
        }
        let (_, list) = cached.as_ref().expect("filled above");
        f(list.as_deref())
    })
}

/// False for a stored value that is not a list, which the caller must refuse rather than guess.
#[export]
pub fn blocked_sites_valid(raw: String) -> bool {
    with_list(&raw, |list| list.is_some())
}

#[export]
pub fn blocked_sites_blocked_by(raw: String, url: String) -> Option<String> {
    with_list(&raw, |list| {
        blocked_sites::blocked_by(&url, list?).map(str::to_owned)
    })
}

/// The regex sources of the patterns that could match somewhere on `origin`.
#[export]
pub fn blocked_sites_patterns_for(raw: String, origin: String) -> Vec<String> {
    with_list(&raw, |list| {
        blocked_sites::patterns_for(&origin, list.unwrap_or_default())
            .into_iter()
            .map(|entry| entry.source.clone())
            .collect()
    })
}
