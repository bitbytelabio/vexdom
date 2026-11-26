//! `vexdom native-host`: the process a browser launches (`native-messaging.ts`). One framed message
//! in, one framed reply out, then exit. Stdout carries only that reply and stderr carries nothing,
//! because a browser reads a stray byte as a broken host, and Firefox up to 157 on macOS can hang on
//! a host's stderr (bug 1981177).
//!
//! The browser launches it only for the extension ids its registration lists, so a request that
//! arrives here comes from our own extension, in a browser running as this user. That is what lets
//! `pair` hand out a grant with no code typed.

use std::io::{Read as _, Write as _};
use std::process::ExitCode;
use std::time::Duration;

use serde_json::{Value, json};
use vexdom_core::pairing::{FIREFOX_CALLER, HOST_OPS, is_pair_reason};
use vexdom_core::proto::{FIREFOX_ADDON_ID, MAX_PAIR_WAIT_MS, is_install_id};
use vexdom_core::{config, lockfile, log};
use vexdom_daemon::ensure;

use vexdom_client::{Bridge, block_on};

const STOPPED: &str =
    "Stopped with \"vexdom stop\"; \"vexdom start\" lets the browser start it again.";
const OFF: &str =
    "Automatic pairing is off on this computer. Pair with a code from \"vexdom pair\" or the app.";
const STATUS_TIMEOUT: Duration = Duration::from_secs(3);
const GRANT_TIMEOUT_MS: u64 = 5_000;
/// The longest a host takes to answer, past any wait it was asked to hold; the extension waits a
/// little longer.
const HOST_DEADLINE_MS: u64 = 12_000;

/// Who launched this host, from the arguments the browser passes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Caller {
    Chromium(String),
    Firefox,
}

impl Caller {
    /// What a grant is bound to: the exact origin for Chromium, the scheme for Firefox.
    fn key(&self) -> &str {
        match self {
            Self::Chromium(origin) => origin,
            Self::Firefox => FIREFOX_CALLER,
        }
    }
}

fn is_chromium_caller(arg: &str) -> bool {
    let Some(id) = arg.strip_prefix("chrome-extension://") else {
        return false;
    };
    let id = id.strip_suffix('/').unwrap_or(id);
    id.len() == 32 && id.bytes().all(|byte| (b'a'..=b'p').contains(&byte))
}

pub fn caller_of(argv: &[String]) -> Option<Caller> {
    if let Some(origin) = argv.iter().find(|arg| is_chromium_caller(arg)) {
        let origin = if origin.ends_with('/') {
            origin.clone()
        } else {
            format!("{origin}/")
        };
        return Some(Caller::Chromium(origin));
    }
    argv.iter()
        .any(|arg| arg == FIREFOX_ADDON_ID)
        .then_some(Caller::Firefox)
}

pub fn encode(value: &Value) -> Vec<u8> {
    let body = value.to_string().into_bytes();
    let mut framed = u32::try_from(body.len())
        .unwrap_or(u32::MAX)
        .to_le_bytes()
        .to_vec();
    framed.extend(body);
    framed
}

/// `None` until a whole message is there; `Some(Value::Null)` for one that is not JSON.
pub fn decode(buffer: &[u8]) -> Option<Value> {
    let head: [u8; 4] = buffer.get(..4)?.try_into().ok()?;
    let length = u32::from_le_bytes(head) as usize;
    let body = buffer.get(4..4 + length)?;
    Some(serde_json::from_slice(body).unwrap_or(Value::Null))
}

fn read_message() -> Value {
    let mut held = Vec::new();
    let mut chunk = [0_u8; 8192];
    let mut stdin = std::io::stdin().lock();
    loop {
        if let Some(message) = decode(&held) {
            return message;
        }
        match stdin.read(&mut chunk) {
            Ok(0) | Err(_) => return Value::Null,
            Ok(read) => held.extend_from_slice(&chunk[..read]),
        }
    }
}

fn wait_for(value: Option<&Value>) -> u64 {
    value
        .and_then(Value::as_f64)
        .map_or(0, |ms| ms.clamp(0.0, f64::from(MAX_PAIR_WAIT_MS)) as u64)
}

fn held() -> bool {
    lockfile::wake_held()
}

fn mode() -> String {
    config::pairing_mode(&config::read_agent_config())
}

pub fn run(argv: &[String]) -> ExitCode {
    // Nothing may reach stderr; a panic is logged instead.
    std::panic::set_hook(Box::new(|info| {
        log::log(&format!("native host stderr: {info}"))
    }));
    let caller = caller_of(argv);
    let message = read_message();
    let op = message.get("op").and_then(Value::as_str).map(str::to_owned);
    let waited = if op.as_deref() == Some("pair") {
        wait_for(message.get("waitMs"))
    } else {
        0
    };
    let reply = block_on(async {
        let deadline = Duration::from_millis(HOST_DEADLINE_MS + waited);
        tokio::time::timeout(deadline, answer(&message, caller.as_ref()))
            .await
            .unwrap_or_else(
                |_| json!({"ok": false, "error": "Vexdom Bridge took too long to answer."}),
            )
    });
    log_reply(op.as_deref(), &reply, caller.as_ref());
    let mut stdout = std::io::stdout().lock();
    let _ = stdout.write_all(&encode(&reply));
    let _ = stdout.flush();
    ExitCode::SUCCESS
}

fn log_reply(op: Option<&str>, reply: &Value, caller: Option<&Caller>) {
    if reply.get("error").and_then(Value::as_str) == Some(STOPPED) && reply.get("refused").is_none()
    {
        return;
    }
    let who = caller.map_or("an unknown caller", Caller::key);
    match op {
        Some("ensure") => {
            if reply["ok"] == Value::Bool(true) {
                log::log("native host: daemon ready");
            } else {
                let failed = reply
                    .get("error")
                    .and_then(Value::as_str)
                    .map(|e| format!(" — {e}"))
                    .unwrap_or_default();
                log::log(&format!("native host: could not start the daemon{failed}"));
            }
        }
        Some("pair") => {
            let outcome = if reply.get("grant").is_some() {
                "granted".to_owned()
            } else if reply.get("wait").is_some() {
                "waiting for approval".to_owned()
            } else if let Some(refused) = reply.get("refused").and_then(Value::as_str) {
                format!("refused ({refused})")
            } else {
                format!(
                    "failed: {}",
                    reply.get("error").and_then(Value::as_str).unwrap_or("")
                )
            };
            log::log(&format!("native host: pair for {who} {outcome}"));
        }
        _ => {}
    }
}

pub async fn answer(message: &Value, caller: Option<&Caller>) -> Value {
    match message.get("op").and_then(Value::as_str) {
        Some("hello") => json!({
            "ok": true,
            "host": 2,
            "bridge": vexdom_daemon::VERSION,
            "ops": HOST_OPS,
            "mode": mode(),
            "held": held(),
            "app": null,
        }),
        Some("pair") => pair(message, caller).await,
        Some("ensure") if held() => json!({"ok": false, "error": STOPPED}),
        Some("ensure") => match ensure::ensure_daemon().await {
            Ok(lock) => json!({"ok": true, "port": lock.port}),
            Err(error) => json!({"ok": false, "error": error}),
        },
        _ => json!({"ok": false, "error": "Expected {\"op\":\"ensure\"}."}),
    }
}

/// The pair request's fields, or `None` when it is missing what it needs.
struct PairRequest {
    install_id: String,
    browser: String,
    extension_version: String,
    gesture: bool,
    wait_ms: u64,
}

fn parse_pair_request(message: &Value) -> Option<PairRequest> {
    let install_id = message
        .get("installId")?
        .as_str()
        .filter(|id| is_install_id(id))?;
    message
        .get("reason")?
        .as_str()
        .filter(|reason| is_pair_reason(reason))?;
    let extension_version = message.get("extensionVersion")?.as_str()?;
    message.get("protocolVersion")?.as_f64()?;
    Some(PairRequest {
        install_id: install_id.to_owned(),
        browser: message
            .get("browser")
            .and_then(Value::as_str)
            .map(|b| b.chars().take(64).collect())
            .unwrap_or_default(),
        extension_version: extension_version.chars().take(32).collect(),
        gesture: message.get("gesture") == Some(&Value::Bool(true)),
        wait_ms: wait_for(message.get("waitMs")),
    })
}

async fn pair(message: &Value, caller: Option<&Caller>) -> Value {
    let Some(caller) = caller else {
        return json!({"ok": false, "refused": "origin", "error": "Vexdom Bridge pairs only with its own extension."});
    };
    let Some(request) = parse_pair_request(message) else {
        return json!({"ok": false, "error": "That pair request is missing what it needs."});
    };
    let mode = mode();
    if mode == "code" {
        return json!({"ok": false, "refused": "off", "error": OFF, "mode": mode});
    }
    if held() {
        return json!({"ok": false, "refused": "stopped", "error": STOPPED, "mode": mode});
    }
    let lock = match ensure::ensure_daemon().await {
        Ok(lock) => lock,
        Err(error) => {
            return json!({"ok": false, "error": format!("Vexdom Bridge could not start: {error}")});
        }
    };
    let bridge = match Bridge::connect_to(lock.port, &lock.token, None).await {
        Ok(bridge) => bridge,
        Err(error) => {
            return json!({"ok": false, "error": format!("Could not reach Vexdom Bridge: {error}")});
        }
    };
    let Ok(Ok(status)) = tokio::time::timeout(STATUS_TIMEOUT, bridge.status()).await else {
        return json!({"ok": false, "error": "Vexdom Bridge did not answer."});
    };
    let current = status["pairing"]["mode"]
        .as_str()
        .unwrap_or(&mode)
        .to_owned();
    let mut grant = bridge.control.grant_request();
    {
        let mut params = grant.get();
        params.set_install_id(&request.install_id);
        params.set_caller(caller.key());
        params.set_browser(&request.browser);
        params.set_extension_version(&request.extension_version);
        params.set_gesture(request.gesture);
        params.set_wait_ms(u32::try_from(request.wait_ms).unwrap_or(MAX_PAIR_WAIT_MS));
    }
    let timeout = Duration::from_millis(request.wait_ms + GRANT_TIMEOUT_MS);
    let outcome = match tokio::time::timeout(timeout, grant.send().promise).await {
        Ok(Ok(reply)) => reply
            .get()
            .ok()
            .and_then(|reply| {
                reply
                    .get_outcome()
                    .ok()
                    .and_then(|text| text.to_string().ok())
            })
            .and_then(|text| serde_json::from_str::<Value>(&text).ok()),
        Ok(Err(error)) => return json!({"ok": false, "error": error.to_string()}),
        Err(_) => None,
    };
    let Some(outcome) = outcome.filter(Value::is_object) else {
        return json!({"ok": false, "error": "Vexdom Bridge did not answer the pairing request."});
    };
    if let Some(grant) = outcome.get("grant") {
        return json!({"ok": true, "grant": grant, "mode": current, "approver": outcome.get("approver")});
    }
    if let Some(wait) = outcome.get("wait") {
        return json!({"ok": false, "wait": wait, "mode": current});
    }
    json!({"ok": false, "refused": outcome.get("refused"), "error": outcome.get("error"), "mode": current})
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_round_trip() {
        let framed = encode(&json!({"op": "hello"}));
        assert_eq!(&framed[..4], &(framed.len() as u32 - 4).to_le_bytes());
        assert_eq!(decode(&framed), Some(json!({"op": "hello"})));
        assert_eq!(decode(&framed[..framed.len() - 1]), None);
        let mut bad = 3_u32.to_le_bytes().to_vec();
        bad.extend(b"{x}");
        assert_eq!(decode(&bad), Some(Value::Null));
    }

    #[test]
    fn callers_from_argv() {
        let origin = format!("chrome-extension://{}", "a".repeat(32));
        assert_eq!(
            caller_of(std::slice::from_ref(&origin)),
            Some(Caller::Chromium(format!("{origin}/")))
        );
        assert_eq!(
            caller_of(&["x".into(), FIREFOX_ADDON_ID.into()]),
            Some(Caller::Firefox)
        );
        assert_eq!(
            caller_of(&[format!("chrome-extension://{}", "z".repeat(32))]),
            None
        );
    }

    #[test]
    fn pair_requests_are_checked() {
        let good = json!({"installId": "abcdefghijklmnop", "reason": "install", "extensionVersion": "0.9.2", "protocolVersion": 27, "waitMs": 99_999});
        let parsed = parse_pair_request(&good).expect("a complete request parses");
        assert_eq!(parsed.wait_ms, u64::from(MAX_PAIR_WAIT_MS));
        assert!(parse_pair_request(&json!({"installId": "short", "reason": "install"})).is_none());
    }
}
