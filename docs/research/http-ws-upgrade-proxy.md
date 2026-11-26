# Research: one loopback listener for HTTP and WebSocket upgrades, and the front-door proxy

Topic `http-ws-upgrade-proxy` (MAP.md). It covers two questions:

- How a tokio listener on 127.0.0.1 serves `GET /health` and WebSocket upgrades on any path, deciding the
  role at upgrade time the way `src/daemon/daemon.ts:251-310` does.
- How the P3 front door forwards HTTP and WebSocket traffic to the internal TS daemon while intercepting
  selected control frames.

Researched 2026-11-25.

Sources:

- crate sources as published: `https://docs.rs/crate/<crate>/<version>/source/<path>`
- ws 8.21.1 as installed: `src/daemon/node_modules/ws/lib/…`
- RFC 6455: `https://www.rfc-editor.org/rfc/rfc6455.html`
- version metadata: `https://crates.io/api/v1/crates/<crate>`

Claims marked **observed** come from probes run on 2026-11-25 (node 26.13.1, cargo 1.99.1):

- A node mirror of the daemon's listener code: the same `createServer`, `authorize`, `refuseUpgrade` and
  `ws` 8.21.1.
- A Rust front-door probe: axum 0.8.9, tokio-tungstenite 0.29.0, hyper 1.13.0, hyper-util 0.1.21 and
  subtle 2.6.1. Its upstream was the node mirror.

The probes lived in the session scratchpad and are not part of the repo. Their relevant code is in §4.

## 1. TS baseline

**Listener and plain HTTP** (`src/daemon/daemon.ts:251-262`):

- A Host that is not loopback gets `403` with an empty body.
- `req.url?.startsWith('/health')` gets `200 application/json`
  `{"ok":true,"pid":…,"version":…,"connected":…}`. The check runs on the raw request-target, so
  `/healthz?x=1` and `POST /health` also get 200, and the absolute-form `GET http://127.0.0.1/health` gets
  404 (**observed**).
- Everything else gets `404`.
- **observed**: Node frames these as `Transfer-Encoding: chunked` with `Date` and `Connection` headers.

**Upgrade** (`src/daemon/daemon.ts:264-272`):

- `http.on('upgrade')` calls `authorize(req)`. When that returns null, `refuseUpgrade` writes the exact
  bytes `HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\nConnection: close\r\nX-Vexdom-Reason:
  unauthorized\r\n\r\n` and destroys the socket (`src/daemon/daemon.ts:1151-1153`).
- Only after that does `wss.handleUpgrade` validate the method (405), `Upgrade` (400), `Sec-WebSocket-Key`
  (400) and `Sec-WebSocket-Version` (∈ {13, 8}, else 400). It refuses with
  `Connection: close, Content-Type: text/html` and a text message (`ws/lib/websocket-server.js:244-280,
  505-532`).
- So **authorization is checked before protocol validity** (**observed**): a bad Origin with a valid key
  gets 403, and a valid token with no key gets `400 Missing or invalid Sec-WebSocket-Key header`.

**Role decision** (`src/daemon/daemon.ts:287-310`). The path is ignored; clients use `/control`
(`src/daemon/remote-bridge.ts:31`) and `/extension` (`packages/extension/src/lib/bridge/socket.ts:543`).

1. `LOOPBACK_HOST = /^(127\.0\.0\.1|\[::1\]|localhost)(:\d+)?$/i` is tested on `Host`, and a failure means
   reject (`src/daemon/daemon.ts:109,1134-1136`). This is the DNS-rebinding guard.
2. `origin && EXTENSION_ORIGIN.test(origin)`, with `EXTENSION_ORIGIN = /^(chrome|moz|safari-web)-extension:\/\//`,
   gives the **extension** role (`src/daemon/daemon.ts:108,293`).
3. Any other truthy Origin is rejected (`src/daemon/daemon.ts:294-297`).
4. With no Origin, `Buffer.from(authorization.replace(/^Bearer\s+/i, ''))` is compared with the token by
   length check plus `timingSafeEqual`, giving the **control** role (`src/daemon/daemon.ts:305-310`). The
   token is `randomBytes(24).toString('base64url')` (`src/daemon/daemon.ts:228`).

RFC 6455 §4.2.2 / §10.2: "If the origin indicated is unacceptable to the server, then it SHOULD respond
to the WebSocket handshake with a reply containing HTTP 403 Forbidden".

**Ports** (`src/daemon/daemon.ts:1103-1128`):

- `daemonPorts` (`VEXDOM_PORTS` or `[8785, 8786, 8787]`, `src/daemon/ports.ts:8`,
  `packages/extension/src/lib/actions/protocol.ts:32`) are tried in order.
- `EADDRINUSE` moves to the next port, any other error rejects, and an exhausted list gives
  `No free port in 8785, 8786, 8787 — another process is using them all`.
- `0` is a valid entry, and the OS picks the port (`src/daemon/ports.ts:3-6,10-17`).

**WebSocket server settings**: `new WebSocketServer({ noServer: true })` (`src/daemon/daemon.ts:264`)
takes ws defaults (`ws/lib/websocket-server.js:35-79`):

- `maxPayload` 100 MiB per message
- `autoPong: true`
- `perMessageDeflate: false`
- `closeTimeout` 30 s
- `skipUTF8Validation: false`

**Heartbeat**: the daemon sends application-level JSON text frames `{t:'ping',id}` every 20 s and answers
`{t:'pong'}` (`src/daemon/extension-link.ts:21,75,141-142`). It sends no RFC 6455 ping frames. Every
received frame is read as `String(raw)`, binary frames included (`src/daemon/extension-link.ts:69`,
`src/daemon/daemon.ts:1145`).

**Close codes used**:

| Code | Reason | Where |
| --- | --- | --- |
| 1000 | link close | `extension-link.ts:116-120`, `remote-bridge.ts:148-149` |
| 1001 | daemon shutting down | `daemon.ts:1084` |
| 1002 | handshake protocol errors | `daemon.ts:332,338,343,348,371` |
| 1011 | handshake failed | `daemon.ts:315` |
| 4401 | unauthorized, after a `{t:'unauthorized',reason,retryable,code}` frame | `daemon.ts:318-327` |

ws accepts close codes 1000–1014 except 1004–1006, plus 3000–4999 (`ws/lib/validation.js:37-46`), and
**throws** when a close reason exceeds 123 bytes (`ws/lib/sender.js:189-198`). The RFC side:

- RFC 6455 §5.5: "All control frames MUST have a payload length of 125 bytes or less" (2 bytes of code
  plus 123 bytes of reason).
- §7.4.1 defines 1002 as "terminating the connection due to a protocol error" and 1011 as "encountered an
  unexpected condition". 1006 "MUST NOT be set as a status code in a Close control frame".
- §7.4.2: "4000-4999 are reserved for private use".

## 2. axum vs hyper + tokio-tungstenite directly

| Crate | Latest | Published | MSRV | License |
| --- | --- | --- | --- | --- |
| axum | 0.8.9 | 2026-04-14 | 1.80 | MIT |
| tokio-tungstenite / tungstenite | 0.30.0 | 2026-07-11 | 1.85 | MIT / MIT OR Apache-2.0 |
| hyper | 1.13.0 | 2026-11-23 (2 days; 1.12.0 before it) | 1.63 | MIT |
| hyper-util | 0.1.21 | 2026-09-24 | 1.85 | MIT |
| subtle | 2.6.1 | 2024-06-24 | – | BSD-3-Clause |

axum 0.8.9's `ws` feature depends on **tokio-tungstenite 0.29.0**, not 0.30
(`https://docs.rs/crate/axum/0.8.9/source/Cargo.toml` L133-139, L264-266).

### axum (`ws` feature)

- **Extractor checks**: `WebSocketUpgrade: FromRequestParts` checks, in order:
  - GET
  - `Connection` contains `upgrade`
  - `Upgrade: websocket`
  - `Sec-WebSocket-Key` present
  - `Sec-WebSocket-Version: 13`
  - then it takes hyper's `OnUpgrade` (`axum 0.8.9 src/extract/ws.rs` L443-512).

  Its rejections are `405`/`400`/`426` with `text/plain` bodies such as ``"`Sec-WebSocket-Key` header
  missing"`` (L948-1003). They do not match ws's texts (**observed**).
- **Reading Origin/Host/Authorization before accepting.** The handler receives the `Request` or
  `HeaderMap` first. Nothing is upgraded until the handler returns the 101 that `on_upgrade` builds;
  hyper performs the switch when that response is written, and the callback runs in a spawned task
  (L347-411). Returning any other `Response` refuses the upgrade.
- **Keeping TS's 403-before-400 order.** Run `authorize` first, then build the extractor from parts
  (`WebSocketUpgrade::from_request_parts(&mut parts, &())`). Alternatively take
  `Result<WebSocketUpgrade, WebSocketUpgradeRejection>`.
- **Any path**: `Router::new().fallback(entry)`.
- **Limits** are set with `.max_message_size()`, `.max_frame_size()` and the buffer sizes (L159-210).
  They default to tungstenite's 64 MiB per message and **16 MiB per frame**
  (`tungstenite 0.29.0/0.30.0 src/protocol/mod.rs` L76-105).
- **Ping/pong**: "Ping messages will be automatically responded to" (`ws.rs` L781). tungstenite queues the
  pong while reading and flushes it on the next read or write (`tungstenite 0.30.0 src/protocol/mod.rs`
  L281-292, L449-470). RFC 6455 §5.5.2 says an endpoint "SHOULD respond with Pong frame as soon as is
  practical".
- **Close**: `Message::Close(Some(CloseFrame { code, reason }))`.
  - tungstenite does **not** bound the reason length (`src/protocol/frame/frame.rs` L329-340).
  - It replies to a received close automatically. A disallowed received code is replaced with 1002
    "Protocol violation" (`src/protocol/mod.rs` L718-740; `src/protocol/frame/coding.rs` L192-194).
- **Server options**: `axum::serve` builds the hyper connection itself, so hyper's HTTP/1 options are out
  of reach. For example `half_close` defaults to `false` (`hyper 1.13.0 src/server/conn/http1.rs` L247,
  L260-271).

### hyper + tokio-tungstenite, without axum

- **Shape**: `hyper::server::conn::http1::Builder::new().serve_connection(io, service_fn(..)).with_upgrades()`
  serves the connection. The service checks the headers and returns either a refusal or a 101 that
  carries `tungstenite::handshake::derive_accept_key(key)` (`tungstenite 0.30.0 src/handshake/mod.rs`
  L117). After that it awaits `hyper::upgrade::on(req)` and wraps the result with
  `WebSocketStream::from_raw_socket(TokioIo::new(upgraded), Role::Server, Some(cfg))`
  (`tokio-tungstenite 0.30.0 src/lib.rs` L218).
- **Cost**: this is what axum does internally (`axum ws.rs` L347-411). It means writing about 40 lines of
  checks yourself (Connection token list, key, version 13, 101 headers).
- **Gain**: access to hyper's builder (`half_close`, `title_case_headers`), and freedom to use
  tokio-tungstenite 0.30.
- `tokio_tungstenite::accept_hdr_async` with a `Callback` can reject a handshake with a custom
  `ErrorResponse` (`tungstenite 0.30.0 src/handshake/server.rs` L173-182). It only works on a stream
  already known to be an upgrade, so it cannot also serve `/health` on the same port. It is not enough on
  its own.

| | axum 0.8.9 | hyper 1 + tokio-tungstenite |
| --- | --- | --- |
| Headers before accept | yes (handler) | yes (service) |
| Custom 403 + `X-Vexdom-Reason` | return a `Response` | return a `Response` |
| Upgrade validation | built in, axum texts | hand-written |
| HTTP/1 knobs (`half_close`) | not via `axum::serve` (hyper-util builder needed) | yes |
| tungstenite version | 0.29 (pinned by axum) | free (0.30) |
| `/health` routing | `Router` | match on the request |

### Trying 8785, 8786, 8787 (observed working, §4.4)

- `tokio::net::TcpListener::bind` calls mio's `bind`, which sets `SO_REUSEADDR` on Unix and deliberately
  not on Windows (`tokio 1.54.1 src/net/tcp/listener.rs` L122-125; `mio 1.2.5 src/net/tcp/listener.rs`
  L63-84).
- `io::ErrorKind::AddrInUse` is the Rust counterpart of `EADDRINUSE`.
- Not verified: libuv's socket options on Windows. Record a busy-port case per OS in P2.

## 3. The front door

The front door owns the public lockfile with its own pid, port and token. The internal TS daemon runs
with `VEXDOM_PORTS=0` and `VEXDOM_LOCKFILE=<private path>` (`src/daemon/ports.ts:8`,
`src/daemon/lockfile.ts:12`). The front door reads the inner port and token from that private file.

Handling per request type:

| Request | Handling | Why |
| --- | --- | --- |
| Non-loopback Host | 403 here (and again in TS, since Host is forwarded unchanged) | rebinding guard first |
| `GET /health*` | **answer here**, with the front door's own `pid`; `connected` comes from the inner daemon | `probeExisting` requires `health.pid === lock.pid` (`src/daemon/ensure-daemon.ts:115-117`); `runningDaemons`/`stop` signal the pid `/health` reports (`ensure-daemon.ts:145-152`) |
| Other plain HTTP | forward (hyper client, one connection) | 404s and 403s stay TS-authored |
| Upgrade with extension Origin | **byte splice**: forward the client's own upgrade request (Origin, Host, `Sec-WebSocket-Key`), return the inner daemon's own 101 or 403, then `copy_bidirectional` | frames, masks, fragmentation, close codes and the HMAC handshake pass untouched; the inner daemon computes `Sec-WebSocket-Accept` from the client's key |
| Upgrade, no Origin, valid **public** token | **frame relay**: dial the inner daemon first with `Authorization: Bearer <inner token>` and the original Host; relay its refusal as HTTP; otherwise accept the downstream and pump messages | the token swap needs a new handshake; interception needs whole messages |
| Anything else | 403 + `X-Vexdom-Reason: unauthorized` | as TS |

The tungstenite client lets the caller keep the Host header. `IntoClientRequest for Uri` puts in a Host
derived from the URI (`tungstenite 0.29.0 src/client.rs` L223-246). `generate_request` writes the
request's own `Host`, `Connection`, `Upgrade`, `Sec-WebSocket-Version` and `Sec-WebSocket-Key` values and
then the remaining headers (`src/handshake/client.rs` L112-172). Inserting `HOST` and `AUTHORIZATION`
into the request therefore works (**observed**: the inner daemon saw `host: 127.0.0.1:<front port>`,
`authorization: Bearer inner`).

Pump rules for the control relay:

- Text and binary messages are forwarded as is. An intercepted text frame is answered locally and never
  forwarded. A frame that is not intercepted is forwarded with its **original text**, not re-serialized.
- One writer task per downstream socket, so local replies never interleave with relayed messages.
- Ping and pong frames are **not** forwarded; each hop answers its own. Forwarding them would produce a
  pong from the proxy and a second one from the far end. TS sends no protocol pings anyway (§1).
- A Close is forwarded with the same code and reason. The reader then keeps reading until its stream ends,
  so tungstenite can flush its automatic close reply.
  - **observed**: a client `close(1000,'client exiting')` reached the inner daemon as `1000 client
    exiting`, and the client got its reply in 4 ms.
  - **observed**: the inner daemon's `4401 unauthorized` reached the client unchanged.
- An abnormal end on one side (EOF or error with no Close) drops both sockets, so the peer sees 1006, as it
  would against TS.
- Limits are 100 MiB per message and per frame in both directions, to match ws `maxPayload`. With
  tungstenite's defaults, a 20 MiB frame from the inner daemon killed the relay: the client saw `1006` and
  the inner daemon saw `1006` (**observed**). With 100 MiB it was relayed whole (**observed**).

## 4. Code sketches (abridged from the probe that compiled and ran; `Daemon`, `health()`, `to_ts`/`to_ax`, `answer_in_rust` are placeholders)

### 4.1 Upgrade authorization (axum)

```rust
// axum = { version = "=0.8.9", default-features = false, features = ["http1", "tokio", "ws"] }
// subtle = "2.6"
use axum::{extract::{FromRequestParts, Request, State, ws::WebSocketUpgrade},
           http::{HeaderMap, HeaderName, HeaderValue, StatusCode, header},
           response::{IntoResponse, Response}};
use subtle::ConstantTimeEq;

enum Role { Extension, Control }

/// /^(127\.0\.0\.1|\[::1\]|localhost)(:\d+)?$/i on Host (daemon.ts:109); a missing Host fails.
fn loopback_host(h: &HeaderMap) -> bool {
    let Some(host) = h.get(header::HOST) else { return false };
    let host = host.as_bytes().to_ascii_lowercase();
    match [&b"127.0.0.1"[..], b"[::1]", b"localhost"].iter().find_map(|p| host.strip_prefix(*p)) {
        Some([]) => true,
        Some([b':', d @ ..]) => !d.is_empty() && d.iter().all(u8::is_ascii_digit),
        _ => false,
    }
}

/// header.replace(/^Bearer\s+/i, ''): no scheme leaves the raw value as the offer (daemon.ts:307).
fn strip_bearer(raw: &[u8]) -> &[u8] {
    if raw.len() > 6 && raw[..6].eq_ignore_ascii_case(b"bearer") {
        let rest = &raw[6..];
        // JS \s over a latin1-decoded header: TAB VT FF SP NBSP (CR/LF cannot occur)
        let n = rest.iter().take_while(|b| matches!(b, b'\t' | 0x0b | 0x0c | b' ' | 0xa0)).count();
        if n > 0 { return &rest[n..]; }
    }
    raw
}

fn authorize(token: &[u8], h: &HeaderMap) -> Option<Role> {
    if !loopback_host(h) { return None; }
    // `origin && …`: "" counts as absent. Bytes, not to_str(): Node decodes header bytes as latin1.
    match h.get(header::ORIGIN).map(HeaderValue::as_bytes).filter(|o| !o.is_empty()) {
        Some(o) if [&b"chrome-extension://"[..], b"moz-extension://", b"safari-web-extension://"]
            .iter().any(|p| o.starts_with(p)) => Some(Role::Extension),
        Some(_) => None,
        None => {
            let offered = strip_bearer(h.get(header::AUTHORIZATION).map_or(&b""[..], HeaderValue::as_bytes));
            // subtle short-circuits on unequal length, like the explicit length check before timingSafeEqual
            bool::from(offered.ct_eq(token)).then_some(Role::Control)
        }
    }
}

fn refuse(reason: &'static str) -> Response {   // daemon.ts:1151-1153 (header case/order differ, see traps)
    (StatusCode::FORBIDDEN,
     [(header::CONNECTION, "close"), (HeaderName::from_static("x-vexdom-reason"), reason)]).into_response()
}

async fn entry(State(d): State<Daemon>, req: Request) -> Response {
    if !is_upgrade(req.headers()) {              // Upgrade header + `upgrade` token in Connection
        if !loopback_host(req.headers()) { return StatusCode::FORBIDDEN.into_response(); }
        // raw request-target, as req.url: absolute-form "http://…/health" must stay 404
        if req.uri().scheme().is_none() && req.uri().path().starts_with("/health") { return d.health(); }
        return StatusCode::NOT_FOUND.into_response();
    }
    let Some(role) = authorize(&d.token, req.headers()) else { return refuse("unauthorized") };
    let (mut parts, _) = req.into_parts();      // authorize first: 403 wins over a malformed handshake
    let ws = match WebSocketUpgrade::from_request_parts(&mut parts, &()).await {
        Ok(ws) => ws.max_message_size(100 << 20).max_frame_size(100 << 20),   // ws maxPayload
        Err(rejection) => return rejection.into_response(),
    };
    let origin = parts.headers.get(header::ORIGIN).cloned();
    ws.on_upgrade(move |socket| async move {
        match role { Role::Extension => d.accept_extension(socket, origin).await,
                     Role::Control => d.accept_control(socket).await }
    })
}
```

To close with TS's codes, send
`socket.send(Message::Close(Some(CloseFrame { code: 4401, reason: "unauthorized".into() })))`. Check that
the reason is at most 123 bytes first, because tungstenite will not.

### 4.2 Bidirectional frame relay with interception (control role)

```rust
// tokio-tungstenite = "=0.29.0" (axum's), futures-util, tokio::sync::mpsc
async fn pump(down: axum::extract::ws::WebSocket, up: WebSocketStream<TcpStream>) {
    let (mut down_tx, mut down_rx) = down.split();
    let (mut up_tx, mut up_rx) = up.split();
    let (to_down, mut q) = mpsc::channel::<ws::Message>(64);        // single downstream writer
    let writer = tokio::spawn(async move { while let Some(m) = q.recv().await { let _ = down_tx.send(m).await; } });
    {
        let up_to_down = { let to_down = to_down.clone(); async move {
            while let Some(next) = up_rx.next().await {
                let Ok(m) = next else { return false };                // abnormal: no grace
                let closing = matches!(m, ts::Message::Close(_));
                if let Some(m) = to_ax(m) { let _ = to_down.send(m).await; }   // Ping/Pong → None
                if closing { while up_rx.next().await.is_some() {} return true; } // let the close reply flush
            }
            false
        }};
        let down_to_up = { let to_down = to_down.clone(); async move {
            while let Some(next) = down_rx.next().await {
                let Ok(m) = next else { return false };
                if let ws::Message::Text(t) = &m && let Some(reply) = intercept(t.as_str()) {
                    let _ = to_down.send(ws::Message::text(reply)).await;      // answered by Rust
                    continue;
                }
                let closing = matches!(m, ws::Message::Close(_));
                if let Some(m) = to_ts(m) { let _ = up_tx.send(m).await; }      // original text, unchanged
                if closing { while down_rx.next().await.is_some() {} return true; }
            }
            false
        }};
        tokio::pin!(up_to_down, down_to_up);
        tokio::select! {      // the first side to end decides; a clean close gives the other 5 s
            clean = &mut up_to_down => if clean { let _ = timeout(Duration::from_secs(5), &mut down_to_up).await; },
            clean = &mut down_to_up => if clean { let _ = timeout(Duration::from_secs(5), &mut up_to_down).await; },
        }
    } // both readers and their senders drop here; dropping the halves closes the sockets
    drop(to_down);
    let _ = writer.await;
}

fn intercept(text: &str) -> Option<String> {     // e.g. a Rust-only op such as `snapshot` (ADR-011)
    let v: serde_json::Value = serde_json::from_str(text).ok()?;
    matches!(v.get("op")?.as_str()?, "snapshot").then(|| answer_in_rust(&v))
}
// to_ts/to_ax map Text↔Text, Binary↔Binary, Close(code, reason)↔Close(code, reason), Ping/Pong → None.
```

Dial the inner daemon **before** accepting the downstream, so that a refusal there is relayed as HTTP and
not as a 101 followed by a close:

```rust
let mut up_req = format!("ws://{inner}{path_and_query}").into_client_request()?;
up_req.headers_mut().insert(header::HOST, parts.headers[header::HOST].clone());
up_req.headers_mut().insert(header::AUTHORIZATION, format!("Bearer {inner_token}").parse()?);
let up = match tokio_tungstenite::client_async_with_config(up_req, TcpStream::connect(inner).await?, Some(cfg_100mib)).await {
    Ok((up, _)) => up,
    Err(tungstenite::Error::Http(resp)) => { let (p, b) = resp.into_parts(); return Response::from_parts(p, Body::from(b.unwrap_or_default())); }
    Err(_) => return StatusCode::BAD_GATEWAY.into_response(),
};
upgrade.on_upgrade(move |down| pump(down, up))
```

### 4.3 Byte splice (extension role)

```rust
let downstream = hyper::upgrade::on(&mut req);                      // before req is moved
let (mut tx, conn) = hyper::client::conn::http1::handshake::<_, Body>(TokioIo::new(TcpStream::connect(inner).await?)).await?;
tokio::spawn(conn.with_upgrades());
let mut resp = tx.send_request(req).await?;                         // same method, target, headers, key
if resp.status() != StatusCode::SWITCHING_PROTOCOLS { return Ok(resp.map(Body::new)); }  // TS's own 403
let upstream = hyper::upgrade::on(&mut resp);
tokio::spawn(async move {
    if let (Ok(d), Ok(u)) = tokio::join!(downstream, upstream) {
        let _ = tokio::io::copy_bidirectional(&mut TokioIo::new(d), &mut TokioIo::new(u)).await;
    }
});
Ok(resp.map(|_| Body::empty()))                                     // TS's own 101 + Sec-WebSocket-Accept
```

### 4.4 Port walk

```rust
async fn bind_first(ports: &[u16]) -> io::Result<TcpListener> {      // daemon.ts:1103-1128
    for &port in ports {
        match TcpListener::bind(("127.0.0.1", port)).await {
            Ok(l) => return Ok(l),                                    // port 0 → l.local_addr() has the OS pick
            Err(e) if e.kind() == io::ErrorKind::AddrInUse => continue,
            Err(e) => return Err(e),
        }
    }
    let list = ports.iter().map(u16::to_string).collect::<Vec<_>>().join(", ");
    Err(io::Error::new(io::ErrorKind::AddrInUse, format!("No free port in {list} — another process is using them all")))
}
```

### Probe results (observed)

- Port walk: with the first port held, the probe bound the second. With all three held, it returned
  `No free port in 18790, 18791, 18792 — another process is using them all`. With port `0`, the OS
  picked one.
- Control relay:
  - the inner daemon saw `Bearer inner` and the original Host
  - `{"op":"frontdoor"}` was answered by Rust, and other frames were echoed by the inner daemon
  - a 20 MiB frame was relayed
  - the inner daemon's 4401 reached the client
- Extension splice: the inner daemon saw the original `Origin`, `Host` and `Sec-WebSocket-Key`. Its echo
  and its 4401 reached the client.
- Refusals: a web Origin or a wrong token got
  `403 {connection: close, x-vexdom-reason: unauthorized, content-length: 0, date}`. `Authorization:
  outer` with no scheme got 101, as in TS.
- Edge cases: Rust and the TS mirror gave the same status for duplicate `Host` (the first value wins),
  duplicate `Authorization` (first wins), NBSP after `Bearer` (accepted), `POST /health` (200) and
  absolute-form `/health` (404).

## Semantic traps

1. **An empty `Origin` counts as no Origin.** `if (origin && …)` is falsy for `""`, so the token path is
   taken (**observed**: `Origin:` plus a token gave 101). `HeaderMap::get` returns `Some("")`, so filter it
   out.
2. **The Origin test is case-sensitive and checks only a prefix** (no `/i`, no `$`). Node hands header
   bytes over latin1-decoded, so use `as_bytes().starts_with(…)`. With `to_str()`, any obs-text byte would
   refuse an Origin that TS accepts.
3. **The Host test is case-insensitive, with an optional `:digits`.** A missing Host (HTTP/1.0) is 403.
   The first of duplicate Host headers decides in both stacks (**observed**).
4. **`/^Bearer\s+/i` has surprises.** A bare token without a scheme is accepted. `Bearer` followed by no
   whitespace is not stripped. JS `\s` includes NBSP `0xA0` (**observed**). Only one prefix is removed.
   The length check comes before the constant-time compare: subtle's slice `ct_eq` short-circuits on
   unequal length (`subtle 2.6.1 src/lib.rs` L289-297), the same as TS.
5. **Authorization runs before handshake validation.** 403 beats 400 and 405. axum's extractor rejects
   first unless it is built after `authorize`.
6. **ws also accepts `Sec-WebSocket-Version: 8`**, while axum and tungstenite accept only 13. Version 8
   gets 101 from TS and 400 from Rust. Every real client sends 13.
7. **Refusal bytes differ** from Node. Rust sends lowercase header names, adds `date:`, uses
   `content-length` instead of chunked, and writes axum `text/plain` rejection texts where ws writes
   `text/html` ones. Clients only read the status (node ws: "Unexpected server response: 403"), so
   normalize headers in P2 replays. Nothing in the repo reads `X-Vexdom-Reason`; `grep` finds only
   `daemon.ts:1152`.
8. **Size caps.** ws allows 100 MiB per message. tungstenite defaults to 64 MiB per message and 16 MiB
   per frame, and ws sends a message as one frame. With the defaults a large screenshot `dataUrl` breaks
   the link with 1006 (**observed** with 20 MiB).
9. **Close reasons over 123 bytes.** ws throws a `RangeError`. Inside `greet` that throw turns into
   `ws.close(1011, 'handshake failed')` (`daemon.ts:313-316`). tungstenite sends the oversized control
   frame anyway, which violates RFC 6455 §5.5. Check the length, and mirror the 1011 path if the port
   keeps the refusal strings.
10. **Text versus binary.** TS treats both as text through `String(raw)`, a UTF-8 decode that replaces
    bad sequences. Rust must decode `Message::Binary` with `String::from_utf8_lossy` to match.
11. **The proxy must not forward Ping/Pong** (double pongs). Each hop's tungstenite answers its own. TS
    heartbeats are text frames and pass through.
12. **hyper ignores half-closed clients.** Its HTTP/1 `half_close` defaults to `false`, so a client that
    shuts down its write side after sending the request gets no response. `nc` showed this: the Rust
    server sent nothing while Node answered (**observed**). `axum::serve` cannot change it; use
    hyper-util's builder if P2 recordings contain such clients.
13. **`/health` must report the front door's pid** (`ensure-daemon.ts:117`). Forwarding it unchanged
    breaks `probeExisting` and makes `vexdom stop` signal the inner daemon instead of the front door.
14. **Match the raw request-target for `/health`.** Use the request-target, not `uri.path()`:
    absolute-form `GET http://127.0.0.1/health` is 404 in TS (**observed**).
15. **A frame relay re-frames.** Fragmentation, masking and ping timing change. Only the control role
    needs interception. The extension role is byte-spliced, so the handshake and HMAC transcript frames
    are byte-identical.
16. **Dial the inner daemon before sending the downstream its 101.** Otherwise an inner refusal reaches
    the client as an accepted socket that then closes, and its HTTP status is lost.
17. **Two tungstenite versions.** axum 0.8.9 pins tokio-tungstenite 0.29. A proxy that adds 0.30 compiles
    two tungstenite copies with distinct `Message` types. Pin 0.29, or drop axum.
18. **Upgrade detection differs.** Node raises `'upgrade'` for any request that asks for an upgrade, and
    then ws answers a non-websocket `Upgrade` with `400 Invalid Upgrade header`. Under axum the same
    request is a normal request, so `is_upgrade()` must classify it the way Node does. Only
    `Upgrade: websocket` was observed; record an `h2c` case in P2.

## Recommendation

**Use axum 0.8.9 (`ws`) on one `TcpListener`, with a fallback handler that classifies each request.
Write the authorization checks by hand: Host, then Origin, then the bearer token compared with `subtle`,
then the extractor. Take tokio-tungstenite 0.29 (axum's) for the front door's upstream client.**

- Raise both WebSocket limits to 100 MiB, add a ≤123-byte guard to the close helper, and use `bind_first`
  for 8785–8787 and port 0.
- axum already does the upgrade validation and the 101, and P3 needs routing for `/health` anyway. Its
  only real limit here, `half_close`, does not matter for the current clients (node ws, browser `fetch`,
  `WebSocket`).
- If P2 recordings show half-closing clients, replace `axum::serve` with hyper-util's
  `server::conn::auto::Builder` (`.http1().half_close(true)`, `serve_connection_with_upgrades`). Handlers
  stay as they are.

**Front door (P3):**

- Answer `/health` locally with the front door's pid and the inner daemon's `connected`, and forward other
  plain HTTP.
- Byte-splice extension upgrades.
- Relay control upgrades frame by frame: dial the inner daemon first, swap the token, keep Host,
  intercept the ops that are ported (ADR-011), and forward everything else as its original text.
- Run the inner TS daemon with `VEXDOM_PORTS=0` and `VEXDOM_LOCKFILE=<private>`.
- The splice also covers ADR-018 unchanged. It does not look inside frames, so capnp binary frames after
  `welcome` pass through as they are. The control relay forwards binary frames as they are too.
- Supervision is open for P3: the inner daemon's idle exit and how SIGTERM is passed on.
