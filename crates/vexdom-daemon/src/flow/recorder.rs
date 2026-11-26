//! The recorder: what a tab's flow recording keeps of the page actions the Bridge saw.
//!
//! agent-browser records every command a `--capture flow` session runs and tags each request
//! to the command that was running when it started. Here a recording belongs to one tab: the
//! extension keeps the tab's HAR, and the Bridge logs each external page action after it ran,
//! with when it started and ended. A request belongs to the command whose window
//! `[startedAt, endedAt + 1.5 s]` holds its start, unless another command began since.

use std::collections::HashMap;

use serde_json::{Value, json};
use vexdom_core::guardrails::secrets::handles_in;
use vexdom_core::tools::{self, READ_ONLY_ACTIONS};

use super::{
    HarEntry, HeaderClass, HostPolicy, SaveInput, action_of, build_flow, carries_secret,
    classify_header, describe_command, dropped_name, may_become_candidate, save_start_url,
    write_flow_in,
};
use crate::Daemon;

/// A request that starts this long after its command ended still belongs to it.
const TAG_GRACE_MS: f64 = 1500.0;

pub(crate) const REDACTED: &str = "<redacted>";

/// Page actions a recording leaves out, beyond the reads (`READ_ONLY_ACTIONS`, waits aside):
/// the capture's own tools, session setup that may carry credentials (cookies, storage,
/// emulated headers and logins, routes, init scripts), and tools that only look at the page.
/// Requests they send belong to no step.
const NOT_RECORDED: &[&str] = &[
    "page.harStart",
    "page.harStop",
    "page.cookies",
    "page.storage",
    "page.emulate",
    "page.routeNetwork",
    "page.unrouteNetwork",
    "page.initScript",
    "page.startDiagnostics",
    "page.stopDiagnostics",
    "page.screenshot",
    "page.highlightElement",
    "page.pickElement",
    "page.checkSites",
    "page.findCaptcha",
    "page.findProgress",
    "page.findSearch",
    "page.startMonitor",
    "page.stopMonitor",
    "page.awaitMonitor",
    "page.startTimer",
    "page.stopTimer",
    "page.traceStart",
    "page.traceStop",
    "page.pdf",
    "page.recordStart",
    "page.recordStop",
    "page.react",
    "page.tabKeep",
];

/// Input keys that name an element; a ref in one is stored as its role and name.
const TARGET_KEYS: &[&str] = &["target", "from", "to", "frame", "container"];

fn is_recordable(action: &str) -> bool {
    action.starts_with("page.")
        && !NOT_RECORDED.contains(&action)
        && (!READ_ONLY_ACTIONS.contains(&action)
            || matches!(action, "page.waitFor" | "page.waitForElement"))
}

/// What the recorder knows about a step besides its command.
#[derive(Clone, Default)]
pub(crate) struct StepMeta {
    pub ok: bool,
    /// The tab's address after the step, read once it finished.
    pub url: Option<String>,
}

/// A command's time in the tab: `seq` is `None` for a command that is not recorded.
#[derive(Clone, Copy, Debug)]
struct Window {
    start: f64,
    end: Option<f64>,
    seq: Option<u64>,
}

/// What the page says about the field a command types into, as far as its target tells.
#[derive(Clone, Default, Debug)]
pub(crate) struct FieldTraits {
    /// The `type` property: `password`, `text`, `email`, ...
    pub input_type: String,
    /// The `autocomplete` attribute.
    pub autocomplete: String,
    /// Label, accessible name, placeholder, test id and selector, space separated.
    pub hint: String,
}

impl FieldTraits {
    /// What a command's `target` says about its element.
    fn of_target(target: &Value) -> Option<FieldTraits> {
        let target = target.get("ref").unwrap_or(target);
        let words: Vec<&str> = [
            "label",
            "name",
            "placeholder",
            "testId",
            "title",
            "selector",
        ]
        .iter()
        .filter_map(|k| target.get(*k).and_then(Value::as_str))
        .collect();
        let hint = match target {
            Value::String(selector) => selector.clone(),
            _ => words.join(" "),
        };
        (!hint.is_empty()).then(|| FieldTraits {
            hint,
            ..FieldTraits::default()
        })
    }

    /// A credential field by its type or autocomplete, or by what it is called.
    pub(crate) fn looks_secret(&self) -> bool {
        self.is_secret() || has_secret_marker(&self.hint)
    }

    /// Fields whose value is a credential, whatever the page calls them.
    fn is_secret(&self) -> bool {
        let ac = self.autocomplete.trim().to_ascii_lowercase();
        self.input_type.eq_ignore_ascii_case("password")
            || ac.split_whitespace().any(|t| {
                matches!(t, "current-password" | "new-password" | "one-time-code")
                    || t.starts_with("cc-")
            })
    }
}

fn has_secret_marker(text: &str) -> bool {
    let text = text.to_lowercase();
    SECRET_MARKERS.iter().any(|m| text.contains(m))
}

pub(crate) const SECRET_MARKERS: &[&str] = &[
    "pass",
    "contraseña",
    "secret",
    "token",
    "otp",
    "pin",
    "cvv",
    "card",
];

/// What a save needs besides the recording.
pub(crate) struct SaveArgs<'a> {
    pub name: &'a str,
    pub from: Option<u64>,
    pub to: Option<u64>,
    pub description: Option<&'a str>,
    pub params: &'a [(String, String)],
    pub output: Option<&'a str>,
    pub entries: &'a [HarEntry],
    /// The page shows a password field at save time.
    pub login_wall: bool,
}

/// The commands and request tags of one tab's recording.
#[derive(Default)]
pub(crate) struct FlowRecorder {
    capturing: bool,
    pub(crate) commands: Vec<(u64, Value)>,
    pub(crate) meta: HashMap<u64, StepMeta>,
    /// HAR request id to the step that was running when the request started.
    pub(crate) request_tags: HashMap<String, u64>,
    windows: Vec<Window>,
    /// Steps that put a value of unknown content into a secret field.
    pub(crate) unknown_secret_seqs: Vec<u64>,
    /// Keys pressed so far in the secret field that has focus.
    key_buffer: String,
    /// The last entry of `secrets` is the current `key_buffer`.
    key_active: bool,
    /// Modifier bitmask held by `pressKey` down phases.
    held_modifiers: i32,
    /// The field the last step aimed at, which holds the focus after it.
    focused: Option<FieldTraits>,
    /// Values typed into secret fields, kept in memory only, so a save can
    /// drop any captured request that carries one.
    pub(crate) secrets: Vec<String>,
    /// The tab's address when the recording began, for flows that start on an open page.
    pub(crate) page_url: Option<String>,
    /// Values typed into fields that are not secrets, as (param name, value, seq).
    pub(crate) typed: Vec<(String, String, u64)>,
    /// Sequence number of the last step covered by a save.
    pub(crate) last_save_seq: u64,
    /// Flows saved from this recording, as (origin, name).
    pub(crate) saved: Vec<(String, String)>,
    next_seq: u64,
}

impl FlowRecorder {
    pub(crate) fn active(&self) -> bool {
        self.capturing
    }

    pub(crate) fn begin_capture(&mut self, page_url: Option<String>) {
        *self = FlowRecorder {
            capturing: true,
            page_url,
            ..FlowRecorder::default()
        };
    }

    /// Stops logging; what was recorded stays for `flowSteps` and `flowSave`.
    pub(crate) fn end_capture(&mut self) -> usize {
        self.capturing = false;
        self.reset_keys();
        self.unsaved_steps()
    }

    /// A page action that ran from `started` to `ended` (epoch ms): recorded as a step, or noted
    /// as a window whose requests belong to no step.
    pub(crate) fn record(
        &mut self,
        action: &str,
        input: &Value,
        started: f64,
        ended: f64,
        ok: bool,
    ) -> Option<u64> {
        if !self.capturing {
            return None;
        }
        if !is_recordable(action) {
            self.untag_between(started, ended);
            return None;
        }
        let seq = self.begin_command_at(started, action, &stable_command(action, input))?;
        self.finish_command(seq, ok, ended);
        Some(seq)
    }

    /// Record a command (`{"do": ...}`) and return its step number.
    pub(crate) fn begin_command_at(&mut self, at: f64, action: &str, cmd: &Value) -> Option<u64> {
        if !self.capturing || !is_recordable(action) {
            return None;
        }
        self.next_seq += 1;
        let mut recorded = cmd.clone();
        let short = action_of(&recorded).to_string();
        let target_traits = recorded.get("target").and_then(FieldTraits::of_target);
        if target_traits.is_some() {
            self.focused = target_traits.clone();
        }
        let traits = target_traits.or_else(|| self.focused.clone());
        let is_key = short == "pressKey";
        if !is_key {
            self.reset_keys();
        }
        self.secrets.extend(redact_headers(&mut recorded));
        let (redacted, secret) = redact_secret(&mut recorded, traits.as_ref());
        if redacted {
            if let Some(secret) = secret {
                self.secrets.push(secret);
            } else {
                // A sealed value: the browser knows it, the Bridge never does.
                self.unknown_secret_seqs.push(self.next_seq);
            }
        } else if is_key {
            self.record_key(&mut recorded, traits.as_ref());
        } else if short == "clipboard" {
            self.record_clipboard(&mut recorded, traits.as_ref());
        } else if let Some(field) = typed_field(&recorded) {
            self.keep_typed_as_param(&mut recorded, field, traits.as_ref());
        }
        self.commands.push((self.next_seq, recorded));
        self.meta.insert(self.next_seq, StepMeta::default());
        self.windows.push(Window {
            start: at,
            end: None,
            seq: Some(self.next_seq),
        });
        Some(self.next_seq)
    }

    /// The step finished at `end`.
    pub(crate) fn finish_command(&mut self, seq: u64, ok: bool, end: f64) {
        if let Some(m) = self.meta.get_mut(&seq) {
            m.ok = ok;
        }
        if let Some(window) = self.windows.iter_mut().rev().find(|w| w.seq == Some(seq)) {
            window.end = Some(end);
        }
    }

    /// The tab's address after step `seq`.
    pub(crate) fn set_url(&mut self, seq: u64, url: Option<String>) {
        if let Some(m) = self.meta.get_mut(&seq) {
            m.url = url;
        }
    }

    /// The page a step ran on: the address after the step before it, else the recording's start.
    pub(crate) fn url_before(&self, seq: u64) -> Option<String> {
        let before = self
            .commands
            .iter()
            .rev()
            .map(|(s, _)| *s)
            .find(|s| *s < seq);
        match before {
            Some(prev) => self
                .meta
                .get(&prev)
                .and_then(|m| m.url.clone())
                .or_else(|| self.url_before(prev)),
            None => self.page_url.clone(),
        }
    }

    /// Something that is not a step ran (or a flow run sent requests from the tab): requests
    /// that start meanwhile belong to no step.
    pub(crate) fn untag_between(&mut self, start: f64, end: f64) {
        self.windows.push(Window {
            start,
            end: Some(end),
            seq: None,
        });
        self.reset_keys();
    }

    /// A request that started at `started_ms` belongs to the step whose window holds it.
    pub(crate) fn tag(&mut self, request_id: &str, started_ms: f64) {
        let window = self
            .windows
            .iter()
            .filter(|w| w.start <= started_ms)
            .max_by(|a, b| a.start.total_cmp(&b.start));
        let Some(Window {
            end,
            seq: Some(seq),
            ..
        }) = window.copied()
        else {
            return;
        };
        if end.is_none_or(|end| started_ms <= end + TAG_GRACE_MS) {
            self.request_tags.insert(request_id.to_string(), seq);
        }
    }

    /// The recorded steps, as `flowSteps` shows them.
    pub(crate) fn steps(&self) -> Value {
        let steps: Vec<Value> = self
            .commands
            .iter()
            .map(|(seq, c)| {
                json!({
                    "seq": seq,
                    "command": describe_command(c),
                    "url": self.meta.get(seq).and_then(|m| m.url.clone()),
                    "requests": self.request_tags.values().filter(|s| *s == seq).count(),
                    "ok": self.is_ok(*seq),
                    "saved": *seq <= self.last_save_seq,
                })
            })
            .collect();
        json!({
            "steps": steps,
            "lastSave": self.last_save_seq,
            "unsaved": self.unsaved_steps(),
        })
    }

    /// Save a range of the recording as a flow and write it under `dir`.
    pub(crate) fn save(
        &mut self,
        dir: &std::path::Path,
        args: SaveArgs<'_>,
    ) -> Result<Value, String> {
        let (first, last) = self.resolve_range(args.from, args.to)?;
        let commands: Vec<(u64, Value)> = self
            .commands
            .iter()
            .filter(|(seq, _)| *seq >= first && *seq <= last)
            .cloned()
            .collect();
        let (flow, mut report) = build_flow(SaveInput {
            name: args.name,
            recorder: self,
            commands: &commands,
            description: args.description,
            entries: args.entries,
            params: args.params,
            output: args.output,
            created_at: vexdom_core::log::iso_now(),
            login_wall: args.login_wall,
        })?;
        let path = write_flow_in(dir, &flow)?;
        self.mark_saved(last);
        self.saved
            .push((flow.origin.clone(), args.name.to_string()));
        report["path"] = json!(path.to_string_lossy());
        report["from"] = json!(first);
        report["to"] = json!(last);
        Ok(report)
    }

    pub(crate) fn unsaved_steps(&self) -> usize {
        self.commands
            .iter()
            .filter(|(seq, _)| *seq > self.last_save_seq)
            .count()
    }

    /// The first and last step a save covers. Without `from` the save
    /// starts at the last navigation after the previous save.
    pub(crate) fn resolve_range(
        &self,
        from: Option<u64>,
        to: Option<u64>,
    ) -> Result<(u64, u64), String> {
        let last = self.next_seq;
        if last == 0 {
            return Err(
                "nothing recorded yet; run page actions while the recording runs, then `flow save`"
                    .to_string(),
            );
        }
        let to = to.unwrap_or(last);
        if to == 0 || to > last {
            return Err(format!("to {to} is outside the recorded steps (1-{last})"));
        }
        let from = from.unwrap_or_else(|| {
            self.commands
                .iter()
                .rev()
                .filter(|(seq, c)| {
                    *seq <= to && *seq > self.last_save_seq && action_of(c) == "navigate"
                })
                .map(|(seq, _)| *seq)
                .next()
                .unwrap_or(self.last_save_seq + 1)
        });
        if from == 0 || from > to {
            return Err(format!(
                "no recorded steps from {from} to {to}; run `flow steps` to see them"
            ));
        }
        Ok((from, to))
    }

    /// A gap for each request that could become a flow request, tagged to a
    /// step in the range, that had not finished loading when the HAR was read,
    /// so its response is not on the entry. Third-party and telemetry requests
    /// never count.
    pub(crate) fn loading_gaps(
        &self,
        from: Option<u64>,
        to: Option<u64>,
        entries: &[HarEntry],
    ) -> Vec<Value> {
        let Ok((first, last)) = self.resolve_range(from, to) else {
            return Vec::new();
        };
        let commands: Vec<(u64, Value)> = self
            .commands
            .iter()
            .filter(|(seq, _)| *seq >= first && *seq <= last)
            .cloned()
            .collect();
        let Some(start) = save_start_url(&commands, self) else {
            return Vec::new();
        };
        let policy = HostPolicy::from_start(&start);
        entries
            .iter()
            .filter(|e| !e.finished && may_become_candidate(e, &policy))
            .filter_map(|e| {
                let seq = *self.request_tags.get(&e.request_id)?;
                let index = commands.iter().position(|(s, _)| *s == seq)?;
                let known = carries_secret(e, &self.secrets);
                let label = if known || self.unknown_secret_seqs.contains(&seq) {
                    dropped_name(e, &self.secrets, known)
                } else {
                    let path = url::Url::parse(&e.url)
                        .map(|u| u.path().to_string())
                        .unwrap_or_else(|_| e.url.clone());
                    format!("{} {}", e.method, path)
                };
                Some(json!({
                    "step": index + 1,
                    "command": describe_command(&commands[index].1),
                    "reason": format!("still loading at save: {label}"),
                }))
            })
            .collect()
    }

    pub(crate) fn add_secret(&mut self, value: &str) {
        if !value.is_empty() && !self.secrets.iter().any(|s| s == value) {
            self.secrets.push(value.to_string());
        }
    }

    fn reset_keys(&mut self) {
        self.key_buffer.clear();
        self.key_active = false;
    }

    fn set_key_buffer(&mut self, buffer: String) {
        if self.key_active {
            self.secrets.pop();
        }
        self.key_buffer = buffer;
        self.key_active = !self.key_buffer.is_empty();
        if self.key_active {
            self.secrets.push(self.key_buffer.clone());
        }
    }

    /// A value typed into an ordinary field is saved as a param, so the flow
    /// file carries a placeholder in the step and the value only as the
    /// param's recorded example.
    fn keep_typed_as_param(&mut self, cmd: &mut Value, field: &str, traits: Option<&FieldTraits>) {
        let Some(value) = cmd[field]
            .as_str()
            .filter(|v| !v.is_empty())
            .map(String::from)
        else {
            return;
        };
        let name = match self.typed.iter().find(|(_, v, _)| *v == value) {
            Some((name, _, _)) => name.clone(),
            None => {
                let taken: Vec<String> = self.typed.iter().map(|(n, _, _)| n.clone()).collect();
                typed_param_name(traits, &taken)
            }
        };
        self.typed.push((name.clone(), value, self.next_seq));
        cmd[field] = json!(format!("{{{{{name}}}}}"));
    }

    /// A key sent to a secret field is part of a secret: it is redacted from
    /// the step and joined to the keys before it, so a request carrying the
    /// whole value is dropped at save.
    fn record_key(&mut self, cmd: &mut Value, traits: Option<&FieldTraits>) {
        let phase = cmd["phase"].as_str().unwrap_or("press").to_string();
        let Some(key) = cmd["key"].as_str().map(String::from) else {
            return;
        };
        let (actual_key, chord_mods) = parse_key_chord(&key);
        let own_bit = key_modifier_bit(&actual_key);
        if let Some(bit) = own_bit {
            match phase.as_str() {
                "down" => self.held_modifiers |= bit,
                "up" => self.held_modifiers &= !bit,
                _ => {}
            }
        }
        if phase != "press" && own_bit.is_some() {
            return;
        }
        if !traits.is_some_and(FieldTraits::looks_secret) {
            self.reset_keys();
            return;
        }
        let listed = cmd["modifiers"]
            .as_array()
            .map(|mods| {
                mods.iter()
                    .filter_map(Value::as_str)
                    .filter_map(key_modifier_bit)
                    .fold(0, |acc, bit| acc | bit)
            })
            .unwrap_or(0);
        let effective_mods = chord_mods.unwrap_or(0) | listed | self.held_modifiers;
        let resolved_key = resolved_key(&actual_key, effective_mods);
        let plain = effective_mods & (1 | 2 | 4) == 0;
        let shift = effective_mods & 8 != 0;
        let paste = ((effective_mods & (2 | 4) != 0) && resolved_key.eq_ignore_ascii_case("v"))
            || (shift && resolved_key.eq_ignore_ascii_case("insert"));
        let pressed = phase != "up";
        if plain && resolved_key.chars().count() == 1 {
            if pressed {
                let mut buffer = self.key_buffer.clone();
                buffer.push_str(&resolved_key);
                self.set_key_buffer(buffer);
            }
            cmd["key"] = json!(REDACTED);
            cmd["redacted"] = json!(true);
        } else if plain && resolved_key == "Backspace" {
            if pressed {
                let mut buffer = self.key_buffer.clone();
                buffer.pop();
                self.set_key_buffer(buffer);
            }
        } else if paste {
            // The pasted value is unknown, so the step is redacted and the
            // requests it tagged are not saved.
            if pressed {
                self.unknown_secret_seqs.push(self.next_seq);
                self.reset_keys();
            }
            cmd["key"] = json!(REDACTED);
            cmd["redacted"] = json!(true);
        } else if is_non_printable_control_key(&resolved_key) || own_bit.is_some() {
            if pressed {
                self.reset_keys();
            }
        } else {
            if pressed {
                self.reset_keys();
            }
            cmd["key"] = json!(REDACTED);
            cmd["redacted"] = json!(true);
        }
    }

    /// `clipboard write` text is a secret; a `clipboard paste` into a secret
    /// field puts an unknown value there.
    fn record_clipboard(&mut self, cmd: &mut Value, traits: Option<&FieldTraits>) {
        match cmd["op"].as_str() {
            Some("write") => {
                if let Some(text) = cmd["text"].as_str().map(String::from) {
                    cmd["text"] = json!(REDACTED);
                    cmd["redacted"] = json!(true);
                    self.add_secret(&text);
                }
            }
            Some("paste") if traits.is_some_and(FieldTraits::looks_secret) => {
                cmd["redacted"] = json!(true);
                self.unknown_secret_seqs.push(self.next_seq);
                self.reset_keys();
            }
            _ => {}
        }
    }

    pub(crate) fn mark_saved(&mut self, last: u64) {
        self.last_save_seq = self.last_save_seq.max(last);
    }

    pub(crate) fn is_ok(&self, seq: u64) -> bool {
        self.meta.get(&seq).is_some_and(|m| m.ok)
    }
}

/// `Control+a` as the key and its modifier bits (Alt 1, Control 2, Meta 4, Shift 8).
fn parse_key_chord(input: &str) -> (String, Option<i32>) {
    let parts: Vec<&str> = input.split('+').collect();
    if parts.len() < 2 {
        return (input.to_string(), None);
    }
    let mut modifiers = 0i32;
    let mut key_parts: Vec<&str> = Vec::new();
    for part in &parts {
        match key_modifier_bit(part) {
            Some(bit) => modifiers |= bit,
            None => key_parts.push(part),
        }
    }
    if modifiers == 0 {
        return (input.to_string(), None);
    }
    let actual_key = if key_parts.is_empty() {
        input.to_string()
    } else {
        key_parts.join("+")
    };
    (actual_key, Some(modifiers))
}

fn key_modifier_bit(key: &str) -> Option<i32> {
    match key.to_ascii_lowercase().as_str() {
        "alt" | "altleft" | "altright" => Some(1),
        "control" | "ctrl" | "controlleft" | "controlright" => Some(2),
        "meta" | "cmd" | "command" | "metaleft" | "metaright" => Some(4),
        "shift" | "shiftleft" | "shiftright" => Some(8),
        _ => None,
    }
}

fn is_non_printable_control_key(key_name: &str) -> bool {
    matches!(
        key_name,
        "Enter"
            | "Tab"
            | "Escape"
            | "Backspace"
            | "Delete"
            | "ArrowUp"
            | "ArrowDown"
            | "ArrowLeft"
            | "ArrowRight"
            | "Home"
            | "End"
            | "PageUp"
            | "PageDown"
    )
}

/// The key a press produces: named keys normalized, a character shifted (US layout).
fn resolved_key(key: &str, modifiers: i32) -> String {
    let named = match key.to_lowercase().as_str() {
        "enter" | "return" => "Enter".to_string(),
        "tab" => "Tab".to_string(),
        "escape" | "esc" => "Escape".to_string(),
        "backspace" => "Backspace".to_string(),
        "delete" => "Delete".to_string(),
        "arrowup" | "up" => "ArrowUp".to_string(),
        "arrowdown" | "down" => "ArrowDown".to_string(),
        "arrowleft" | "left" => "ArrowLeft".to_string(),
        "arrowright" | "right" => "ArrowRight".to_string(),
        "home" => "Home".to_string(),
        "end" => "End".to_string(),
        "pageup" => "PageUp".to_string(),
        "pagedown" => "PageDown".to_string(),
        "space" => " ".to_string(),
        lower => lower
            .strip_prefix("key")
            .filter(|rest| rest.len() == 1 && rest.as_bytes()[0].is_ascii_lowercase())
            .or_else(|| {
                lower
                    .strip_prefix("digit")
                    .filter(|rest| rest.len() == 1 && rest.as_bytes()[0].is_ascii_digit())
            })
            .map(String::from)
            .unwrap_or_else(|| key.to_string()),
    };
    if modifiers & 8 != 0
        && let Some(shifted) = shifted_char(&named)
    {
        return shifted.to_string();
    }
    named
}

fn shifted_char(key_name: &str) -> Option<char> {
    let mut chars = key_name.chars();
    let ch = chars.next()?;
    if chars.next().is_some() {
        return None;
    }
    if ch.is_ascii_lowercase() {
        return Some(ch.to_ascii_uppercase());
    }
    const PAIRS: &[(char, char)] = &[
        ('1', '!'),
        ('2', '@'),
        ('3', '#'),
        ('4', '$'),
        ('5', '%'),
        ('6', '^'),
        ('7', '&'),
        ('8', '*'),
        ('9', '('),
        ('0', ')'),
        ('`', '~'),
        ('-', '_'),
        ('=', '+'),
        ('[', '{'),
        (']', '}'),
        ('\\', '|'),
        (';', ':'),
        ('\'', '"'),
        (',', '<'),
        ('.', '>'),
        ('/', '?'),
    ];
    PAIRS
        .iter()
        .find(|(plain, _)| *plain == ch)
        .map(|(_, shifted)| *shifted)
}

/// The field a command stores typed input in: `fillInput` uses `value`,
/// `typeText` and `insertText` use `text`.
fn typed_field(cmd: &Value) -> Option<&'static str> {
    let field = match action_of(cmd) {
        "fillInput" => "value",
        "typeText" | "insertText" => "text",
        _ => return None,
    };
    cmd[field].as_str().map(|_| field)
}

/// Headers a command sends are classified like a request's: a credential or
/// cookie value is never stored. The step is marked redacted, and the values
/// count as secrets.
fn redact_headers(cmd: &mut Value) -> Vec<String> {
    let mut found = Vec::new();
    let Some(headers) = cmd.get_mut("headers").and_then(|h| h.as_object_mut()) else {
        return found;
    };
    for (name, value) in headers.iter_mut() {
        let lower = name.to_ascii_lowercase();
        let secret = classify_header(&lower) == HeaderClass::Credential
            || matches!(lower.as_str(), "cookie" | "set-cookie");
        if secret {
            if let Some(text) = value.as_str().filter(|t| !t.is_empty()) {
                found.push(text.to_string());
            }
            *value = json!(REDACTED);
        }
    }
    if !found.is_empty() || headers.values().any(|v| v == REDACTED) {
        cmd["redacted"] = json!(true);
    }
    found
}

/// A value typed into a field that looks like a password or code, or a sealed
/// handle, is never stored; the flow keeps a marker and cannot be replayed
/// through the UI. Returns whether the step was redacted and the value removed
/// when the Bridge saw it in the clear.
fn redact_secret(cmd: &mut Value, traits: Option<&FieldTraits>) -> (bool, Option<String>) {
    let typed = typed_field(cmd);
    let Some(obj) = cmd.as_object_mut() else {
        return (false, None);
    };
    let Some(field) = typed.or_else(|| {
        ["value", "text"]
            .into_iter()
            .find(|k| obj.get(*k).and_then(|v| v.as_str()).is_some())
    }) else {
        return (false, None);
    };
    let Some(original) = obj.get(field).and_then(Value::as_str).map(String::from) else {
        return (false, None);
    };
    let sealed = !handles_in(&original).is_empty();
    let target = obj.get("target").cloned().unwrap_or(Value::Null);
    let target = target.get("ref").unwrap_or(&target);
    let mut words = ["selector", "name", "label", "placeholder"]
        .iter()
        .filter_map(|k| target.get(*k).and_then(|v| v.as_str()))
        .collect::<Vec<_>>()
        .join(" ");
    if let Some(t) = traits {
        words.push(' ');
        words.push_str(&t.hint);
    }
    if !(sealed || traits.is_some_and(|t| t.is_secret()) || has_secret_marker(&words)) {
        return (false, None);
    }
    obj.insert(field.to_string(), json!(REDACTED));
    obj.insert("redacted".to_string(), json!(true));
    let secret = (!sealed && !original.is_empty()).then_some(original);
    (true, secret)
}

/// A param name from what the page calls the field: lowercase letters,
/// digits and underscores, starting with a letter.
pub(crate) fn typed_param_name(traits: Option<&FieldTraits>, taken: &[String]) -> String {
    let base: String = traits
        .and_then(|t| t.hint.split_whitespace().next())
        .map(|w| {
            w.chars()
                .map(|c| {
                    if c.is_ascii_alphanumeric() {
                        c.to_ascii_lowercase()
                    } else {
                        '_'
                    }
                })
                .collect::<String>()
        })
        .map(|w| w.trim_matches('_').chars().take(24).collect::<String>())
        .filter(|w| w.chars().next().is_some_and(|c| c.is_ascii_lowercase()))
        .unwrap_or_else(|| "typed".to_string());
    let mut name = base.clone();
    let mut n = 1;
    while taken.contains(&name) {
        n += 1;
        name = format!("{base}{n}");
    }
    name
}

/// A ref in a target, which lives only until the next snapshot, stored as the
/// role and name it pointed at.
fn stable_target(value: &Value) -> Value {
    let Some(entry) = value.get("ref").filter(|r| r.is_object()) else {
        return value.clone();
    };
    let text = |k: &str| entry.get(k).and_then(Value::as_str).unwrap_or("");
    if text("role").is_empty() || text("name").is_empty() {
        return value.clone();
    }
    let mut out = json!({ "role": text("role"), "name": text("name"), "exact": true });
    if let Some(nth) = entry
        .get("nth")
        .filter(|n| n.as_u64().is_some_and(|n| n > 0))
    {
        out["nth"] = nth.clone();
    }
    out
}

/// The command a step stores: `{"do": "<action>", ...its input}`, refs as role and name,
/// without the tab (a replay runs in the run's tab) or the approver tag.
pub(crate) fn stable_command(action: &str, input: &Value) -> Value {
    let mut out = serde_json::Map::new();
    out.insert("do".into(), json!(tools::short_name(action)));
    if let Some(object) = input.as_object() {
        for (key, value) in object {
            if matches!(key.as_str(), "tabId" | "_approver" | "do") {
                continue;
            }
            let value = if TARGET_KEYS.contains(&key.as_str()) {
                stable_target(value)
            } else {
                value.clone()
            };
            out.insert(key.clone(), value);
        }
    }
    Value::Object(out)
}

/// One tab's recording.
pub(crate) struct Recording {
    pub install_id: String,
    pub tab: i64,
    pub recorder: FlowRecorder,
    pub entries: Vec<HarEntry>,
    pub started_at: String,
    next_entry: u64,
}

impl Recording {
    /// Requests read back from the tab's HAR, tagged to the steps they belong to.
    pub(crate) fn add_har(&mut self, har: &Value) {
        let entries = super::har_entries(har, &mut self.next_entry);
        for e in &entries {
            self.recorder.tag(&e.request_id, e.started_ms);
        }
        self.entries.extend(entries);
    }

    pub(crate) fn summary(&self) -> Value {
        json!({
            "tabId": self.tab,
            "capturing": self.recorder.active(),
            "steps": self.recorder.commands.len(),
            "requests": self.entries.len(),
            "unsaved": self.recorder.unsaved_steps(),
            "startedAt": self.started_at,
        })
    }
}

/// Every tab's recording, and the tab the Bridge last saw the user act in.
#[derive(Default)]
pub(crate) struct Recorders {
    pub(crate) list: Vec<Recording>,
    active_tab: Option<i64>,
}

impl Recorders {
    /// The recording for `tab`, or for the tab page actions without a tab reach.
    pub(crate) fn find(&self, tab: Option<i64>, capturing_only: bool) -> Option<usize> {
        let eligible = |r: &&Recording| !capturing_only || r.recorder.active();
        let index_of = |wanted: i64| {
            self.list
                .iter()
                .rposition(|r| r.tab == wanted && eligible(&r))
        };
        match tab.or(self.active_tab) {
            Some(wanted) => index_of(wanted).or_else(|| {
                // A call without a tab when the Bridge lost track of the active one.
                tab.is_none()
                    .then(|| self.list.iter().rposition(|r| eligible(&r)))
                    .flatten()
            }),
            None => self.list.iter().rposition(|r| eligible(&r)),
        }
    }

    /// A new recording on `tab`, replacing what the tab recorded before.
    pub(crate) fn start(&mut self, install_id: &str, tab: i64, page_url: Option<String>) {
        self.list
            .retain(|r| !(r.tab == tab && r.install_id == install_id));
        let mut recorder = FlowRecorder::default();
        recorder.begin_capture(page_url);
        self.list.push(Recording {
            install_id: install_id.to_string(),
            tab,
            recorder,
            entries: Vec::new(),
            started_at: vexdom_core::log::iso_now(),
            next_entry: 0,
        });
        self.active_tab = Some(tab);
    }

    /// Which tab later calls without a tab reach, as far as the actions tell.
    fn note_tab(&mut self, action: &str, input: &Value, ok: bool) {
        if !ok {
            return;
        }
        match action {
            "page.switchTab" => {
                self.active_tab = input.get("tabId").and_then(Value::as_i64);
            }
            "page.openTab" | "page.closeTab" | "page.openWindow" => self.active_tab = None,
            _ => {}
        }
    }
}

/// Every external page action, after it ran, so a recording tab logs it. `tab` is the input's
/// tabId (`None` = the active tab). Actions outside `page.*` are never logged.
pub(crate) fn observe(
    daemon: &Daemon,
    tab: Option<i64>,
    action: &str,
    input: &Value,
    started_at_ms: i64,
    ended_at_ms: i64,
    ok: bool,
) {
    if !action.starts_with("page.") {
        return;
    }
    // These name the tab they pick, not the tab they run from.
    let tab = if matches!(action, "page.switchTab" | "page.openTab" | "page.closeTab") {
        None
    } else {
        tab
    };
    let mut flows = daemon.0.flows.borrow_mut();
    let index = flows.find(tab, true);
    flows.note_tab(action, input, ok);
    let Some(index) = index else { return };
    let recording = &mut flows.list[index];
    let Some(seq) =
        recording
            .recorder
            .record(action, input, started_at_ms as f64, ended_at_ms as f64, ok)
    else {
        return;
    };
    let (install_id, tab) = (recording.install_id.clone(), recording.tab);
    drop(flows);
    let Some(link) = daemon.link(&install_id).filter(|link| link.is_open()) else {
        return;
    };
    // The step's address, read once it finished, for `flowSteps` and the next step's start.
    let daemon = daemon.clone();
    tokio::task::spawn_local(async move {
        let reply = link
            .invoke("page.query", &json!({"what": "url"}), Some(tab), None)
            .await;
        let url = reply
            .result
            .ok()
            .and_then(|data| data.get("value").and_then(Value::as_str).map(String::from));
        let mut flows = daemon.0.flows.borrow_mut();
        if let Some(recording) = flows
            .list
            .iter_mut()
            .find(|r| r.install_id == install_id && r.tab == tab)
        {
            recording.recorder.set_url(seq, url);
        }
    });
}

/// A secret the Bridge handled in the clear (a vault login's password): every running
/// recording drops the requests that carry it. Nothing happens without a recording.
pub(crate) fn add_secret(daemon: &Daemon, secret: &str) {
    for recording in daemon
        .0
        .flows
        .borrow_mut()
        .list
        .iter_mut()
        .filter(|r| r.recorder.active())
    {
        recording.recorder.add_secret(secret);
    }
}
