//! The listener (`daemon.ts:164-310`): `/health`, the Host check, the upgrade's role by Origin or
//! Bearer token, and the raw 403 refusal. axum's tasks must be `Send`, so an accepted WebSocket is
//! handed to the `LocalSet`, where the Cap'n Proto side runs.

use std::io;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use axum::Router;
use axum::extract::ws::WebSocket;
use axum::extract::{FromRequestParts, Request, State, WebSocketUpgrade};
use axum::http::{HeaderMap, HeaderName, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use futures::{SinkExt, StreamExt};
use subtle::ConstantTimeEq;
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use vexdom_core::log;
use vexdom_link::WsStream;

use crate::Daemon;
use crate::wire::WsMessage;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Role {
    Extension,
    Control,
}

#[derive(Clone)]
struct Shared {
    token: Arc<str>,
    pid: u32,
    connected: Arc<AtomicBool>,
    accepted: mpsc::UnboundedSender<(Role, WebSocket, String)>,
}

/// The first free port in order, with the TS error text when none is.
pub(crate) async fn bind_first(ports: &[u16]) -> io::Result<TcpListener> {
    for &port in ports {
        match TcpListener::bind(("127.0.0.1", port)).await {
            Ok(listener) => return Ok(listener),
            Err(error) if error.kind() == io::ErrorKind::AddrInUse => {}
            Err(error) => return Err(error),
        }
    }
    let list = ports
        .iter()
        .map(u16::to_string)
        .collect::<Vec<_>>()
        .join(", ");
    Err(io::Error::new(
        io::ErrorKind::AddrInUse,
        format!("No free port in {list} — another process is using them all"),
    ))
}

pub(crate) fn serve(daemon: Daemon, listener: TcpListener) {
    let (accepted, mut incoming) = mpsc::unbounded_channel();
    let shared = Shared {
        token: Arc::from(daemon.token()),
        pid: std::process::id(),
        connected: daemon.health_connected(),
        accepted,
    };
    let app = Router::new().fallback(entry).with_state(shared);
    tokio::task::spawn_local(async move {
        if let Err(error) = axum::serve(listener, app).await {
            log::log(&format!("listener stopped: {error}"));
        }
    });
    tokio::task::spawn_local(async move {
        while let Some((role, socket, origin)) = incoming.recv().await {
            let stream = WsStream::<_, WsMessage>::new(
                socket
                    .with(|WsMessage(message)| futures::future::ok::<_, axum::Error>(message))
                    .map(|frame| frame.map(WsMessage)),
            );
            match role {
                Role::Extension => crate::gate::accept(&daemon, stream, origin),
                Role::Control => crate::control::accept(&daemon, stream),
            }
        }
    });
}

async fn entry(State(shared): State<Shared>, req: Request) -> Response {
    if !is_upgrade(req.headers()) {
        if !loopback_host(req.headers()) {
            return StatusCode::FORBIDDEN.into_response();
        }
        let target = req.uri().path_and_query().map_or("", |pq| pq.as_str());
        if req.uri().scheme().is_none() && target.starts_with("/health") {
            let body = format!(
                "{{\"ok\":true,\"pid\":{},\"version\":\"{}\",\"connected\":{}}}",
                shared.pid,
                crate::VERSION,
                shared.connected.load(Ordering::SeqCst)
            );
            return (
                StatusCode::OK,
                [(
                    header::CONTENT_TYPE,
                    HeaderValue::from_static("application/json"),
                )],
                body,
            )
                .into_response();
        }
        return StatusCode::NOT_FOUND.into_response();
    }
    let Some(role) = authorize(shared.token.as_bytes(), req.headers()) else {
        return refuse();
    };
    let origin = req
        .headers()
        .get(header::ORIGIN)
        .and_then(|origin| origin.to_str().ok())
        .unwrap_or("")
        .to_owned();
    let (mut parts, _) = req.into_parts();
    let upgrade = match WebSocketUpgrade::from_request_parts(&mut parts, &()).await {
        Ok(upgrade) => upgrade,
        Err(rejection) => return rejection.into_response(),
    };
    let accepted = shared.accepted.clone();
    upgrade
        .max_message_size(128 * 1024 * 1024)
        .max_frame_size(128 * 1024 * 1024)
        .on_upgrade(move |socket| async move {
            let _ = accepted.send((role, socket, origin));
        })
}

/// An `Upgrade` header and the `upgrade` token in `Connection`.
fn is_upgrade(headers: &HeaderMap) -> bool {
    headers.contains_key(header::UPGRADE)
        && headers.get_all(header::CONNECTION).iter().any(|value| {
            value
                .as_bytes()
                .split(|&b| b == b',')
                .any(|token| token.trim_ascii().eq_ignore_ascii_case(b"upgrade"))
        })
}

/// `/^(127\.0\.0\.1|\[::1\]|localhost)(:\d+)?$/i` on the Host.
fn loopback_host(headers: &HeaderMap) -> bool {
    let Some(host) = headers.get(header::HOST) else {
        return false;
    };
    let host = host.as_bytes();
    [&b"127.0.0.1"[..], b"[::1]", b"localhost"]
        .iter()
        .find_map(|name| {
            (host.len() >= name.len() && host[..name.len()].eq_ignore_ascii_case(name))
                .then(|| &host[name.len()..])
        })
        .is_some_and(|rest| match rest {
            [] => true,
            [b':', digits @ ..] => !digits.is_empty() && digits.iter().all(u8::is_ascii_digit),
            _ => false,
        })
}

/// `header.replace(/^Bearer\s+/i, '')` over latin1 bytes.
fn strip_bearer(raw: &[u8]) -> &[u8] {
    if raw.len() > 6 && raw[..6].eq_ignore_ascii_case(b"bearer") {
        let rest = &raw[6..];
        let spaces = rest
            .iter()
            .take_while(|b| matches!(b, b'\t' | 0x0b | 0x0c | b' ' | 0xa0))
            .count();
        if spaces > 0 {
            return &rest[spaces..];
        }
    }
    raw
}

/// Host, then Origin (an empty one counts as none), then the token.
fn authorize(token: &[u8], headers: &HeaderMap) -> Option<Role> {
    if !loopback_host(headers) {
        log::log(&format!(
            "rejected host {}",
            headers
                .get(header::HOST)
                .and_then(|h| h.to_str().ok())
                .unwrap_or("")
        ));
        return None;
    }
    let origin = headers
        .get(header::ORIGIN)
        .map(HeaderValue::as_bytes)
        .filter(|origin| !origin.is_empty());
    match origin {
        Some(origin)
            if [
                &b"chrome-extension://"[..],
                b"moz-extension://",
                b"safari-web-extension://",
            ]
            .iter()
            .any(|prefix| origin.starts_with(prefix)) =>
        {
            Some(Role::Extension)
        }
        Some(origin) => {
            log::log(&format!(
                "rejected web origin {}",
                String::from_utf8_lossy(origin)
            ));
            None
        }
        None => {
            let offered = strip_bearer(
                headers
                    .get(header::AUTHORIZATION)
                    .map_or(&b""[..], HeaderValue::as_bytes),
            );
            let ok = bool::from(offered.ct_eq(token));
            if !ok {
                log::log("rejected control client: missing or invalid token");
            }
            ok.then_some(Role::Control)
        }
    }
}

fn refuse() -> Response {
    (
        StatusCode::FORBIDDEN,
        [
            (header::CONNECTION, HeaderValue::from_static("close")),
            (
                HeaderName::from_static("x-vexdom-reason"),
                HeaderValue::from_static("unauthorized"),
            ),
        ],
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers(pairs: &[(&str, &[u8])]) -> HeaderMap {
        let mut map = HeaderMap::new();
        for (name, value) in pairs {
            map.append(
                HeaderName::from_bytes(name.as_bytes()).unwrap(),
                HeaderValue::from_bytes(value).unwrap(),
            );
        }
        map
    }

    #[test]
    fn host_must_be_loopback() {
        for ok in [
            &b"127.0.0.1"[..],
            b"127.0.0.1:8785",
            b"LOCALHOST:1",
            b"[::1]:9",
            b"localhost",
        ] {
            assert!(loopback_host(&headers(&[("host", ok)])), "{ok:?}");
        }
        for bad in [
            &b"evil.example"[..],
            b"127.0.0.1:",
            b"localhost:80x",
            b"127.0.0.1.evil",
            b"",
        ] {
            assert!(!loopback_host(&headers(&[("host", bad)])), "{bad:?}");
        }
    }

    #[test]
    fn roles() {
        let host: (&str, &[u8]) = ("host", b"127.0.0.1:1");
        let role = |pairs: &[(&str, &[u8])]| authorize(b"tok", &headers(pairs));
        assert_eq!(
            role(&[host, ("origin", b"chrome-extension://x")]),
            Some(Role::Extension)
        );
        assert_eq!(
            role(&[host, ("origin", b"https://a"), ("authorization", b"tok")]),
            None
        );
        assert_eq!(
            role(&[host, ("origin", b""), ("authorization", b"Bearer tok")]),
            Some(Role::Control)
        );
        assert_eq!(role(&[host, ("authorization", b"Bearer tox")]), None);
        assert_eq!(strip_bearer(b"bearer\t \xa0abc"), b"abc");
        assert!(is_upgrade(&headers(&[
            ("upgrade", b"websocket"),
            ("connection", b"keep-alive, Upgrade")
        ])));
    }
}
