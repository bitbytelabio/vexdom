//! Saved flows (`bridge.flow*`): record a task once in the browser, then replay its first-party
//! API requests from the page. A port of agent-browser `cli/src/native/flow/*` (f0a9835,
//! Apache-2.0, see NOTICE) for the Bridge: capture is an explicit per-tab recording (a HAR from
//! the extension plus the external page actions the Bridge saw), the tiers are the page's own
//! `fetch` and the replay of the recorded commands (no plain-HTTP tier: cookie values never leave
//! the browser), and a write the run was not allowed to send waits for the user's approval.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use serde_json::{Map, Value, json};
use vexdom_core::guardrails::secrets::handles_in;
use vexdom_core::paths;

mod handlers;
mod hosts;
mod model;
mod outcome;
mod page_tier;
mod recorder;
mod replay;
mod send;
mod template;

// The entry points `invoke.rs` and `auth_vault.rs` call.
pub(crate) use handlers::handle;
pub(crate) use hosts::{HostPolicy, base_domain, host_of, validate_origin};
pub(crate) use model::{
    CreatedBy, Expect, FLOW_VERSION, Flow, Param, ParamFrom, ParamSource, Request, Step, Tier,
    Write, WriteKind,
};
pub(crate) use recorder::{FlowRecorder, REDACTED, Recorders};
pub(crate) use recorder::{add_secret, observe};
use template::{Body, Parts, Slot};

// ---------------------------------------------------------------------------
// Captured requests
// ---------------------------------------------------------------------------

/// One request of a recording, read back from the extension's HAR.
#[derive(Clone, Debug, Default)]
pub(crate) struct HarEntry {
    /// Minted by the Bridge: the HAR carries no ids.
    pub request_id: String,
    /// When the request started, epoch milliseconds.
    pub started_ms: f64,
    pub method: String,
    pub url: String,
    pub request_headers: Vec<(String, String)>,
    pub post_data: Option<String>,
    pub resource_type: String,
    /// `None` until a response arrived.
    pub status: Option<i64>,
    pub mime_type: String,
    pub response_headers: Vec<(String, String)>,
    pub response_body: Option<String>,
    pub response_body_base64: bool,
    /// An answer (or a failure) arrived before the HAR was read.
    pub finished: bool,
}

fn pairs(list: &Value) -> Vec<(String, String)> {
    list.as_array()
        .map(|items| {
            items
                .iter()
                .map(|h| {
                    (
                        h["name"].as_str().unwrap_or("").to_string(),
                        h["value"].as_str().unwrap_or("").to_string(),
                    )
                })
                .collect()
        })
        .unwrap_or_default()
}

/// The entries of a HAR 1.2 document (`{log: {entries}}`), ids minted from `next`.
pub(crate) fn har_entries(har: &Value, next: &mut u64) -> Vec<HarEntry> {
    let Some(entries) = har.pointer("/log/entries").and_then(Value::as_array) else {
        return Vec::new();
    };
    entries
        .iter()
        .map(|e| {
            *next += 1;
            let request = &e["request"];
            let response = &e["response"];
            let status = response["status"].as_i64().filter(|s| *s > 0);
            let failed = !response["statusText"].as_str().unwrap_or("").is_empty();
            let content = &response["content"];
            HarEntry {
                request_id: format!("h{next}"),
                started_ms: e["startedDateTime"]
                    .as_str()
                    .and_then(|t| t.parse::<jiff::Timestamp>().ok())
                    .map_or(0.0, |t| t.as_millisecond() as f64),
                method: request["method"].as_str().unwrap_or("GET").to_string(),
                url: request["url"].as_str().unwrap_or("").to_string(),
                request_headers: pairs(&request["headers"]),
                post_data: request
                    .pointer("/postData/text")
                    .and_then(Value::as_str)
                    .map(String::from),
                resource_type: e["_resourceType"].as_str().unwrap_or("Other").to_string(),
                status,
                // The extension writes a placeholder type when no headers came back.
                mime_type: if status.is_some() {
                    content["mimeType"].as_str().unwrap_or("").to_string()
                } else {
                    String::new()
                },
                response_headers: pairs(&response["headers"]),
                response_body: content["text"].as_str().map(String::from),
                response_body_base64: content["encoding"] == "base64",
                finished: status.is_some() || failed,
            }
        })
        .collect()
}

/// `urlencoding::encode`: everything but `A-Z a-z 0-9 - _ . ~` as `%XX`.
pub(crate) fn url_encode(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

/// `urlencoding::decode`: `%XX` escapes, `None` when the bytes are not UTF-8.
fn url_decode(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = |b: u8| (b as char).to_digit(16);
        if bytes[i] == b'%'
            && let (Some(h), Some(l)) = (
                bytes.get(i + 1).copied().and_then(hex),
                bytes.get(i + 2).copied().and_then(hex),
            )
        {
            out.push((h * 16 + l) as u8);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8(out).ok()
}

// ---------------------------------------------------------------------------
// Request selection
// ---------------------------------------------------------------------------

const TELEMETRY_PATH_MARKERS: &[&str] = &[
    "/collect",
    "/track",
    "/beacon",
    "/analytics",
    "/pixel",
    "/telemetry",
    "/metrics",
    "/log",
    "/rum",
    "/sentry",
];

/// How a recorded request header is treated.
#[derive(PartialEq)]
pub(crate) enum HeaderClass {
    /// Carries a credential the session supplies at run time. Never saved; a
    /// request recorded with one may get a 401 or 403 when sent again.
    Credential,
    /// Per-connection or session-owned. Never saved.
    Dropped,
    /// Set by the browser itself; saved, but not sent by an in-page fetch.
    BrowserSet,
    Kept,
}

pub(crate) fn classify_header(name: &str) -> HeaderClass {
    let lower = name.to_ascii_lowercase();
    match lower.as_str() {
        "authorization" | "proxy-authorization" => HeaderClass::Credential,
        _ if lower.contains("token") || lower.contains("csrf") => HeaderClass::Credential,
        "cookie" | "host" | "content-length" | "connection" | "accept-encoding" => {
            HeaderClass::Dropped
        }
        "user-agent" | "referer" | "origin" | "accept-language" | "priority" | "dnt" => {
            HeaderClass::BrowserSet
        }
        _ if lower.starts_with("sec-") => HeaderClass::BrowserSet,
        // Per-request tracing and client identity: not part of what a flow replays.
        "x-client-ip" | "x-forwarded-for" | "x-real-ip" | "forwarded" | "traceparent"
        | "tracestate" | "x-request-id" | "request-id" | "x-amzn-trace-id" | "sentry-trace"
        | "baggage" | "cf-connecting-ip" | "true-client-ip" => HeaderClass::Dropped,
        _ => HeaderClass::Kept,
    }
}

pub(crate) fn is_telemetry(url: &str) -> bool {
    let path = url::Url::parse(url)
        .map(|u| u.path().to_ascii_lowercase())
        .unwrap_or_default();
    TELEMETRY_PATH_MARKERS
        .iter()
        .any(|m| path == *m || path.starts_with(&format!("{m}/")) || path.ends_with(m))
}

fn is_api_entry(e: &HarEntry, policy: &HostPolicy) -> bool {
    matches!(e.resource_type.as_str(), "XHR" | "Fetch")
        && e.mime_type.contains("json")
        && e.status.map(|s| (200..300).contains(&s)).unwrap_or(false)
        && policy.allows_url(&e.url)
        && !is_telemetry(&e.url)
}

// ---------------------------------------------------------------------------
// Response signatures
// ---------------------------------------------------------------------------

fn type_name(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

const SIGNATURE_DEPTH: usize = 3;

/// Shape of a JSON body, a few levels deep: object keys with their types,
/// and the element shape for arrays.
pub(crate) fn signature(v: &Value) -> Value {
    signature_at(v, SIGNATURE_DEPTH)
}

fn signature_at(v: &Value, depth: usize) -> Value {
    match v {
        Value::Object(map) if depth > 0 => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            let mut sig = Map::new();
            for k in keys {
                sig.insert(k.clone(), signature_at(&map[k], depth - 1));
            }
            json!({ "object": sig })
        }
        Value::Array(items) if depth > 0 => match items.first() {
            Some(first) => json!({ "array": signature_at(first, depth - 1) }),
            None => json!({ "array": null }),
        },
        other => json!({ "scalar": type_name(other) }),
    }
}

fn signature_type(sig: &Value) -> &str {
    if sig.get("object").is_some() {
        "object"
    } else if sig.get("array").is_some() {
        "array"
    } else {
        sig.get("scalar")
            .and_then(|t| t.as_str())
            .unwrap_or("unknown")
    }
}

/// Every key the recording saw must still be there with the same type. New
/// keys are fine; a missing or retyped key means the contract moved.
pub(crate) fn signature_matches(recorded: &Value, current: &Value) -> Result<(), String> {
    matches_at(recorded, current, "", false)
}

/// Like `signature_matches`, but a recorded count may be missing: many APIs
/// drop the totals when a list is empty. A missing list is never accepted.
pub(crate) fn matches_without_counts(recorded: &Value, current: &Value) -> bool {
    matches_at(recorded, current, "", true).is_ok()
}

/// A non-empty list in `body` under a key the recording did not have, to name
/// as where a missing recorded list went. A list whose elements are the same
/// type as the missing one's comes first.
pub(crate) fn moved_list(recorded: &Value, body: &Value) -> Option<String> {
    let current = signature_at(body, 8);
    let mut lost = Vec::new();
    missing_lists(recorded, &current, "", &mut lost);
    let mut found = Vec::new();
    appeared_lists(recorded, &current, "", &mut found);
    let same_type = |elem: &Value| {
        lost.iter()
            .any(|l| signature_type(l) == signature_type(elem))
    };
    found
        .iter()
        .find(|(_, elem)| same_type(elem))
        .or(found.first())
        .map(|(p, _)| p.clone())
}

fn child_path(path: &str, key: &str) -> String {
    if path.is_empty() {
        key.to_string()
    } else {
        format!("{path}.{key}")
    }
}

/// Element shapes of the recorded lists that `current` no longer has.
fn missing_lists(recorded: &Value, current: &Value, path: &str, out: &mut Vec<Value>) {
    let Some(rec) = recorded.get("object").and_then(|v| v.as_object()) else {
        return;
    };
    let cur = current.get("object").and_then(|v| v.as_object());
    for (key, sig) in rec {
        let child = child_path(path, key);
        match cur.and_then(|c| c.get(key)) {
            Some(cur_sig) => missing_lists(sig, cur_sig, &child, out),
            None => lists_within(sig, &child, &mut |_, elem| out.push(elem.clone())),
        }
    }
}

/// Non-empty lists in `current` under keys the recording did not have, with their element shape.
fn appeared_lists(recorded: &Value, current: &Value, path: &str, out: &mut Vec<(String, Value)>) {
    let Some(cur) = current.get("object").and_then(|v| v.as_object()) else {
        return;
    };
    let rec = recorded.get("object").and_then(|v| v.as_object());
    for (key, sig) in cur {
        let child = child_path(path, key);
        match rec.and_then(|r| r.get(key)) {
            None => lists_within(sig, &child, &mut |p, elem| {
                if !elem.is_null() {
                    out.push((p.to_string(), elem.clone()))
                }
            }),
            Some(rec_sig) => appeared_lists(rec_sig, sig, &child, out),
        }
    }
}

/// Calls `f` with the path and element shape of each list at or under `sig`.
/// A recorded list that was cut off by the signature depth has no element shape.
fn lists_within(sig: &Value, path: &str, f: &mut dyn FnMut(&str, &Value)) {
    if signature_type(sig) == "array" {
        f(path, sig.get("array").unwrap_or(&Value::Null));
    } else if let Some(map) = sig.get("object").and_then(|v| v.as_object()) {
        for (k, v) in map {
            lists_within(v, &child_path(path, k), f);
        }
    }
}

fn matches_at(
    recorded: &Value,
    current: &Value,
    path: &str,
    counts_optional: bool,
) -> Result<(), String> {
    let (rec_ty, cur_ty) = (signature_type(recorded), signature_type(current));
    if rec_ty == "null" || cur_ty == "null" {
        return Ok(());
    }
    if rec_ty != cur_ty {
        let at = if path.is_empty() {
            "response".to_string()
        } else {
            format!("'{path}'")
        };
        return Err(format!("{at} changed from {rec_ty} to {cur_ty}"));
    }
    if let (Some(rec), Some(cur)) = (
        recorded.get("object").and_then(|v| v.as_object()),
        current.get("object").and_then(|v| v.as_object()),
    ) {
        for (key, sig) in rec {
            let child = child_path(path, key);
            match cur.get(key) {
                // Counts and totals often go with an empty list.
                None if counts_optional && signature_type(sig) == "number" => {}
                None => return Err(format!("key '{child}' is missing")),
                Some(cur_sig) => matches_at(sig, cur_sig, &child, counts_optional)?,
            }
        }
    }
    if let (Some(rec), Some(cur)) = (recorded.get("array"), current.get("array"))
        && !rec.is_null()
        && !cur.is_null()
    {
        matches_at(rec, cur, &format!("{path}[]"), counts_optional)?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Params
// ---------------------------------------------------------------------------

pub(crate) fn parse_params(cmd: &Value) -> Result<Vec<(String, String)>, String> {
    let mut out = Vec::new();
    if let Some(obj) = cmd.get("params").and_then(|v| v.as_object()) {
        for (k, v) in obj {
            let value = v.as_str().ok_or(format!("param '{k}' must be a string"))?;
            out.push((k.clone(), value.to_string()));
        }
    }
    Ok(out)
}

pub(crate) fn map_strings(
    v: &mut Value,
    f: &dyn Fn(&str) -> Result<String, String>,
) -> Result<(), String> {
    match v {
        Value::String(s) => *s = f(s)?,
        Value::Array(items) => {
            for item in items {
                map_strings(item, f)?;
            }
        }
        Value::Object(map) => {
            for item in map.values_mut() {
                map_strings(item, f)?;
            }
        }
        _ => {}
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Save
// ---------------------------------------------------------------------------

pub(crate) struct SaveInput<'a> {
    pub name: &'a str,
    pub recorder: &'a FlowRecorder,
    /// The recorded steps this flow covers.
    pub commands: &'a [(u64, Value)],
    pub description: Option<&'a str>,
    pub entries: &'a [HarEntry],
    pub params: &'a [(String, String)],
    pub output: Option<&'a str>,
    pub created_at: String,
    /// The page shows a password field at save time.
    pub login_wall: bool,
}

/// The action of a recorded command.
pub(crate) fn action_of(cmd: &Value) -> &str {
    cmd.get("do").and_then(Value::as_str).unwrap_or("")
}

fn start_url(commands: &[(u64, Value)]) -> Option<String> {
    commands.iter().find_map(|(_, c)| {
        (action_of(c) == "navigate")
            .then(|| c.get("url").and_then(|u| u.as_str()).map(String::from))
            .flatten()
    })
}

/// A recorded command in one line: the action and what it names.
pub(crate) fn describe_command(cmd: &Value) -> String {
    let action = cmd.get("do").and_then(|a| a.as_str()).unwrap_or("?");
    let target = cmd.get("target");
    let named = cmd.get("url").and_then(Value::as_str).or_else(|| {
        [
            "selector",
            "name",
            "label",
            "placeholder",
            "text",
            "testId",
            "title",
            "alt",
        ]
        .iter()
        .find_map(|k| target.and_then(|t| t.get(*k)).and_then(Value::as_str))
    });
    let target = named.or_else(|| {
        ["text", "value", "key"]
            .iter()
            .find_map(|k| cmd.get(*k).and_then(|v| v.as_str()))
    });
    match target {
        Some(t) => format!("{action} {t}"),
        None => action.to_string(),
    }
}

/// How a dropped request is named: its path without the query, with any
/// segment that holds a secret hidden. When the secret is unknown, only the host.
pub(crate) fn dropped_name(e: &HarEntry, secrets: &[String], known: bool) -> String {
    let Ok(url) = url::Url::parse(&e.url) else {
        return format!("{} ?", e.method);
    };
    if !known {
        return format!("{} {}", e.method, url.host_str().unwrap_or("?"));
    }
    let path: Vec<String> = url
        .path()
        .split('/')
        .map(|seg| {
            let decoded = percent_decode_lossless(seg);
            let hit = !handles_in(&decoded).is_empty()
                || secrets.iter().any(|s| {
                    let escaped = serde_json::to_string(s).unwrap_or_default();
                    seg.contains(s.as_str())
                        || seg.contains(&url_encode(s))
                        || seg.contains(escaped.trim_matches('"'))
                        || decoded.contains(s.as_str())
                });
            if hit { REDACTED } else { seg }.to_string()
        })
        .collect();
    let path = path.join("/");
    if holds_secret(&path, secrets) {
        return format!("{} {}", e.method, url.host_str().unwrap_or("?"));
    }
    format!("{} {}", e.method, path)
}

pub(crate) fn carries_secret(e: &HarEntry, secrets: &[String]) -> bool {
    holds_secret(&e.url, secrets)
        || e.post_data
            .as_deref()
            .is_some_and(|t| holds_secret(t, secrets))
        || e.request_headers
            .iter()
            .any(|(_, v)| holds_secret(v, secrets))
}

/// Whether `text` holds a secret as is, form-encoded, percent-encoded or JSON-escaped, or a
/// sealed handle (`⟦kind:id#tag⟧`): what the browser sealed is a credential by construction.
pub(crate) fn holds_secret(text: &str, secrets: &[String]) -> bool {
    if decoded_forms(text)
        .iter()
        .chain(std::iter::once(&text.to_string()))
        .any(|t| !handles_in(t).is_empty())
    {
        return true;
    }
    secrets.iter().any(|secret| {
        let encoded = url_encode(secret);
        let escaped = serde_json::to_string(secret).unwrap_or_default();
        let escaped = escaped.trim_matches('"');
        text.contains(secret.as_str())
            || text.contains(&encoded)
            || text.contains(escaped)
            || decoded_forms(text)
                .iter()
                .any(|d| d.contains(secret.as_str()))
    })
}

/// Percent decoding that never fails: an escape that does not form valid
/// UTF-8 stays exactly as written, so the rest of the text is still compared.
fn percent_decode_lossless(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = String::new();
    // Decoded bytes of a run of escapes, each with the source it came from.
    let mut run: Vec<(u8, &str)> = Vec::new();
    let flush = |run: &mut Vec<(u8, &str)>, out: &mut String| {
        let raw: Vec<u8> = run.iter().map(|(b, _)| *b).collect();
        let mut at = 0;
        while at < raw.len() {
            match std::str::from_utf8(&raw[at..]) {
                Ok(valid) => {
                    out.push_str(valid);
                    break;
                }
                Err(e) => {
                    let good = at + e.valid_up_to();
                    out.push_str(std::str::from_utf8(&raw[at..good]).unwrap_or(""));
                    let end = good + e.error_len().unwrap_or(raw.len() - good);
                    for (_, src) in &run[good..end] {
                        out.push_str(src);
                    }
                    at = end;
                }
            }
        }
        run.clear();
    };
    let mut i = 0;
    while i < bytes.len() {
        let hex = |b: u8| (b as char).to_digit(16);
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let (Some(h), Some(l)) = (hex(bytes[i + 1]), hex(bytes[i + 2]))
        {
            run.push(((h * 16 + l) as u8, &text[i..i + 3]));
            i += 3;
            continue;
        }
        flush(&mut run, &mut out);
        let ch_len = text[i..].chars().next().map_or(1, char::len_utf8);
        out.push_str(&text[i..i + ch_len]);
        i += ch_len;
    }
    flush(&mut run, &mut out);
    out
}

/// A request text as the server reads it: form encoding (`+` is a space),
/// percent encoding, and the strings inside a JSON body, whose escapes the
/// parser resolves.
fn decoded_forms(text: &str) -> Vec<String> {
    let mut out = vec![
        percent_decode_lossless(&text.replace('+', " ")),
        percent_decode_lossless(text),
    ];
    if let Ok(json) = serde_json::from_str::<Value>(text) {
        let mut strings = Vec::new();
        collect_scalars(&json, &mut strings);
        out.extend(strings);
    }
    out
}

/// Data a recorded request can answer with: the JSON body of an API call,
/// or each JSON block embedded in a first-party HTML document (Next.js
/// `__NEXT_DATA__`, JSON-LD, framework state), for sites that render on the
/// server.
fn candidates<'a>(
    tagged: &[&'a HarEntry],
    policy: &HostPolicy,
) -> Vec<(&'a HarEntry, Value, Option<Value>)> {
    let mut out = Vec::new();
    for e in tagged.iter().copied() {
        let Some(body) = e
            .response_body
            .as_deref()
            .filter(|_| !e.response_body_base64)
        else {
            continue;
        };
        if is_api_entry(e, policy) {
            if let Ok(parsed) = serde_json::from_str::<Value>(body) {
                out.push((e, parsed, None));
            }
        } else if is_document_entry(e, policy) {
            for (spec, parsed) in embedded_json(body) {
                out.push((e, parsed, Some(spec)));
            }
        }
    }
    out
}

fn is_document_entry(e: &HarEntry, policy: &HostPolicy) -> bool {
    e.resource_type == "Document"
        && e.method == "GET"
        && e.mime_type.contains("html")
        && e.status.map(|s| (200..300).contains(&s)).unwrap_or(false)
        && policy.allows_url(&e.url)
}

/// `<script>` blocks whose whole content is a JSON object or array, each with
/// a spec that finds it again: its `id` if it has one, else its position.
pub(crate) fn embedded_json(html: &str) -> Vec<(Value, Value)> {
    let mut out = Vec::new();
    let lower = html.to_ascii_lowercase();
    let mut pos = 0;
    let mut index = 0;
    while let Some(start) = lower[pos..].find("<script") {
        let open = pos + start;
        let Some(tag_end) = lower[open..].find('>').map(|i| open + i) else {
            break;
        };
        let Some(close) = lower[tag_end..].find("</script").map(|i| tag_end + i) else {
            break;
        };
        let attrs = &html[open + 7..tag_end];
        let content = html[tag_end + 1..close].trim();
        pos = close + 8;
        let ty = attr(attrs, "type").unwrap_or_default().to_ascii_lowercase();
        if !(ty.is_empty() || ty.contains("json")) {
            continue;
        }
        let content = assigned_value(content).unwrap_or(content);
        if !(content.starts_with('{') || content.starts_with('[')) {
            continue;
        }
        let Ok(parsed) = serde_json::from_str::<Value>(content) else {
            continue;
        };
        let spec = match attr(attrs, "id") {
            Some(id) => json!({ "scriptId": id }),
            None => json!({ "scriptIndex": index }),
        };
        index += 1;
        out.push((spec, parsed));
    }
    out
}

/// The JSON in a script that only assigns it: `window.__context__ = {...};`.
fn assigned_value(content: &str) -> Option<&str> {
    let eq = content.find('=')?;
    let target = content[..eq].trim();
    let target = ["window.", "self.", "var ", "let ", "const "]
        .iter()
        .find_map(|p| target.strip_prefix(p))
        .unwrap_or(target)
        .trim();
    let simple = !target.is_empty()
        && target.chars().all(|c| {
            c.is_ascii_alphanumeric() || matches!(c, '_' | '$' | '.' | '[' | ']' | '"' | '\'')
        });
    simple.then(|| content[eq + 1..].trim().trim_end_matches(';').trim())
}

fn attr(attrs: &str, name: &str) -> Option<String> {
    let lower = attrs.to_ascii_lowercase();
    let at = lower.find(&format!("{name}="))?;
    let rest = &attrs[at + name.len() + 1..];
    let (quote, rest) = match rest.chars().next()? {
        q @ ('"' | '\'') => (q, &rest[1..]),
        _ => (' ', rest),
    };
    let end = rest
        .find(|c: char| c == quote || (quote == ' ' && c == '>'))
        .unwrap_or(rest.len());
    Some(rest[..end].to_string())
}

/// Pull the recorded JSON block back out of a document body.
pub(crate) fn extract_embedded(html: &str, spec: &Value) -> Option<String> {
    embedded_json(html)
        .into_iter()
        .find(|(s, _)| {
            (spec.get("scriptId").is_some() && s.get("scriptId") == spec.get("scriptId"))
                || (spec.get("scriptIndex").is_some()
                    && s.get("scriptIndex") == spec.get("scriptIndex"))
        })
        .map(|(_, v)| v.to_string())
}

/// Params the agent passed, plus query values of the start URL that the
/// recorded requests sent too (`?q=tea` -> param `q`), including the page
/// itself when its HTML embeds the data.
fn with_page_query_params(
    start: &str,
    policy: &HostPolicy,
    given: &[(String, String)],
    entries: &[HarEntry],
) -> Vec<(String, String)> {
    let mut out = given.to_vec();
    let Ok(url) = url::Url::parse(start) else {
        return out;
    };
    for (key, value) in url.query_pairs() {
        let (key, value) = (key.into_owned(), value.into_owned());
        if value.chars().count() < 3 || out.iter().any(|(k, v)| *k == key || *v == value) {
            continue;
        }
        let encoded = url_encode(&value);
        let sent = entries.iter().any(|e| {
            (is_api_entry(e, policy)
                || (is_document_entry(e, policy)
                    && e.response_body
                        .as_deref()
                        .is_some_and(|b| !embedded_json(b).is_empty())))
                && (e.url.contains(&encoded)
                    || e.post_data.as_deref().is_some_and(|b| b.contains(&value)))
        });
        if sent {
            out.push((key, value));
        }
    }
    out
}

/// The page a saved range starts on: its first `navigate`, else the page the
/// first step ran on, else the page open when the recording began.
pub(crate) fn save_start_url(commands: &[(u64, Value)], recorder: &FlowRecorder) -> Option<String> {
    start_url(commands)
        .or_else(|| {
            commands
                .first()
                .and_then(|(seq, _)| recorder.url_before(*seq))
                .filter(|u| u.starts_with("http"))
        })
        .or_else(|| recorder.page_url.clone().filter(|u| u.starts_with("http")))
}

/// An unfinished entry that could still become a flow request: it is
/// first-party, not telemetry, an XHR/Fetch or a document, and its response
/// (if the headers already arrived) is not a stream.
pub(crate) fn may_become_candidate(e: &HarEntry, policy: &HostPolicy) -> bool {
    if !policy.allows_url(&e.url) || is_telemetry(&e.url) {
        return false;
    }
    let mime = e.mime_type.to_ascii_lowercase();
    match e.resource_type.as_str() {
        "XHR" | "Fetch" => mime.is_empty() || mime.contains("json"),
        "Document" => mime.is_empty() || mime.contains("html"),
        _ => false,
    }
}

/// Recorded actions that should have sent something; one that sent nothing is a gap.
const INTERACTIVE: &[&str] = &[
    "navigate",
    "clickElement",
    "trustedClick",
    "fillInput",
    "typeText",
    "insertText",
    "pressKey",
    "selectOption",
    "setChecked",
    "submitForm",
];

/// Build the flow file and the save report from what was recorded.
pub(crate) fn build_flow(input: SaveInput<'_>) -> Result<(Flow, Value), String> {
    let start = save_start_url(input.commands, input.recorder).ok_or(
        "nothing to save: the flow has no `navigate` step and no page was open when the recording began",
    )?;
    let host = host_of(&start).ok_or("the flow's start URL has no host")?;
    let base = base_domain(&host);
    let policy = HostPolicy::from_start(&start);
    // Values typed into ordinary fields are params: the step keeps a
    // placeholder, and a param with the same value names it.
    let mut typed: Vec<(String, String)> = Vec::new();
    let mut renames: Vec<(String, String)> = Vec::new();
    for (name, value, seq) in &input.recorder.typed {
        if !input.commands.iter().any(|(s, _)| s == seq) {
            continue;
        }
        let mut name = name.clone();
        if let Some((explicit, _)) = input.params.iter().find(|(_, v)| v == value) {
            if *explicit != name && !renames.iter().any(|(from, _)| *from == name) {
                renames.push((name, explicit.clone()));
            }
            continue;
        }
        if input.params.iter().any(|(n, _)| *n == name) {
            let renamed = format!("{name}_typed");
            renames.push((name, renamed.clone()));
            name = renamed;
        }
        if !typed.iter().any(|(n, _)| *n == name) {
            typed.push((name, value.clone()));
        }
    }
    let commands: Vec<(u64, Value)> = input
        .commands
        .iter()
        .map(|(seq, cmd)| {
            let mut cmd = cmd.clone();
            let _ = map_strings(&mut cmd, &|text| {
                let mut text = text.to_string();
                for (from, to) in &renames {
                    text = text.replace(&format!("{{{{{from}}}}}"), &format!("{{{{{to}}}}}"));
                }
                Ok(text)
            });
            (*seq, cmd)
        })
        .collect();
    let mut explicit_and_typed = input.params.to_vec();
    explicit_and_typed.extend(
        typed
            .iter()
            .filter(|(_, v)| v.chars().count() >= 3)
            .cloned(),
    );
    let mut given = with_page_query_params(&start, &policy, &explicit_and_typed, input.entries);

    let mut steps: Vec<Step> = Vec::new();
    let mut gaps = Vec::new();
    let mut dropped_headers: Vec<String> = Vec::new();
    let mut request_ids: Vec<String> = Vec::new();
    let mut bodies: Vec<(String, Value)> = Vec::new();
    let mut secret_requests: Vec<String> = Vec::new();
    let mut secret_dropped: Vec<String> = Vec::new();
    let mut har_ids: HashMap<String, String> = HashMap::new();
    // The id each request had in the recording (`s<seq>r<n>`), which `flowSteps`
    // numbering gives, next to its id in the saved range.
    let mut capture_ids: Vec<(String, String)> = Vec::new();
    // Recorded Referer values, to restore if a page-query param turns out to
    // be carried by the Referer alone.
    let mut referers: Vec<(String, String, String)> = Vec::new();

    for (index, (seq, cmd)) in commands.iter().enumerate() {
        let tagged: Vec<&HarEntry> = input
            .entries
            .iter()
            .filter(|e| input.recorder.request_tags.get(&e.request_id) == Some(seq))
            .collect();
        let mut requests: Vec<Request> = Vec::new();
        let mut seen = Vec::new();
        for (e, parsed, extract) in candidates(&tagged, &policy) {
            let known = carries_secret(e, &input.recorder.secrets);
            if known || input.recorder.unknown_secret_seqs.contains(seq) {
                if !secret_requests.contains(&e.request_id) {
                    secret_requests.push(e.request_id.clone());
                    let what = dropped_name(e, &input.recorder.secrets, known);
                    let verb = if known { "carried" } else { "may carry" };
                    secret_dropped.push(format!("{what} {verb}"));
                    gaps.push(json!({
                        "step": index + 1,
                        "command": describe_command(cmd),
                        "reason": format!(
                            "{what} {verb} a value typed or pasted into a secret field, so it was not saved; fill a non-secret field or use a declared tool for a reusable flow"
                        ),
                    }));
                }
                continue;
            }
            let key = format!(
                "{} {} {} {}",
                e.method,
                e.url,
                e.post_data.as_deref().unwrap_or(""),
                extract.as_ref().map(|x| x.to_string()).unwrap_or_default()
            );
            if seen.contains(&key) {
                continue;
            }
            seen.push(key);
            let body_len = serde_json::to_string(&parsed).map(|t| t.len()).unwrap_or(0);
            let mut headers = BTreeMap::new();
            let mut stripped_auth = false;
            for (k, v) in &e.request_headers {
                let lower = k.to_ascii_lowercase();
                if lower.starts_with(':') {
                    continue;
                }
                let class = classify_header(&lower);
                if matches!(class, HeaderClass::Credential | HeaderClass::Dropped) {
                    if class == HeaderClass::Credential {
                        stripped_auth = true;
                    }
                    if !dropped_headers.contains(&lower) && lower != "host" {
                        dropped_headers.push(lower);
                    }
                    continue;
                }
                if lower == "referer" {
                    referers.push((
                        format!("s{}r{}", index + 1, requests.len() + 1),
                        k.clone(),
                        v.clone(),
                    ));
                }
                headers.insert(k.clone(), template::templatize(v, &given, Slot::Raw));
            }
            let item_fields = item_fields(&parsed);
            let id = format!("s{}r{}", index + 1, requests.len() + 1);
            // A write's response is not a source: the flow would have to send it.
            if !is_write(&e.method, e.post_data.as_deref()) {
                bodies.push((id.clone(), parsed.clone()));
            }
            request_ids.push(id.clone());
            capture_ids.push((format!("s{}r{}", seq, requests.len() + 1), id.clone()));
            har_ids.insert(id.clone(), e.request_id.clone());
            requests.push(Request {
                id,
                method: e.method.clone(),
                url: template::templatize(&e.url, &given, Slot::Url),
                headers,
                body: e
                    .post_data
                    .as_deref()
                    .map(|b| template::templatize_body(b, &given, &e.request_headers)),
                tier: None,
                expect: Expect {
                    status: e.status.and_then(|s| u16::try_from(s).ok()),
                    mime: e.mime_type.clone(),
                    signature: signature(&parsed),
                    bytes: body_len as u64,
                    item_fields,
                    extract,
                    // A key with a dot in it cannot be told from a nested path.
                    list: main_list(&parsed)
                        .and_then(|(path, _)| path)
                        .filter(|path| !list_gone(path, &parsed)),
                    stripped_auth,
                },
            });
        }
        if requests.is_empty() && INTERACTIVE.contains(&action_of(cmd)) {
            let reason = if input.login_wall && index + 1 == input.commands.len() {
                "the page asked for a login; this step needs an account, and the flow stops before it"
            } else if tagged.is_empty() {
                "no network traffic while this command ran"
            } else if tagged.iter().any(|e| policy.allows_url(&e.url)) {
                "first-party traffic was documents or static files, not a JSON API"
            } else {
                "only third-party traffic"
            };
            // `kind` marks the reason a step with no request gets, so a save
            // can replace it; the save report does not carry it.
            let kind = if reason.starts_with("the page asked") {
                "login-wall"
            } else {
                "no-requests"
            };
            gaps.push(json!({
                "step": index + 1,
                "command": describe_command(cmd),
                "reason": reason,
                "kind": kind,
            }));
        }
        steps.push(Step {
            seq: *seq,
            ok: input.recorder.is_ok(*seq),
            command: cmd.clone(),
            requests,
        });
    }

    if request_ids.is_empty() && !secret_dropped.is_empty() {
        return Err(format!(
            "no first-party JSON requests could be saved for {base}: {} a value typed into a secret field, so not saved. To make a reusable flow, fill a non-secret field or use a declared tool",
            secret_dropped.join(" and ")
        ));
    }
    if request_ids.is_empty() {
        return Err(format!(
            "no first-party JSON requests were recorded for {base}; this flow has nothing to replay"
        ));
    }
    // A page-query value that only a Referer carries is the page's address,
    // not an input of the requests: keep the recorded Referer.
    let unused: Vec<String> = given
        .iter()
        .filter(|(k, _)| {
            !input.params.iter().any(|(n, _)| n == k) && !typed.iter().any(|(n, _)| n == k)
        })
        .map(|(k, _)| k.clone())
        .filter(|k| {
            let placeholder = format!("{{{{{k}}}}}");
            let used = steps.iter().flat_map(|s| s.requests.iter()).any(|r| {
                r.url.contains(&placeholder)
                    || r.body.as_deref().is_some_and(|b| b.contains(&placeholder))
                    || r.headers.iter().any(|(h, v)| {
                        !h.eq_ignore_ascii_case("referer") && v.contains(&placeholder)
                    })
            });
            !used
        })
        .collect();
    if !unused.is_empty() {
        given.retain(|(k, _)| !unused.contains(k));
        for req in steps.iter_mut().flat_map(|s| s.requests.iter_mut()) {
            for (id, key, original) in &referers {
                if *id == req.id {
                    let kept = template::templatize(original, &given, Slot::Raw);
                    req.headers.insert(key.clone(), kept);
                }
            }
        }
    }
    let page_segments: Vec<String> = url::Url::parse(&start)
        .ok()
        .and_then(|u| u.path_segments().map(|p| p.map(String::from).collect()))
        .unwrap_or_default();
    let links = link_requests(&mut steps, &bodies, &har_ids, &page_segments, &given);
    let mut params: Vec<Param> = given
        .iter()
        .map(|(k, v)| {
            if input.params.iter().any(|(n, _)| n == k) {
                Param {
                    name: k.clone(),
                    example: Some(v.clone()),
                    source: ParamSource::Typed,
                    from: None,
                }
            } else if typed.iter().any(|(n, _)| n == k) {
                Param {
                    name: k.clone(),
                    example: None,
                    source: ParamSource::Typed,
                    from: None,
                }
            } else {
                Param {
                    name: k.clone(),
                    example: Some(v.clone()),
                    source: ParamSource::Query,
                    from: Some(ParamFrom {
                        page_query: Some(k.clone()),
                        ..ParamFrom::default()
                    }),
                }
            }
        })
        .collect();
    for (name, _) in &typed {
        if !given.iter().any(|(n, _)| n == name) {
            params.push(Param {
                name: name.clone(),
                example: None,
                source: ParamSource::Typed,
                from: None,
            });
        }
    }
    params.extend(links.params);
    let output = match input.output {
        Some(id) if request_ids.iter().any(|r| r == id) => {
            let url_of = |saved: &str| {
                steps
                    .iter()
                    .flat_map(|s| s.requests.iter())
                    .find(|r| r.id == saved)
                    .map(|r| r.url.clone())
                    .unwrap_or_default()
            };
            match capture_ids.iter().find(|(c, saved)| c == id && saved != id) {
                Some((_, other)) => {
                    return Err(format!(
                        "{id} is ambiguous: saved {id} ({}) or capture {id}, saved as {other} ({}); \
                         use output with one of them after renumbering the range, or save from step 1",
                        url_of(id),
                        url_of(other)
                    ));
                }
                None => id.to_string(),
            }
        }
        Some(id) => match capture_ids.iter().find(|(c, _)| c == id) {
            Some((_, saved)) => saved.clone(),
            None => {
                let mut valid: Vec<String> = request_ids.clone();
                for (c, s) in &capture_ids {
                    if c != s {
                        valid.push(c.clone());
                    }
                }
                return Err(format!(
                    "unknown request id '{id}' for output; valid ids: {}",
                    valid.join(", ")
                ));
            }
        },
        None => default_output(&steps),
    };

    let item_fields = steps
        .iter()
        .flat_map(|s| s.requests.iter())
        .find(|r| r.id == output)
        .map(|r| r.expect.item_fields.clone())
        .unwrap_or_default();
    let writes = collect_writes(&input, &policy, &given, &har_ids);
    let mut flow = Flow {
        version: FLOW_VERSION,
        name: input.name.to_string(),
        origin: host.clone(),
        start_url: start.clone(),
        allowed_hosts: policy.hosts().to_vec(),
        created_at: input.created_at.clone(),
        created_by: Some(CreatedBy {
            tool: "vexdom".to_string(),
            version: crate::VERSION.to_string(),
        }),
        description: input.description.map(String::from),
        redacted_steps: commands
            .iter()
            .enumerate()
            .filter(|(_, (_, c))| c["redacted"] == true)
            .map(|(i, _)| i + 1)
            .collect(),
        params,
        output: output.clone(),
        item_fields,
        provenance: "observed".to_string(),
        verified_at: None,
        writes,
        steps,
    };
    scrub(&mut flow, &input.recorder.secrets);

    let requests_report: Vec<Value> = flow
        .requests()
        .map(|r| json!({ "id": r.id, "method": r.method, "url": r.url, "bytes": r.expect.bytes }))
        .collect();

    let mut report = json!({
        "name": input.name,
        "origin": host,
        "steps": input.commands.len(),
        "requests": requests_report,
        "output": output,
        "params": flow.params.iter().map(|p| p.name.clone()).collect::<Vec<_>>(),
        "detectedParams": flow.params.iter().filter(|p| p.from.is_some()).collect::<Vec<_>>(),
        "references": links.references,
        "writes": flow.writes,
        "gaps": gaps,
        "droppedHeaders": dropped_headers,
        "verified": false,
    });
    // The report names commands from the recording; they pass the same
    // scrub as the file.
    if let Some(gaps) = report["gaps"].as_array_mut() {
        for g in gaps {
            if let Some(text) = g["command"].as_str() {
                let clean = scrub_command_text(text, &input.recorder.secrets);
                g["command"] = json!(clean);
            }
        }
    }
    Ok((flow, report))
}

/// A `describe_command` line with its URL and any known secret scrubbed.
pub(crate) fn scrub_command_text(text: &str, secrets: &[String]) -> String {
    if holds_secret(text, secrets) {
        let action = text.split(' ').next().unwrap_or("");
        return format!("{action} {REDACTED}");
    }
    match text.split_once(' ') {
        Some((action, target)) => match scrub_url(target) {
            Some(clean) => format!("{action} {clean}"),
            None => text.to_string(),
        },
        None => text.to_string(),
    }
}

// ---------------------------------------------------------------------------
// Params and references found in the recording
// ---------------------------------------------------------------------------

struct Links {
    /// Values that came from an item of an earlier list: the agent picks one.
    params: Vec<Param>,
    /// Values that came from a single field of an earlier response: filled in
    /// at run time from that response.
    references: Vec<Value>,
}

/// Values a request sends: query values, path segments and JSON body scalars.
fn sent_values(req: &Request) -> Vec<String> {
    let mut out = Vec::new();
    if let Ok(url) = url::Url::parse(&req.url) {
        out.extend(url.query_pairs().map(|(_, v)| v.into_owned()));
        if let Some(segs) = url.path_segments() {
            out.extend(segs.map(|s| url_decode(s).unwrap_or_default()));
        }
    }
    if let Some(body) = req
        .body
        .as_deref()
        .and_then(|b| serde_json::from_str::<Value>(b).ok())
    {
        collect_scalars(&body, &mut out);
    }
    // Short values (ids like "1", flags like "es") match unrelated fields by chance.
    out.retain(|v| v.chars().count() >= 3 && !v.contains("{{"));
    out.sort();
    out.dedup();
    out
}

fn collect_scalars(v: &Value, out: &mut Vec<String>) {
    match v {
        Value::Object(m) => m.values().for_each(|c| collect_scalars(c, out)),
        Value::Array(a) => a.iter().for_each(|c| collect_scalars(c, out)),
        Value::String(s) => out.push(s.clone()),
        Value::Number(n) => out.push(n.to_string()),
        _ => {}
    }
}

fn looks_generated(v: &str) -> bool {
    let mixed_case =
        v.chars().any(|c| c.is_ascii_uppercase()) && v.chars().any(|c| c.is_ascii_lowercase());
    v.chars().count() >= 6 && (v.chars().any(|c| c.is_ascii_digit()) || mixed_case)
}

/// Replace a value, plain or URL-encoded, with a placeholder.
fn replace_value(text: &str, value: &str, placeholder: &str) -> String {
    let encoded = url_encode(value);
    replace_token(
        &replace_token(text, &encoded, placeholder),
        value,
        placeholder,
    )
}

/// Replace `needle` only where it stands alone, not inside a longer word:
/// `401` in `theater=401`, never in `14011`.
fn replace_token(text: &str, needle: &str, with: &str) -> String {
    if needle.is_empty() {
        return text.to_string();
    }
    let word = |c: char| c.is_ascii_alphanumeric() || c == '_';
    let mut out = String::new();
    let mut rest = text;
    while let Some(i) = rest.find(needle) {
        let before = rest[..i].chars().next_back();
        let after = rest[i + needle.len()..].chars().next();
        let bounded = !before.is_some_and(word) && !after.is_some_and(word);
        out.push_str(&rest[..i]);
        out.push_str(if bounded { with } else { needle });
        rest = &rest[i + needle.len()..];
    }
    out.push_str(rest);
    out
}

/// Where `needle` sits in `body`, as a path like `token` or `data[].sessionId`,
/// and whether the path crosses a list.
fn find_value(body: &Value, needle: &str, path: &str) -> Option<(String, bool)> {
    match body {
        Value::Object(m) => m
            .iter()
            .find_map(|(k, v)| find_value(v, needle, &child_path(path, k))),
        Value::Array(a) => a
            .iter()
            .find_map(|v| find_value(v, needle, &format!("{path}[]")))
            .map(|(p, _)| (p, true)),
        Value::String(s) if s == needle => Some((path.to_string(), false)),
        Value::Number(n) if n.to_string() == needle => Some((path.to_string(), false)),
        _ => None,
    }
}

fn link_requests(
    steps: &mut [Step],
    bodies: &[(String, Value)],
    har_ids: &HashMap<String, String>,
    page_segments: &[String],
    given: &[(String, String)],
) -> Links {
    let mut links = Links {
        params: Vec::new(),
        references: Vec::new(),
    };
    let given_values: Vec<&str> = given.iter().map(|(_, v)| v.as_str()).collect();
    // Path segments of each request, to tell a response value that is the
    // site's own structure (`api`, `v10`) from one the run chooses.
    let segments: HashMap<String, Vec<String>> = steps
        .iter()
        .flat_map(|s| s.requests.iter())
        .map(|r| {
            let segs = url::Url::parse(&r.url)
                .ok()
                .and_then(|u| u.path_segments().map(|p| p.map(String::from).collect()))
                .unwrap_or_default();
            (r.id.clone(), segs)
        })
        .collect();
    // Position of every request, writes included, so a write can take a value
    // from any response that came before it.
    let rank: HashMap<String, usize> = steps
        .iter()
        .flat_map(|s| s.requests.iter())
        .enumerate()
        .map(|(i, r)| (r.id.clone(), i))
        .collect();
    for step in steps.iter_mut() {
        for req in step.requests.iter_mut() {
            let id = req.id.clone();
            let own = rank.get(&id).copied().unwrap_or(0);
            let earlier: Vec<(String, Value)> = bodies
                .iter()
                .filter(|(from, _)| rank.get(from).is_some_and(|r| *r < own))
                .cloned()
                .collect();
            for value in sent_values(req) {
                // The page's own path (owner, repo, section) is the site's
                // structure, not a choice the run makes.
                if given_values.contains(&value.as_str()) || page_segments.contains(&value) {
                    continue;
                }
                // Blocks of the same document are one response, not a chain.
                let same_response = |from: &str| {
                    har_ids.get(from).is_some() && har_ids.get(from) == har_ids.get(&id)
                };
                // The latest response that holds the value supplies it, except
                // an echo: a response whose own request path already has the
                // value just repeats it. A list is preferred over a single field.
                let sources: Vec<(String, String, bool)> = earlier
                    .iter()
                    .rev()
                    .filter(|(from, _)| !same_response(from))
                    .filter(|(from, _)| {
                        !segments.get(from).is_some_and(|segs| segs.contains(&value))
                    })
                    .filter_map(|(from, body)| {
                        find_value(body, &value, "").map(|(p, l)| (from.clone(), p, l))
                    })
                    .collect();
                let Some((from, path, in_list)) = sources
                    .iter()
                    .find(|(_, _, l)| *l)
                    .or(sources.first())
                    .cloned()
                else {
                    continue;
                };
                // A value in the path of a request sent before that response
                // could have supplied it, or of the request it came from, is
                // structure rather than data from that response.
                let own_rank = rank.get(&from).copied().unwrap_or(0);
                let structural = segments.iter().any(|(rid, segs)| {
                    rank.get(rid).is_some_and(|r| *r <= own_rank) && segs.contains(&value)
                });
                if structural {
                    continue;
                }
                // Words and codes like "es-AR" or "public" repeat across a site's
                // responses; only generated values (ids, keys) are worth chaining.
                if !in_list && !looks_generated(&value) {
                    continue;
                }
                let placeholder = if in_list {
                    let field = path
                        .rsplit(['.', ']'])
                        .next()
                        .map(|f| {
                            f.chars()
                                .filter(|c| c.is_ascii_alphanumeric() || *c == '_')
                                .collect::<String>()
                        })
                        .filter(|f| !f.is_empty())
                        .unwrap_or_else(|| "value".to_string());
                    let name = match links
                        .params
                        .iter()
                        .find(|p| p.example.as_deref() == Some(value.as_str()))
                    {
                        Some(p) => p.name.clone(),
                        None => {
                            let mut name = field.clone();
                            let mut n = 2;
                            while links.params.iter().any(|p| p.name == name)
                                || given.iter().any(|(k, _)| *k == name)
                            {
                                name = format!("{field}{n}");
                                n += 1;
                            }
                            links.params.push(Param {
                                name: name.clone(),
                                example: Some(value.clone()),
                                source: ParamSource::Response,
                                from: Some(ParamFrom {
                                    request: Some(from.clone()),
                                    list: Some(path.clone()),
                                    ..ParamFrom::default()
                                }),
                            });
                            name
                        }
                    };
                    format!("{{{{{name}}}}}")
                } else {
                    links
                        .references
                        .push(json!({ "request": id, "from": from, "path": path }));
                    format!("{{{{@{from}.{path}}}}}")
                };
                req.url = replace_value(&req.url, &value, &placeholder);
                req.body = req
                    .body
                    .as_deref()
                    .map(|b| replace_value(b, &value, &placeholder));
            }
        }
    }
    links
}

/// Request ids a request needs answers from, in run order.
pub(crate) fn dependencies(flow: &Flow, id: &str) -> Vec<String> {
    // A flow with no output answers with nothing in particular, so a run
    // sends every request and every write goes through the gate.
    if id.is_empty() {
        return flow.requests().map(|r| r.id.clone()).collect();
    }
    let refs_of = |rid: &str| -> Vec<String> {
        flow.request(rid)
            .map(|r| {
                let text = format!("{} {}", r.url, r.body.as_deref().unwrap_or(""));
                text.match_indices("{{@")
                    .filter_map(|(i, _)| text[i + 3..].split('.').next().map(String::from))
                    .collect()
            })
            .unwrap_or_default()
    };
    let mut needed = vec![id.to_string()];
    let mut i = 0;
    while i < needed.len() {
        for dep in refs_of(&needed[i].clone()) {
            if !needed.contains(&dep) {
                needed.push(dep);
            }
        }
        i += 1;
    }
    flow.requests()
        .map(|r| r.id.clone())
        .filter(|r| needed.iter().any(|n| n == r))
        .collect()
}

/// Field names an agent can pass to `where` and `fields`: the keys of the
/// first item of the response's main list, with one level of dotted paths.
fn item_fields(body: &Value) -> Vec<String> {
    let Some((_, items)) = main_list(body) else {
        return Vec::new();
    };
    let Some(first) = items.first().and_then(|i| i.as_object()) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for (k, v) in first {
        match v.as_object() {
            Some(child) if !child.is_empty() => {
                out.extend(child.keys().map(|c| format!("{k}.{c}")))
            }
            _ => out.push(k.clone()),
        }
    }
    out
}

/// The request the flow answers with when the agent does not pick one: the
/// largest response among requests that use a param, else the largest.
fn default_output(steps: &[Step]) -> String {
    let requests: Vec<&Request> = steps.iter().flat_map(|s| s.requests.iter()).collect();
    let uses_param =
        |r: &&Request| r.url.contains("{{") || r.body.as_deref().is_some_and(|b| b.contains("{{"));
    let pool: Vec<&Request> = if requests.iter().any(uses_param) {
        requests.iter().copied().filter(uses_param).collect()
    } else {
        requests
    };
    pool.iter()
        .max_by_key(|r| r.expect.bytes)
        .map(|r| r.id.clone())
        .unwrap_or_default()
}

// ---------------------------------------------------------------------------
// Output shaping
// ---------------------------------------------------------------------------

/// `where` (case-insensitive substring per field), `fields` and `limit`.
#[derive(Default)]
pub(crate) struct Shape {
    pub filters: Vec<(String, String)>,
    pub fields: Vec<String>,
    pub limit: Option<usize>,
}

impl Shape {
    pub(crate) fn from_cmd(cmd: &Value) -> Shape {
        let filters = cmd
            .get("where")
            .and_then(|v| v.as_object())
            .map(|o| {
                o.iter()
                    .map(|(k, v)| (k.clone(), v.as_str().unwrap_or("").to_lowercase()))
                    .collect()
            })
            .unwrap_or_default();
        let fields = match cmd.get("fields") {
            Some(Value::String(f)) => f
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect(),
            Some(Value::Array(items)) => items
                .iter()
                .filter_map(Value::as_str)
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect(),
            _ => Vec::new(),
        };
        let limit = cmd
            .get("limit")
            .and_then(|v| v.as_u64())
            .map(|n| n as usize);
        Shape {
            filters,
            fields,
            limit,
        }
    }

    fn is_empty(&self) -> bool {
        self.filters.is_empty() && self.fields.is_empty() && self.limit.is_none()
    }
}

pub(crate) fn lookup<'a>(v: &'a Value, path: &str) -> Option<&'a Value> {
    path.split('.').try_fold(v, |cur, key| cur.get(key))
}

pub(crate) fn as_text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// The list the response is about: the body itself if it is an array, else
/// the heaviest list of objects anywhere in it (GraphQL nests it under `data`).
pub(crate) fn main_list(body: &Value) -> Option<(Option<String>, &Vec<Value>)> {
    if let Some(items) = body.as_array() {
        return Some((None, items));
    }
    let mut best: Option<(String, &Vec<Value>)> = None;
    collect_lists(body, "", &mut best);
    best.map(|(k, v)| (Some(k), v))
}

fn collect_lists<'a>(v: &'a Value, path: &str, best: &mut Option<(String, &'a Vec<Value>)>) {
    let Some(map) = v.as_object() else { return };
    for (k, child) in map {
        let here = child_path(path, k);
        match child {
            Value::Array(items) if items.first().is_some_and(|i| i.is_object()) => {
                // The heaviest list, not the longest: GraphQL metadata (cache
                // hints, errors) can have more entries than the data.
                let weight =
                    |l: &Vec<Value>| serde_json::to_string(l).map(|t| t.len()).unwrap_or(0);
                if best.as_ref().is_none_or(|(_, b)| weight(items) > weight(b)) {
                    *best = Some((here, items));
                }
            }
            Value::Object(_) => collect_lists(child, &here, best),
            _ => {}
        }
    }
}

/// Whether the list at the recorded path in `body` is gone: absent or no
/// longer an array. A list that is present and empty is not gone.
pub(crate) fn list_gone(path: &str, body: &Value) -> bool {
    !lookup(body, path).is_some_and(|v| v.is_array())
}

/// Field names used in `where` or `fields` that no item has.
pub(crate) fn unknown_fields(body: &Value, shape: &Shape) -> Vec<String> {
    let Some((_, items)) = main_list(body) else {
        return Vec::new();
    };
    if items.is_empty() {
        return Vec::new();
    }
    shape
        .filters
        .iter()
        .map(|(f, _)| f)
        .chain(shape.fields.iter())
        .filter(|f| !items.iter().any(|item| lookup(item, f).is_some()))
        .cloned()
        .collect()
}

pub(crate) fn apply_shape(body: Value, shape: &Shape) -> Value {
    if shape.is_empty() {
        return body;
    }
    let Some((key, items)) = main_list(&body) else {
        return body;
    };
    let total = items.len();
    let mut matched = 0usize;
    let mut kept = Vec::new();
    let matching = items.iter().filter(|item| {
        shape.filters.iter().all(|(path, needle)| {
            lookup(item, path)
                .map(|v| as_text(v).to_lowercase().contains(needle.as_str()))
                .unwrap_or(false)
        })
    });
    for item in matching {
        matched += 1;
        if shape.limit.is_some_and(|limit| kept.len() >= limit) {
            continue;
        }
        if shape.fields.is_empty() {
            kept.push(item.clone());
        } else {
            let mut out = Map::new();
            for f in &shape.fields {
                if let Some(v) = lookup(item, f) {
                    out.insert(f.clone(), v.clone());
                }
            }
            kept.push(Value::Object(out));
        }
    }
    json!({ "list": key, "total": total, "matched": matched, "items": kept })
}

// ---------------------------------------------------------------------------
// Storage
// ---------------------------------------------------------------------------

/// `~/.vexdom/flows`: one folder per origin host, one file per flow.
pub(crate) fn flows_dir() -> PathBuf {
    paths::state_dir().join("flows")
}

pub(crate) fn validate_name(name: &str) -> Result<(), String> {
    if name.is_empty()
        || !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(format!(
            "invalid flow name '{name}': use letters, digits, '-' or '_'"
        ));
    }
    Ok(())
}

pub(crate) fn flow_path(dir: &Path, origin: &str, name: &str) -> Result<PathBuf, String> {
    validate_origin(origin)?;
    validate_name(name)?;
    Ok(dir.join(origin).join(format!("{name}.json")))
}

/// Flow files hold recorded URLs and form values, so only the owner can read
/// them, from the moment they are created.
pub(crate) fn write_flow_in(dir: &Path, flow: &Flow) -> Result<PathBuf, String> {
    // Whatever produced the flow, nothing sensitive reaches the file.
    let mut flow = flow.clone();
    scrub(&mut flow, &[]);
    let path = flow_path(dir, &flow.origin, &flow.name)?;
    let text = serde_json::to_string_pretty(&flow).map_err(|e| e.to_string())?;
    paths::write_private(&path, text.as_bytes())
        .map_err(|e| format!("Failed to write {}: {e}", path.display()))?;
    #[cfg(unix)]
    for folder in [path.parent(), Some(dir)].into_iter().flatten() {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(folder, std::fs::Permissions::from_mode(0o700));
    }
    Ok(path)
}

/// The one pass over everything a flow file stores, run before it is
/// written. A known secret, in any encoding, anywhere in a command, a
/// request or a write becomes `<redacted>`. Credential query parameters
/// (a token in a navigation's query) are redacted where the browser went,
/// not where the run must send: in commands, the start URL, the Referer and
/// Origin headers and the writes, never in a saved request's URL, which the
/// run sends as recorded. A param named like a credential keeps no recorded
/// value, so a run must pass it. A step whose command it touches is marked
/// redacted, so the command replay never sends it.
pub(crate) fn scrub(flow: &mut Flow, secrets: &[String]) {
    flow.start_url = scrub_url(&flow.start_url).unwrap_or_else(|| flow.start_url.clone());
    for (index, step) in flow.steps.iter_mut().enumerate() {
        let hit = scrub_strings(&mut step.command, secrets);
        for r in &mut step.requests {
            if holds_secret(&r.url, secrets) {
                r.url = REDACTED.to_string();
            }
            if r.body.as_deref().is_some_and(|b| holds_secret(b, secrets)) {
                r.body = Some(REDACTED.to_string());
            }
            for (name, v) in r.headers.iter_mut() {
                if holds_secret(v, secrets) {
                    *v = REDACTED.to_string();
                } else if matches!(name.to_ascii_lowercase().as_str(), "referer" | "origin")
                    && let Some(clean) = scrub_url(v)
                {
                    // The page the browser was on, not something the run sends.
                    *v = clean;
                }
            }
        }
        if hit {
            step.command["redacted"] = json!(true);
            if !flow.redacted_steps.contains(&(index + 1)) {
                flow.redacted_steps.push(index + 1);
            }
        }
    }
    flow.redacted_steps.sort_unstable();
    for w in &mut flow.writes {
        if holds_secret(&w.url, secrets) {
            w.url = REDACTED.to_string();
        } else if let Some(clean) = scrub_url(&w.url) {
            w.url = clean;
        }
    }
    for p in &mut flow.params {
        if p.example.as_deref().is_some_and(|e| e != REDACTED)
            && (is_credential_param(&p.name)
                || p.example
                    .as_deref()
                    .is_some_and(|e| holds_secret(e, secrets)))
        {
            p.example = None;
        }
    }
}

/// Whether a URL had a credential parameter redacted by the scrub.
pub(crate) fn is_redacted_url(url: &str) -> bool {
    url.contains(REDACTED) || url.contains(&url_encode(REDACTED))
}

/// Replace every string under `v` that holds a secret; true if any did.
fn scrub_strings(v: &mut Value, secrets: &[String]) -> bool {
    match v {
        Value::String(s) if s != REDACTED && holds_secret(s, secrets) => {
            *s = REDACTED.to_string();
            true
        }
        Value::String(s) => match scrub_url(s) {
            Some(clean) => {
                *s = clean;
                true
            }
            None => false,
        },
        // Every item is visited: stopping at the first hit would leave the
        // rest unscrubbed.
        Value::Array(items) => {
            let mut hit = false;
            for item in items {
                hit |= scrub_strings(item, secrets);
            }
            hit
        }
        Value::Object(map) => {
            let mut hit = false;
            for item in map.values_mut() {
                hit |= scrub_strings(item, secrets);
            }
            hit
        }
        _ => false,
    }
}

/// The URL with credential parameter values redacted, if it had any, in
/// its query or in a fragment of `k=v` pairs (an OAuth implicit-grant
/// callback). Only those values change; the rest stays byte for byte, so a
/// template (`{{name}}`, `{{@s1r1.token}}`), which a run fills, is kept, and
/// so is a name with no value.
fn scrub_url(url: &str) -> Option<String> {
    if !url.starts_with("http://") && !url.starts_with("https://") {
        return None;
    }
    let (before_fragment, fragment) = match url.split_once('#') {
        Some((b, f)) => (b, Some(f)),
        None => (url, None),
    };
    let (head, query) = match before_fragment.split_once('?') {
        Some((h, q)) => (h, Some(q)),
        None => (before_fragment, None),
    };
    let query = query.map(scrub_pairs);
    // A hash-router fragment (`#/callback?k=v`) carries its pairs after `?`.
    let fragment = fragment.map(|f| match f.split_once('?') {
        Some((route, pairs)) => {
            let (pairs, hit) = scrub_pairs(pairs);
            (format!("{route}?{pairs}"), hit)
        }
        None => scrub_pairs(f),
    });
    let changed = query.as_ref().is_some_and(|q| q.1) || fragment.as_ref().is_some_and(|f| f.1);
    changed.then(|| {
        let mut out = head.to_string();
        if let Some((q, _)) = query {
            out.push('?');
            out.push_str(&q);
        }
        if let Some((f, _)) = fragment {
            out.push('#');
            out.push_str(&f);
        }
        out
    })
}

/// `k=v&k=v` with credential values redacted, and whether any were.
fn scrub_pairs(pairs: &str) -> (String, bool) {
    let redacted = url_encode(REDACTED);
    let mut changed = false;
    let parts: Vec<String> = pairs
        .split('&')
        .map(|part| {
            let Some((k, v)) = part.split_once('=') else {
                return part.to_string();
            };
            let name = url::form_urlencoded::parse(k.as_bytes())
                .next()
                .map(|(k, _)| k.into_owned())
                .unwrap_or_default();
            let value = percent_decode_lossless(&v.replace('+', " "));
            if is_credential_param(&name)
                && !value.is_empty()
                && value != REDACTED
                && !value.contains("{{")
            {
                changed = true;
                format!("{k}={redacted}")
            } else {
                part.to_string()
            }
        })
        .collect();
    (parts.join("&"), changed)
}

/// A query parameter that carries a credential: named like a credential
/// header (`authorization`, anything with `token` or `csrf`) except a
/// pagination cursor (`pageToken`, `nextPageToken`, `continuation_token`), or
/// one word of its name is a secret-field marker (`pass`, `secret`, `otp`,
/// `pin`, `cvv`, `card`), `password`, or a common credential name (`auth`,
/// `apikey`, `signature`, `jwt`). Bare `key`, `sig` and `sid` are left out:
/// publishable keys (Algolia, Google) ride in the query. Words split on
/// punctuation and match whole, so `passengers` and `shipping` are not
/// secrets.
fn is_credential_param(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    let squashed: String = lower
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect();
    let cursor = squashed.ends_with("pagetoken")
        || squashed.contains("continuation")
        || squashed.contains("cursor");
    if cursor {
        return false;
    }
    if classify_header(name) == HeaderClass::Credential {
        return true;
    }
    lower
        .split(|c: char| !c.is_ascii_alphanumeric())
        .any(|word| {
            recorder::SECRET_MARKERS.contains(&word)
                || matches!(word, "password" | "auth" | "apikey" | "signature" | "jwt")
        })
}

/// A flow file on disk, with the directory it was found in.
struct StoredFlow {
    dir_origin: String,
    flow: Flow,
}

impl StoredFlow {
    /// The directory, the file's origin and its start URL must agree.
    fn is_consistent(&self) -> bool {
        self.flow.origin == self.dir_origin
            && host_of(&self.flow.start_url).as_deref() == Some(self.dir_origin.as_str())
    }
}

fn stored_flows(dir: &Path, origin: Option<&str>) -> Vec<StoredFlow> {
    let mut out = Vec::new();
    let Ok(origins) = std::fs::read_dir(dir) else {
        return out;
    };
    let mut dirs: Vec<_> = origins.flatten().filter(|d| d.path().is_dir()).collect();
    dirs.sort_by_key(|d| d.file_name());
    for folder in dirs {
        let dir_origin = folder.file_name().to_string_lossy().to_string();
        if origin.is_some_and(|o| base_domain(o) != base_domain(&dir_origin)) {
            continue;
        }
        let Ok(files) = std::fs::read_dir(folder.path()) else {
            continue;
        };
        let mut files: Vec<_> = files.flatten().collect();
        files.sort_by_key(|f| f.file_name());
        for f in files {
            let Ok(text) = std::fs::read_to_string(f.path()) else {
                continue;
            };
            let Ok(flow) = serde_json::from_str::<Flow>(&text) else {
                continue;
            };
            out.push(StoredFlow {
                dir_origin: dir_origin.clone(),
                flow,
            });
        }
    }
    out
}

/// Saved flows as the summaries `flowList` shows.
pub(crate) fn list_flows(dir: &Path, origin: Option<&str>) -> Vec<Value> {
    stored_flows(dir, origin)
        .into_iter()
        .map(|stored| {
            let flow = &stored.flow;
            json!({
                "name": flow.name,
                "origin": stored.dir_origin,
                "valid": stored.is_consistent(),
                "params": flow.params.iter().map(|p| p.name.clone()).collect::<Vec<_>>(),
                "examples": flow.params.iter()
                    .map(|p| (p.name.clone(), json!(p.example.clone().unwrap_or_else(|| "<value>".to_string()))))
                    .collect::<Map<String, Value>>(),
                "description": tool_description(flow),
                "inputSchema": input_schema(flow),
                "paramSources": flow.params.iter()
                    .filter_map(|p| {
                        let from = p.from.as_ref()?;
                        Some((p.name.clone(), json!(format!("{} {}", from.request.as_ref()?, from.list.as_ref()?))))
                    })
                    .collect::<Map<String, Value>>(),
                "itemFields": flow.item_fields,
                "recordedOn": flow.start_url,
                "provenance": flow.provenance,
                "verifiedAt": flow.verified_at,
            })
        })
        .collect()
}

/// A one-line description in the shape of a WebMCP tool's.
fn tool_description(flow: &Flow) -> String {
    if let Some(text) = flow.description.as_deref().filter(|t| !t.trim().is_empty()) {
        return text.chars().take(300).collect();
    }
    let mut out = format!("Runs the task recorded on {}", flow.start_url);
    if !flow.item_fields.is_empty() {
        out.push_str(&format!(
            "; returns items with {}",
            flow.item_fields.join(", ")
        ));
    }
    out
}

/// The flow's params as a JSON Schema, the same shape WebMCP tools declare.
/// A param with a recorded value is optional; a typed one must be passed.
pub(crate) fn input_schema(flow: &Flow) -> Value {
    let mut properties = Map::new();
    for p in &flow.params {
        let mut description = match p.example.as_deref() {
            Some(example) => format!("Recorded value: {example}"),
            None => "Typed during the recording; pass a value".to_string(),
        };
        if let Some(list) = p.from.as_ref().and_then(|f| f.list.as_deref()) {
            description.push_str(&format!(". One of the site's {list} values"));
        }
        properties.insert(
            p.name.clone(),
            json!({ "type": "string", "description": description }),
        );
    }
    json!({ "type": "object", "properties": properties, "additionalProperties": false })
}

/// Why a file named like the flow cannot be read, when one exists.
fn unreadable_flow(dir: &Path, name: &str, origin: Option<&str>) -> Option<String> {
    let dirs = std::fs::read_dir(dir).ok()?;
    for folder in dirs.flatten().filter(|d| d.path().is_dir()) {
        let host = folder.file_name().to_string_lossy().to_string();
        if origin.is_some_and(|o| base_domain(o) != base_domain(&host)) {
            continue;
        }
        let path = folder.path().join(format!("{name}.json"));
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        if let Err(e) = serde_json::from_str::<Flow>(&text) {
            return Some(format!("{} is not a valid flow: {e}", path.display()));
        }
    }
    None
}

/// A saved flow by name, or why it cannot be run: `(code, message)`.
pub(crate) fn load_flow(
    dir: &Path,
    name: &str,
    origin: Option<&str>,
) -> Result<Flow, (&'static str, String)> {
    validate_name(name).map_err(|e| ("INVALID_INPUT", e))?;
    let mut matches: Vec<StoredFlow> = stored_flows(dir, origin)
        .into_iter()
        .filter(|s| s.flow.name == name)
        .collect();
    match matches.len() {
        0 => Err(match unreadable_flow(dir, name, origin) {
            Some(reason) => ("ACTION_FAILED", reason),
            None => (
                "FLOW_NOT_FOUND",
                format!("no saved flow named '{name}'. Run `vexdom flow list` to see saved flows"),
            ),
        }),
        1 => {
            let stored = matches.remove(0);
            if !stored.is_consistent() {
                return Err((
                    "ACTION_FAILED",
                    format!(
                        "flow '{name}' does not match its origin directory {}; re-save it",
                        stored.dir_origin
                    ),
                ));
            }
            if stored.flow.version != FLOW_VERSION {
                return Err((
                    "ACTION_FAILED",
                    format!(
                        "flow '{name}' has unsupported version {}",
                        stored.flow.version
                    ),
                ));
            }
            // A request that changes state must be listed as a write, or a run
            // would send it with no gate.
            if let Some(r) = stored.flow.requests().find(|r| {
                is_write(&r.method, r.body.as_deref()) && stored.flow.write(&r.id).is_none()
            }) {
                return Err((
                    "ACTION_FAILED",
                    format!(
                        "flow '{name}' sends {} {} without listing it as a write; re-save it",
                        r.method, r.url
                    ),
                ));
            }
            Ok(stored.flow)
        }
        _ => Err((
            "INVALID_INPUT",
            format!(
                "flow '{name}' exists for several origins ({}); pass origin",
                matches
                    .iter()
                    .map(|m| m.dir_origin.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        )),
    }
}

// ---------------------------------------------------------------------------
// Run
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub(crate) struct PreparedRequest {
    pub id: String,
    pub step: usize,
    pub method: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Option<String>,
    pub expect: Expect,
    /// A run needs this request allowed before it sends it.
    pub write: bool,
    /// The templates the text above was bound from; references stay
    /// unbound until the run has the responses.
    pub parts: Option<Parts>,
    /// Headers (lowercase name, value) that the flow records identically on two
    /// or more requests: those do not change per request.
    pub repeated_headers: Vec<(String, String)>,
}

/// Requests to send for a run: the one the flow answers with, and the ones
/// whose responses it takes values from.
pub(crate) fn prepare(
    flow: &Flow,
    cmd_params: &[(String, String)],
) -> Result<Vec<PreparedRequest>, String> {
    let needed = dependencies(flow, &flow.output);
    Ok(prepare_all(flow, cmd_params)?
        .into_iter()
        .filter(|r| needed.contains(&r.id))
        .collect())
}

pub(crate) fn prepare_all(
    flow: &Flow,
    cmd_params: &[(String, String)],
) -> Result<Vec<PreparedRequest>, String> {
    let mut params: HashMap<String, String> = flow
        .params
        .iter()
        .filter_map(|p| Some((p.name.clone(), p.example.clone()?)))
        .collect();
    let known: Vec<String> = flow.params.iter().map(|p| p.name.clone()).collect();
    for (k, v) in cmd_params {
        if !known.contains(k) {
            return Err(format!(
                "flow '{}' has no param '{k}' (params: {})",
                flow.name,
                if known.is_empty() {
                    "none".to_string()
                } else {
                    known.join(", ")
                }
            ));
        }
        params.insert(k.clone(), v.clone());
    }

    let mut seen: HashMap<(String, String), usize> = HashMap::new();
    for r in flow.requests() {
        for (k, v) in &r.headers {
            *seen.entry((k.to_ascii_lowercase(), v.clone())).or_default() += 1;
        }
    }
    let repeated: Vec<(String, String)> = seen
        .into_iter()
        .filter(|(_, n)| *n >= 2)
        .map(|(kv, _)| kv)
        .collect();

    let mut out = Vec::new();
    for (index, step) in flow.steps.iter().enumerate() {
        for r in &step.requests {
            let header_list: Vec<(String, String)> = r
                .headers
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect();
            let headers = header_list
                .iter()
                .filter_map(|(k, v)| {
                    let bound =
                        template::bind_params(&template::parse(v), Slot::Raw, None, &params);
                    Some((k.clone(), template::show(&bound.ok()?)))
                })
                .collect();
            let url = template::bind_params(&template::parse(&r.url), Slot::Url, None, &params)?;
            let body = match &r.body {
                Some(b) => Some(Body::parse(b, &header_list).bind_params(&params)?),
                None => None,
            };
            let url_text = template::show(&url);
            let body_text = body.as_ref().map(|b| b.show());
            let write = flow.write(&r.id).is_some();
            if !write && becomes_write(&r.method, &url_text, body_text.as_deref()) {
                return Err(not_listed_as_write(&r.id));
            }
            out.push(PreparedRequest {
                id: r.id.clone(),
                step: index + 1,
                method: r.method.clone(),
                url: url_text,
                headers,
                body: body_text,
                expect: r.expect.clone(),
                write,
                parts: Some(Parts { url, body }),
                repeated_headers: repeated.clone(),
            });
        }
    }
    Ok(out)
}

impl PreparedRequest {
    /// Fill reference placeholders from the responses received so far.
    pub(crate) fn resolve(
        &self,
        bodies: &HashMap<String, Value>,
    ) -> Result<PreparedRequest, String> {
        let parts = self
            .parts
            .clone()
            .unwrap_or_else(|| Parts::of_text(&self.url, self.body.as_deref(), &self.headers));
        let mut out = self.clone();
        out.url = template::bind_refs(&parts.url, Slot::Url, None, bodies)?;
        if let Some(body) = &parts.body {
            out.body = Some(body.bind_refs(bodies)?);
        }
        if !out.write && becomes_write(&out.method, &out.url, out.body.as_deref()) {
            return Err(not_listed_as_write(&out.id));
        }
        Ok(out)
    }
}

/// A request that is a write, or a GET whose path names a state change.
fn becomes_write(method: &str, url: &str, body: Option<&str>) -> bool {
    is_write(method, body) || (method == "GET" && has_mutating_segment(url))
}

fn not_listed_as_write(id: &str) -> String {
    format!(
        "request {id} is a write once its values are filled in, but the flow does not list it as one; it was not sent"
    )
}

const MUTATING_SEGMENTS: &[&str] = &["logout", "signout", "unsubscribe", "delete", "remove"];

/// Whether a path has a whole segment that names a state change. Substrings
/// do not count: `/address` is not `/add`.
fn has_mutating_segment(url: &str) -> bool {
    url::Url::parse(url).is_ok_and(|u| {
        u.path_segments().is_some_and(|mut segments| {
            segments.any(|s| MUTATING_SEGMENTS.contains(&s.to_ascii_lowercase().as_str()))
        })
    })
}

/// The writes a recording caused, kept as requests or not: each one is
/// sent again by a run (or by the command replay) and must be allowed.
/// `suspected` marks a GET whose path has a whole mutating segment.
fn collect_writes(
    input: &SaveInput<'_>,
    policy: &HostPolicy,
    given: &[(String, String)],
    har_ids: &HashMap<String, String>,
) -> Vec<Write> {
    let mut writes: Vec<Write> = Vec::new();
    let mut seen: Vec<String> = Vec::new();
    for (index, (seq, _)) in input.commands.iter().enumerate() {
        let mut extra = 0;
        let tagged = input
            .entries
            .iter()
            .filter(|e| input.recorder.request_tags.get(&e.request_id) == Some(seq));
        for e in tagged {
            if carries_secret(e, &input.recorder.secrets)
                || input.recorder.unknown_secret_seqs.contains(seq)
                || is_telemetry(&e.url)
            {
                continue;
            }
            let kept = har_ids
                .iter()
                .find(|(_, har)| **har == e.request_id)
                .map(|(id, _)| id.clone());
            let first_party = policy.allows_url(&e.url);
            let explicit = is_write(&e.method, e.post_data.as_deref());
            let suspected =
                !explicit && e.method == "GET" && first_party && has_mutating_segment(&e.url);
            if !explicit && !suspected {
                continue;
            }
            let key = format!(
                "{} {} {}",
                e.method,
                e.url,
                e.post_data.as_deref().unwrap_or("")
            );
            // A repeat that only the replay sends is listed once; a saved
            // request is always listed, or the flow would not load.
            if kept.is_none() && seen.contains(&key) {
                continue;
            }
            seen.push(key);
            let id = kept.unwrap_or_else(|| {
                extra += 1;
                format!("s{}w{extra}", index + 1)
            });
            writes.push(Write {
                id,
                method: e.method.clone(),
                url: template::templatize(&e.url, given, Slot::Url),
                kind: if explicit {
                    WriteKind::Explicit
                } else {
                    WriteKind::Suspected
                },
                step: index + 1,
            });
        }
    }
    writes
}

impl Flow {
    /// The writes a run sends from the page: the ones among the requests the
    /// output needs. Each must be allowed or approved first.
    pub(crate) fn run_writes(&self) -> Vec<&Write> {
        let needed = dependencies(self, &self.output);
        self.writes
            .iter()
            .filter(|w| needed.contains(&w.id))
            .collect()
    }

    /// Writes the command replay would send beyond the ones a run sends itself.
    pub(crate) fn replay_only_writes(&self) -> Vec<&Write> {
        let own = self.run_writes();
        self.writes
            .iter()
            .filter(|w| !own.iter().any(|o| o.id == w.id))
            .collect()
    }
}

/// Which writes `allowWrite` covers: `all`, or a list of write ids.
#[derive(Default, Clone, Debug, PartialEq)]
pub(crate) struct AllowWrite {
    pub all: bool,
    pub ids: Vec<String>,
}

impl AllowWrite {
    pub(crate) fn parse(value: Option<&Value>) -> Result<AllowWrite, String> {
        let usage = "allowWrite takes \"all\" or a list of write ids";
        let items: Vec<String> = match value {
            None | Some(Value::Null) | Some(Value::Bool(false)) => {
                return Ok(AllowWrite::default());
            }
            Some(Value::String(text)) => text.split(',').map(|t| t.trim().to_string()).collect(),
            Some(Value::Array(items)) => items
                .iter()
                .map(|i| i.as_str().map(|t| t.trim().to_string()).ok_or(usage))
                .collect::<Result<_, _>>()?,
            Some(_) => return Err(usage.to_string()),
        };
        let items: Vec<String> = items.into_iter().filter(|i| !i.is_empty()).collect();
        if items.is_empty() {
            return Err(usage.to_string());
        }
        if items.iter().any(|i| i == "all") {
            return Ok(AllowWrite {
                all: true,
                ids: Vec::new(),
            });
        }
        Ok(AllowWrite {
            all: false,
            ids: items,
        })
    }

    pub(crate) fn covers(&self, id: &str) -> bool {
        self.all || self.ids.iter().any(|i| i == id)
    }

    /// Every id given must name a write of the flow.
    pub(crate) fn check(&self, flow: &Flow) -> Result<(), String> {
        for id in &self.ids {
            if flow.write(id).is_none() {
                let known: Vec<&str> = flow.writes.iter().map(|w| w.id.as_str()).collect();
                return Err(format!(
                    "allowWrite names '{id}', which is not a write of this flow. Writes: {}",
                    if known.is_empty() {
                        "none".to_string()
                    } else {
                        known.join(", ")
                    }
                ));
            }
        }
        Ok(())
    }
}

pub(crate) fn is_write(method: &str, body: Option<&str>) -> bool {
    !matches!(method, "GET" | "HEAD" | "OPTIONS") && !(method == "POST" && is_graphql_query(body))
}

/// A GraphQL POST that only runs queries reads data; mutations still count
/// as writes, and so does an operation with a placeholder in its text, since
/// a value could turn it into one.
fn is_graphql_query(body: Option<&str>) -> bool {
    let Some(parsed) = body.and_then(|b| serde_json::from_str::<Value>(b).ok()) else {
        return false;
    };
    let ops: Vec<&Value> = match &parsed {
        Value::Array(items) => items.iter().collect(),
        other => vec![other],
    };
    !ops.is_empty()
        && ops.iter().all(|op| {
            op["query"].as_str().is_some_and(|q| {
                let q = q.trim_start();
                (q.starts_with("query") || q.starts_with('{'))
                    && !q.contains("mutation")
                    && !q.contains("{{")
            })
        })
}

/// A flow from a partial fixture: the required fields are filled in.
#[cfg(test)]
pub(crate) fn flow_of(mut v: Value) -> Flow {
    let o = v.as_object_mut().expect("a fixture is a JSON object");
    for (k, d) in [
        ("version", json!(FLOW_VERSION)),
        ("name", json!("f")),
        ("origin", json!("a.test")),
        ("startUrl", json!("https://a.test/")),
        ("steps", json!([])),
    ] {
        o.entry(k).or_insert(d);
    }
    serde_json::from_value(v).unwrap_or_else(|e| panic!("fixture is not a flow: {e}"))
}

#[cfg(test)]
mod tests;
