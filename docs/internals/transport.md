# Transport and authorization

How the extension and local clients (the CLI, MCP servers, `Vexdom.app`) connect to the Bridge
over loopback, and how each side proves it is allowed to.

---

## One listener, three ports

The Bridge runs one axum server (`crates/vexdom-daemon/src/http.rs`) that answers `GET /health`
and upgrades everything else to a WebSocket. It binds the first free port of **8785, 8786, 8787**
(`DAEMON_PORTS` in `crates/vexdom-core/src/proto.rs` and `packages/extension/src/lib/actions/protocol.ts`).
If all three are taken it exits with `No free port in 8785, 8786, 8787 — another process is using
them all`.

`/health` answers `{"ok":true,"pid":…,"version":"…","connected":…}`. Clients use the pid to make
sure the Bridge on a port is the one the lockfile names (`probe_existing()` in
`crates/vexdom-daemon/src/ensure.rs`).

`VEXDOM_PORTS` replaces the port list for the Bridge and the CLI: comma-separated, `0` for a port
the OS picks (`crates/vexdom-core/src/paths.rs`). The extension cannot read it and always walks
the three defaults, so a browser never finds a Bridge moved off them.

---

## The origin gate

Every upgrade is classified before anything else happens (`authorize()` in `http.rs`):

| `Origin` | Role | Requirement |
| --- | --- | --- |
| `chrome-extension://…`, `moz-extension://…`, `safari-web-extension://…` | extension | The pairing handshake below, served by `crates/vexdom-daemon/src/gate.rs` |
| Any other value | none | **Refused** with 403 and `X-Vexdom-Reason: unauthorized`. This keeps web pages out |
| Absent or empty | control | `Authorization: Bearer <token>` equal to the lockfile's token, compared in constant time; served by `crates/vexdom-daemon/src/control.rs` |

Every request, `/health` included, must also carry a loopback `Host` (`127.0.0.1`, `[::1]` or
`localhost`, with an optional port). A page whose DNS points at `127.0.0.1` still sends its own
hostname, so **DNS rebinding gets a 403 before the `Origin` check runs**.

Any web page can open a WebSocket to loopback, but the browser sets `Origin` and page JavaScript
cannot forge it, so a page is rejected outright. Native clients send no `Origin` and land in the
control lane, which needs a token only a process running as you can read. The origin says what
kind of peer this is; the credential says whether this peer is allowed.

---

## The control token and the lockfile

Each Bridge mints a fresh token (24 random bytes, base64url) at start and writes it to the
lockfile, `~/.vexdom/daemon.json` (or `$VEXDOM_HOME/daemon.json`; `VEXDOM_LOCKFILE` overrides the
path), at mode `0600` with `pid`, `port`, `token`, `protocolVersion` and `daemonVersion`
(`crates/vexdom-core/src/lockfile.rs`). The token dies with that Bridge.

`ensure_daemon()` reads the lockfile before every connection, checks the pid is alive and that
`/health` on that port reports the same pid, and otherwise starts `vexdom daemon` (a detached
child, or a launchd job when the caller runs quarantined; see `quarantine.rs`). A start lock
(`daemon.start.lock`) keeps several callers from each starting one.

Print the token with `vexdom token`.

---

## Cap'n Proto over WebSocket

Every message on both sockets is a **binary** WebSocket frame carrying Cap'n Proto RPC bytes as
an unaligned byte stream (ADR-018, ADR-025): where a frame cuts the bytes does not matter, and a
text frame is a protocol error. The schema is
`crates/vexdom-schema/schema/vexdom.capnp`; the adapter is `crates/vexdom-link`. Frames may be up
to 128 MiB.

| Socket | Bootstrap interface | Other interfaces |
| --- | --- | --- |
| Extension | `ExtensionGate` (`hello`, `prove`) | `Page` (served by the extension: `describe`, `invoke`, `screenshot`, `runTask`, `artifact`), `ExtensionUi` (Bridge pushes: run events, agent and skill state, preferences, tasks, notices), `ExtensionSession` (extension requests: `instruct`, `cancel`, `decision`, `ping`, …) |
| Control | `Control` (`status`, `describe`, `invoke`, `snapshot`, `read`, `pair`, `sessions`, `grant`, `decide`, `watch`, `confirmations`, `confirm`, `stop`, …) | `EventSink` for `watch` topics: `manifestChanged`, `pairRequestsChanged`, `noticesChanged`, `settingsChanged`, `confirmationsChanged` |

Documents whose shape the extension owns (action input and results, run context, agent state,
tasks, tool descriptors) travel as `Json` text fields; everything protocol-level is typed.
Screenshots travel as `Data` bytes, and so do the files of `Page.artifact`.

On the extension side, the RPC system runs inside **ext-core**, the wasm module built from
`crates/ext-core` with BoltFFI (ADR-017). JavaScript owns the WebSocket
(`packages/extension/src/lib/bridge/socket.ts`) and drives the wasm with synchronous calls:
`linkOpen` per socket, `linkFeed` with each binary frame, `linkPump` to run until it stalls and
take the bytes to send, `linkEvents` to learn what happened, `linkHello`/`linkProve`/`linkRequest`
to call the Bridge, and `linkReply`/`linkReplyScreenshot`/`linkReplyArtifact` to answer a
`PageCall`. Page calls are answered only after `linkTrust`, which the extension calls once the
Bridge's `welcome` has been verified.

**Keepalive.** The extension calls `ExtensionSession.ping` every 20 s; the Bridge drops a link
silent for 60 s.

---

## Pairing and sessions

The extension connects to nothing until it is paired, by a code you type or by a grant the
native host hands over. Both go through the same handshake. The crypto is in
`crates/vexdom-pure/src/handshake.rs`, which the Bridge and ext-core both run.

1. `vexdom pair` asks the Bridge (`Control.pair`) for a code: 8 characters, valid **10 minutes**,
   single use, at most three live at once.
2. You paste it into the extension and press **Connect**. The extension dials
   `ws://127.0.0.1:<port>/extension`, walking the three ports, and calls `ExtensionGate.hello`
   with its protocol version, the oldest Bridge protocol it accepts, its version, its manifest
   hash, which kind of secret it holds (`pair` or `session`), a fresh nonce, its **install id**
   (16–64 characters, minted once per browser profile) and a browser label. Never the secret.
3. The Bridge answers with a nonce of its own. Both sides now share a transcript: the protocol
   version the extension claimed, its version, the manifest hash and the two nonces.
4. The extension calls `prove` with `HMAC-SHA256(secret, "vexdom/client|transcript")`. For a
   pairing code the secret is not the code but `PBKDF2-HMAC-SHA256(code, "vexdom/pair|nonces",
   250 000 rounds)`, so a recorded handshake does not let anyone grind an 8-character code
   offline. The Bridge runs PBKDF2 on a blocking thread.
5. The Bridge verifies, then proves itself: `Welcome.proof` is
   `HMAC(secret, "vexdom/server|transcript|daemonVersion|manifestHash|manifestInSync|sealedSessionKey")`.
   The labels differ, so an impostor cannot reflect the extension's own proof back.
6. On pairing, the Bridge mints a **session key** (32 random bytes, base64url) bound to the install
   id, and returns it XORed with an HMAC keystream derived from the same secret, so the long-lived
   credential never crosses the wire in the clear. Later connections prove that key. It survives
   restarts of the browser and the Bridge and ends only with `vexdom revoke`.

The install id names a browser profile because the origin cannot: every browser that installs the
same store build, or loads the same unpacked folder, presents the same origin.

A handshake step that does not arrive within 10 s closes the socket.

### Pairing without a code: grants

`vexdom native-host` (`app/src/native_host.rs`) answers one framed message per launch, then exits:

| Op | Answer |
| --- | --- |
| `ensure` | Starts the Bridge, unless `vexdom stop` left the `stopped` marker in the state folder |
| `hello` | The host version, its ops, the pairing mode, whether the Bridge is held, whether `Vexdom.app` is installed. Never starts the Bridge |
| `pair` | A grant, a wait, a refusal, or an error |

Anything else gets `Expected {"op":"ensure"}.`

The browser launches the host only for the extension ids its registration lists, and tells it the
caller (`chrome-extension://<id>/` on Chromium, the add-on id on Firefox). The host asks the Bridge
over the control socket (`Control.grant`). The pairing desk (`crates/vexdom-core/src/pairing.rs`)
answers by pairing mode (`vexdom pairing auto|ask|code`): `code` refuses, `ask` holds the request
until an approver (`Vexdom.app` or `vexdom approve`) decides, and `auto` mints a **grant**: 26
characters, valid **120 s**, single use, bound to the install id and the caller, kept only in
memory. The extension redeems it with the same `pair` hello and PBKDF2 proof a typed code uses
(`redeemGrant()` in `socket.ts`), dialling the grant's port first. The session records
`pairedBy`: `auto`, `approved` or `code`.

`vexdom revoke` deletes a session and blocks its install id (the newest 200 blocks are kept in
`auth.json`), so the extension's next automatic attempt is refused instead of pairing again. A
control connection that speaks for an agent run is refused `grant`, `decide`, `pairing` and
`label`.

### Refusals

A refused `hello` is a Cap'n Proto exception whose text is the reason. A failed `prove` returns
`Unauthorized` with `reason`, `retryable` and `code`:

| Code | Meaning | What the extension does |
| --- | --- | --- |
| `bad-code` | A code or grant that matched nothing | Reports it |
| `unknown` | A session key the Bridge has no session for, and the install is not blocked (a wiped or reinstalled Bridge) | Marks the key stale and pairs again through the host |
| `revoked` | A key for a browser this computer revoked | Marks the key stale and waits for **Reconnect** |

### Why the Bridge proves itself too

The three ports are well known and any local process can bind one first. So a socket that closes
without a verified `welcome` (a squatter, an older Bridge, an `Unauthorized`, 5 s of silence) is
abandoned, and the walk moves to the next port. Only a peer that proved it holds the same secret
gets page calls answered.

### Reconnection

Exponential backoff from 1 s to 30 s with jitter, plus a reconnect alarm (`vexdom/reconnect`)
that re-dials if the worker was stopped in between. When no port answers, the extension asks the
native host to `ensure` the Bridge (`native-wake.ts`) and dials again.

---

## Protocol version

Both sides compile in `SOCKET_PROTOCOL_VERSION`, currently **27**: the first Cap'n Proto version.
The Bridge's `MIN_EXTENSION_PROTOCOL` is also 27, so an extension from before Cap'n Proto (which
speaks JSON) cannot connect. The checks in `gate.rs`:

- A `hello` below 27 is refused with `protocol version mismatch: Vexdom Bridge speaks v27 and takes
  extensions from v27 on`. The extension treats that text as not worth retrying and tells the
  user to update or rebuild.
- A `hello` whose `minDaemonProtocol` is above the Bridge's own version is refused with
  `daemon too old: …`. The extension sends its own version there.
- `Welcome.protocolVersion` carries the Bridge's version, so a newer extension knows what not to
  send.

The rule for changing it: a change a peer can do without (a new method, a new optional field)
bumps `SOCKET_PROTOCOL_VERSION` and is used only with a peer that speaks it (`Link::speaks()` on
the Bridge, `daemonSpeaks()` in the extension). Only a change neither side can do without raises
the minimum. Older thresholds still checked: `NOTICE_PROTOCOL` (24, notices handed to the desktop
app) and `TABS_PROTOCOL` (26, the conversation's tabs in a run's context).

---

## What this does not protect against

Pairing controls **which browser**, not which local process. Anything running as your user can
read the lockfile and drive a paired browser through the control socket. Vexdom treats your user
account as the trust boundary (see [guide/limits.md](../guide/limits.md)).

---

## Next

**[The action registry →](registry.md)**: what can be called once a connection exists.
