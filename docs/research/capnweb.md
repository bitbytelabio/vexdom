# Cap'n Web as an alternative to Cap'n Proto for the extension link and control socket

Researched 2026-11-25 for ADR-014 (`DECISIONS.md:106-116`). Companion files:
`research/capnp-wasm.md` (capnp-rpc in wasm, spike S1) and `research/boltffi.md` (S2).

**Sources and pinned revisions** (line numbers below refer to these):

| Tag | What | Revision |
| --- | --- | --- |
| `CW:` | `github.com/cloudflare/capnweb`, repo root | `0b19df9` (2026-11-20), npm `capnweb@0.13.1` |
| `CW docs/` | `CW:packages/docs/src/content/docs/` (source of capnweb.com) | same |
| `CPR:` | `github.com/capnproto/capnproto-rust` | `033d483` (capnp 0.28.2, capnp-rpc 0.28.0) |
| `capn-rs:` | `github.com/currentspace/capn-rs` | `e80b41a` (2025-09-30) |
| `cwt:` | crates.io `capnweb_types` 0.5.0 tarball | 2026-05-27 |
| `VX:` | this repo | `c55d7f4` |
| Blog | blog.cloudflare.com/capnweb-javascript-rpc-library/, Kenton Varda, 2025-09-22 | fetched 2026-11-25 |

## TL;DR

- Cap'n Web is a schemaless, JSON-over-text object-capability RPC with promise pipelining, by the
  author of Cap'n Proto. It does not interoperate with Cap'n Proto on the wire
  (`CW docs/guides/comparisons.mdx:125-127`).
- There is a written, fairly complete wire spec (`CW docs/reference/protocol.mdx`, 590 lines), and
  the TS source agrees with it on every message type checked below.
- **No usable Rust implementation exists.** The only crate family claiming one (`capnweb-core` /
  `-server` from capn-rs) uses message formats the spec does not have. `capnweb_types` is a
  types-only serde crate. Details are in [Rust implementations](#rust-implementations).
- The Cap'n Web docs themselves say that for a non-JS backend "the answer today is to use Cap'n
  Proto instead" (`CW docs/guides/comparisons.mdx:129-133`). They also say a statically typed port is
  "much harder" (`:141-142`).
- Browser side, it fits MV3 well: pure JS, no `eval` and no `new Function` in `dist/index.js`, about
  16-17 kB min+gzip, and no wasm (so no `'wasm-unsafe-eval'`).
- For vexdom it would mean writing and maintaining our own Rust peer for both directions (client and
  server). The daemon calls the extension (`invoke`) as much as the extension calls the daemon.
- **Recommendation:** keep ADR-014 (Cap'n Proto). Cap'n Web is the named fallback for the extension
  link only, if S1's wasm gates fail. See [Recommendation](#recommendation).

## 1. Wire protocol

### Serialization

- **Format.** JSON with a preprocessing step. A non-JSON value becomes an array whose first element
  is a string type code, e.g. `["date", 1749342170815]` (`CW docs/reference/protocol.mdx:14-43`).
- **Literal arrays are escaped** by wrapping them in a second array: `[["just","an","array"]]`
  (`protocol.mdx:45-50`, `:259-294`). The evaluator treats `value.length == 1 && value[0] instanceof
  Array` as an escaped array (`CW:src/serialize.ts:890-896`).
- **Type codes in the spec:**

  | Group | Codes |
  | --- | --- |
  | Values | `undefined`, `inf`, `-inf`, `nan`, `bytes`, `bigint`, `date`, `error` |
  | Containers | `set`, `map`, `regexp`, `url`, `blob` |
  | Fetch API | `headers`, `request`, `response` |
  | References | `import`, `pipeline`, `remap`, `export`, `promise` |
  | Streams | `writable`, `readable` |

  Source: `protocol.mdx:296-590`. Each one is implemented in `CW:src/serialize.ts:899-1350`.
- **Unknown codes are fatal.** An unknown code throws `unknown special value` (`serialize.ts:1352`).
  The throw escapes `readLoop`, and `readLoop().catch(err => this.abort(err))` aborts the whole
  session (`CW:src/rpc.ts:524`).
- **Prototype keys are dropped.** Keys that shadow `Object.prototype`, plus `toJSON`, are removed
  from incoming objects (`serialize.ts:1355-1371`).
- **Why JSON.** The rationale is native `JSON.parse` speed, small code size, readable traffic, and
  leaving the redundancy to `permessage-deflate` (`protocol.mdx:19-35`).

### Tables and ID conventions

- Each side keeps an **imports** table and an **exports** table. One side's exports are the other
  side's imports (`protocol.mdx:79-84`).
- **Who picks the ID decides its sign.** The importer picks positive IDs from 1 when it initiates a
  call (the ID names the call's result). The exporter picks negative IDs from -1 when it sends a stub.
  ID 0 is the "main" interface (`protocol.mdx:86-97`).
- **Code matches the spec.** Export 0 is the local main and import 0 is the peer's main
  (`rpc.ts:518-522`). Exports count down from -1 (`rpc.ts:474-477`, `:546`).
- **Questions and answers are merged in.** Cap'n Proto's four tables (imports, exports, questions,
  answers) become two (`protocol.mdx:99-101`).
- **IDs are never reused.** Allocation is sequential, on the assumption that a session stays under
  2^53 IDs (`protocol.mdx:107-109`).

### Top-level messages

All eight are in the spec (`protocol.mdx:144-252`) and the receive switch (`rpc.ts:1033-1134`).

| Message | Shape | Semantics |
| --- | --- | --- |
| push | `["push", expr]` | Evaluate `expr`. The result takes the next positive import ID (`protocol.mdx:148-156`, `rpc.ts:1034-1047`). |
| pull | `["pull", importId]` | Ask for a `resolve`/`reject` of that import (`protocol.mdx:158-166`). Sent only when the app awaits (`rpc.ts:247-254`). |
| resolve | `["resolve", exportId, expr]` | Settles a call result or an exported promise. One message covers both, unlike Cap'n Proto's `Return` vs `Resolve` (`protocol.mdx:139-142`, `:168-175`). |
| reject | `["reject", exportId, expr]` | `expr` must contain no stubs (`protocol.mdx:177-185`). |
| release | `["release", importId, refcount]` | `refcount` = the number of times the ID was introduced, +1 if it came from a push. The exporter frees at zero (`protocol.mdx:187-202`, `rpc.ts:573-587`). |
| stream | `["stream", expr]` | A push that is auto-pulled and auto-released, with no pipelining. Used for streaming writes (`protocol.mdx:204-220`, `rpc.ts:1049-1067`). |
| pipe | `["pipe"]` | Creates a remote `TransformStream`. The import is the writable end, and the readable end is referenced once via `["readable", id]` (`protocol.mdx:222-242`, `rpc.ts:1069-1077`). |
| abort | `["abort", expr]` | Fatal. No further messages (`protocol.mdx:244-252`, `rpc.ts:1128-1133`). |

Anything else throws `bad RPC message` and aborts the session (`rpc.ts:1137`).

### Pipelining, capability passing and `map`

- **A call is a push of a pipeline expression:** `["push", ["pipeline", targetImportId,
  propertyPath, args]]`. The client then allocates the next import ID for the result
  (`rpc.ts:803-837`).
- **`pipeline` vs `import`.** On receipt, `pipeline` evaluates to a promise. Promises are resolved
  before delivery to the app. `import` evaluates to a stub, delivered immediately
  (`protocol.mdx:476-496`, `serialize.ts:1176-1253`).
- **Properties are reachable remotely.** `propertyPath` walks properties of the target. With `args`,
  the property found at that path is called (`protocol.mdx:491-496`).
- **Passing objects and functions.** Passing an `RpcTarget` subclass or a function produces
  `["export", negId]`, and the peer gets a stub (`values.mdx:38-45`, `protocol.mdx:530-537`).
- **Re-exports share one ID.** Re-exporting the same hook reuses its ID and increments its refcount
  (`rpc.ts:538-552`).
- **Promises always get a fresh ID** and are resolved proactively, with no pull needed
  (`protocol.mdx:539-551`, `rpc.ts:554-565`).
- **Unawaited calls are never pulled.** A promise the app never awaits is not pulled, so its result
  never crosses the wire, but it can still be pipelined (`protocol.mdx:131-133`; blog,
  "RPC protocol").
- **`.map()` is record-replay, not code.** The client runs the callback once with placeholder
  stubs, records the RPCs as instructions and sends them as `["remap", importId, path, captures,
  instructions]` (`CW docs/concepts/map.mdx:168-192`, `protocol.mdx:498-528`).
- **`remap` indexing.** Inside instructions, negative indexes are captures, 0 is the input and
  positive indexes are earlier instructions (`protocol.mdx:520-528`).
- **`remap` receive side.** The server replays the recording through `MapApplicator`
  (`CW:src/map.ts:242-276`).
- **Which side builds a recording.** Any peer can *evaluate* a `remap`. Only a language with
  reflection or expression trees can conveniently *produce* one (`comparisons.mdx:187-192`).

### Disposal and refcounting

- **No GC across the wire.** Remote references are not garbage-collected
  (`CW docs/concepts/disposal.mdx:8-28`). Stubs implement `Symbol.dispose` and work with `using`
  (`disposal.mdx:39-50`). The library polyfills `Symbol.dispose` and `Promise.withResolvers`
  (`CW:src/core.ts:8-22`).
- **Ownership: the caller disposes everything.** Stubs in params are dup'd on the wire and the
  callee's copies are disposed when the call returns. Stubs in results move to the caller
  (`disposal.mdx:57-71`).
- **Keeping a callback needs `.dup()`.** A callee that keeps a callback past the call that delivered
  it must `.dup()` it (`disposal.mdx:104-142`, `CW docs/transports/websocket.mdx:61-74`).
- **Disposing an unresolved import** sends `release` with the accumulated remote refcount and
  rejects local waiters (`rpc.ts:256-297`, `:926-931`).

### Is there a written spec?

- **Yes:** `CW docs/reference/protocol.mdx` ("what actually goes over the socket… you do need it to
  implement an interoperating peer", `:11-12`).
- **It is meant as a spec.** The docs call the wire protocol "a specification you can implement
  yourself" and note that the framing, tables and expression language need no JS peer
  (`comparisons.mdx:177-185`).
- **Gaps (checked against the source):**
  - Embargoes are unspecified. The implementation has a `TODO: Need embargo handling here?`
    (`rpc.ts:201-208`).
  - There is no clean-shutdown message (`rpc.ts:532-536`).
  - There is no version or feature negotiation. Limits are "a purely local, receiver-side decision…
    the protocol has no negotiation step" (`rpc.ts:460-462`).
  - The exact property-access rules for `RpcTarget` are documented in prose only.

### Protocol churn (wire forms added in minor releases)

18 stable releases from 0.0.1 (2025-09-12) to 0.13.1 (2026-11-20), according to the npm registry
`time` field. Wire-visible additions:

| Version | Added | Source |
| --- | --- | --- |
| 0.2.0 | `inf` / `-inf` / `nan` | `CW:CHANGELOG.md:183` |
| 0.5.0 | Streams: `stream`, `pipe`, `writable`, `readable` | `CHANGELOG.md:153` |
| 0.8.0 | `blob`, and error `props` | `CHANGELOG.md:115` |
| 0.9.0 | Encoding levels. The MessagePort wire format changed, so both ends must upgrade | `CHANGELOG.md:107-110` |
| 0.11.0 | Typed byte containers, `url` | `CHANGELOG.md:68-70` |
| 0.13.0 | `map`, `set`, `regexp` | `CHANGELOG.md:13-24` |

A peer that receives a form it does not know aborts the session (see Serialization above).

## 2. Transports

### Transport interface

- **Default interface:** string `send(message)`, `receive(): Promise<string>`, and an optional
  `abort` (`rpc.ts:13-44`, `CW docs/transports/custom.mdx:10-38`).
- **Encoding levels** let a transport take JS value trees instead of strings:
  - `jsonCompatible`
  - `jsonCompatibleWithBytes`: `Uint8Array` stays raw, so CBOR or MessagePack can avoid base64
  - `structuredClonable`

  Sources: `rpc.ts:50-85`, `custom.mdx:65-90`.
- **Framing is the transport's job.** WebSocket and MessagePort carry one RPC message per transport
  message. HTTP batch is newline-delimited (`protocol.mdx:63-77`).

### WebSocket

- **Takes an open socket.** `newWebSocketRpcSession(webSocket: WebSocket | string, localMain?,
  options?)` accepts an already-open socket (`CW:src/websocket.ts:13-22`).
- **Text frames.** The default `WebSocketTransport<string>` sends text frames, and
  `WebSocketTransport<ArrayBuffer>` is a building block for binary ones (`websocket.ts:44-53`,
  `:101-108`).
- **Queues sends** while the socket is `CONNECTING` (`websocket.ts:55-67`).
- **Close or error breaks the session.** Either one rejects `receive()`, which breaks the session
  (`websocket.ts:85-91`, `:143-152`). `abort` closes with code 3000 (`websocket.ts:123-141`).

### Bidirectional calls on one socket

- **Fully symmetric.** Either side may expose a main object and call the other's
  (`protocol.mdx:52-61`, `custom.mdx:61-63`, `websocket.mdx:50-74`).
- **Missing main object.** Without a `localMain`, calls into that side fail with "This connection
  has no main object." (`rpc.ts:1172-1178`).

### Other transports

- **HTTP batch:** one request = one session, which suits stateless use (`CW docs/guides/sessions.mdx:16-20`).
- **MessagePort** (structured clone since 0.9.0). Possibly useful later for TS↔TS links between the
  SW, the offscreen document and the side panel. That is out of scope here.

## 3. Disconnect and reconnect

- **Session = socket.** A WebSocket session lasts exactly as long as its socket. Nothing persists,
  and export IDs are "like serializing a file descriptor" (`sessions.mdx:11-25`).
- **No automatic reconnect.** On drop, "every stub from that session is permanently broken". Pending
  calls reject and new calls fail. Nothing reconnects automatically (`sessions.mdx:67-79`,
  `websocket.mdx:99-113`).
- **Recovery pattern:** one owner of the root stub that re-derives everything
  (`sessions.mdx:81-117`), plus resumable subscriptions keyed by a client cursor
  (`sessions.mdx:138-152`, `CW:examples/session-recovery/README.md:10-21`).
- **`onRpcBroken`** fires on disconnect (`disposal.mdx:154-168`). It cannot be unregistered, and
  registering twice fires twice (`sessions.mdx:132-135`, issue #234).
- **Third-party reconnect wrapper:** `VastBlast/capnweb-auto-reconnect` exists (1 star, last push
  2026-03-11, from `gh search repos capnweb`). Not evaluated.
- **Comparison.** At Level 1, capnp-rpc has the same per-connection capability lifetime. It also
  ships `auto_reconnect` / `lazy_auto_reconnect` client wrappers (`CPR:capnp-rpc/src/reconnect.rs:318`,
  `:339`).

## 4. Binary data (screenshots)

- **Base64 in JSON.** At the default `string` level, bytes are sent as `["bytes", base64]`: the
  standard alphabet with padding stripped (`serialize.ts:404-448`, strip at `:447`;
  `protocol.mdx:312-327`). That is the "~33% base64 size overhead" plus encode and decode CPU
  (`custom.mdx:85-87`).
- **No worse than today.** The current link already ships screenshots as base64 data URLs inside
  JSON (`VX:packages/extension/src/lib/bridge/pick.ts:84`).
- **Raw bytes are possible.** A custom transport at `jsonCompatibleWithBytes` level with CBOR or
  MessagePack over binary frames keeps raw bytes (`custom.mdx:71-87`, `serialize.ts:421-430`). The
  Rust peer would then need the same CBOR mapping, and traffic is no longer readable in DevTools.
- **Size limits.** The default receive limits are `maxMessageSize` = 32 Mi UTF-16 code units,
  `maxDepth` = 256 and `maxBigIntDigits` = 16384 (`serialize.ts:40-72`). They are enforced only after
  the transport has buffered the whole message (`rpc.ts:1014-1025`, `custom.mdx:135-140`).
- **Contrast with Cap'n Proto:** `Data` is raw bytes in a binary frame. The default reader
  traversal limit is 8 Mi words (= 64 MiB) (`CPR:capnp/src/message.rs:117-119`).

## 5. Typing and validation

- **No IDL.** The TypeScript interface is the contract (`CW:README.md:14-17`, `:41`).
- **No runtime checks by default.** "As always with TypeScript, no type checks occur at runtime…
  use a runtime type-checking system like Zod" (blog, "TypeScript"). The docs add that "at runtime
  that signature is a comment" (`CW docs/guides/validation.mdx:8-10`).
- **`capnweb-validate`** generates validators from the TS types with a bundler plugin
  (`validation.mdx:12-31`). It depends on `typescript >=5.7.0 <7` directly, so it still works in a
  TS 7 workspace (`validation.mdx:66-69`).
  - vexdom is on `typescript ^7.0.2` and already ships `zod ^4.4.3` (`VX:package.json`).
- **Evolution rules** are JS-API rules (add methods and optional trailing params; never rename).
  There are no field numbers (`sessions.mdx:154-178`).
- **Rust side.** No schema means no codegen. The Rust peer would need hand-written serde types that
  mirror the TS interfaces, or generated TS from Rust (e.g. ts-rs or specta; not evaluated). Either
  way the source of truth is not shared mechanically, unlike a `.capnp` file compiled by `capnpc`.

## 6. Size and CSP in an MV3 service worker

- **Bundle.** I measured the npm `capnweb@0.13.1` tarball (`dist/index.js`, the default `import`
  entry per `CW:package.json` `exports`):

  | Form | Size |
  | --- | --- |
  | Unminified | 107 kB (22.9 kB gzip) |
  | `bun build --minify --target=browser` (bun 1.4.3) | 62.2 kB |
  | Minified + `gzip -9` | 16.9 kB |
  | Minified + brotli q11 | 15.1 kB |

  - The README says "under 16 kB" min+gzip (`CW:README.md:25`). That figure comes from esbuild in
    `CW:packages/docs/scripts/measure-bundle.mjs:27-39`.
  - The blog's launch claim was "under 10 kB" (blog, bullets). The measuring script admits that
    number went stale (`measure-bundle.mjs:4-6`).
  - Dependencies: none (`npm view` shows no `dependencies`).
- **No `eval`, no `new Function`, no `import(`** in `CW:src/*.ts` (grep: 0 hits) or in
  `dist/index.js` (grep: 0 hits).
- **The Workers-only path stays out of browser bundles.** The static `import "cloudflare:workers"`
  exists only in the `workerd` export condition (`CW:src/inject-workers-module.ts:13-14`,
  `CW:package.json` `exports`). A browser or WXT build resolves to `dist/index.js`.
- **CSP fit.**
  - MV3 extension pages and workers share one policy. The minimum is `script-src 'self'
    'wasm-unsafe-eval'; object-src 'self'`, and `'unsafe-eval'` is rejected
    (developer.chrome.com/docs/extensions/reference/manifest/content-security-policy).
  - Cap'n Web is plain JS from `'self'`, so it needs no CSP change. capnp-in-wasm needs
    `'wasm-unsafe-eval'` (see capnp-wasm.md).
- **Runtime APIs used:** `Proxy`, `TransformStream`, `TextEncoder` and `WebSocket`, all present in
  SWs. `Symbol.dispose` and `Promise.withResolvers` are polyfilled (`core.ts:8-22`). The README says
  it runs in all major browsers (`CW:README.md:22-23`).
  - Firefox MV2 background pages: expected to work as plain JS. Unverified, part of the spike.
- **SW lifecycle.**
  - Chrome terminates an idle SW after 30 s. Since Chrome 116, sending or receiving WebSocket
    messages resets that timer (developer.chrome.com/docs/extensions/develop/concepts/service-workers/lifecycle,
    "Idle and shutdown" > "Chrome 116"). Globals are lost on shutdown ("Persist data rather than
    using global variables").
  - The Chrome WebSocket guide recommends a 20 s keepalive and `minimum_chrome_version: "116"`
    (developer.chrome.com/docs/extensions/how-to/web-platform/websockets).
  - vexdom already pings every 20 s from the daemon (`VX:src/daemon/extension-link.ts:21`, `:75`) and
    answers `pong` (`VX:packages/extension/src/lib/bridge/socket.ts:673-674`).
  - Under Cap'n Web, a non-RPC frame on the session socket aborts the session (`rpc.ts:1137`). The
    ping therefore has to become an RPC call or be filtered out in a custom transport.

## 7. Rust implementations

Searches run 2026-11-25:

- crates.io API: `q=capnweb`, `cap-n-web`, `capn-web`, `capnweb-rs`, `cap'n web`.
- `gh search repos`: `capnweb`, `cap'n web`, `capn-rs`, `capnweb language:rust`.
- `gh search code "capnweb" --language rust`.

| Hit | What it is | Maturity | Spec conformance |
| --- | --- | --- | --- |
| `capnweb` 0.1.0 (crates.io, no repo) | Name placeholder: `pub fn capnweb() { panic!("TBD") }` (crate `src/lib.rs`) | 30 downloads, published 2026-04-14 | None |
| `capnweb-core` / `-server` / `-client` / `-transport` 0.1.0 (`currentspace/capn-rs`) | ~22k LOC Rust, 93 commits, all by one author, written 2025-09-23..30, no commits since. Has a `CLAUDE.md`. | 82 stars. Downloads 313-1163 per crate. README claims "production-ready" (`capn-rs:README.md:1-25`). The crates it says it renamed to (`currentspace-capnweb-*`) do not exist on crates.io. | **Non-conformant.** `release` is `["release", [ids…]]` (`capn-rs:capnweb-core/src/protocol/wire.rs:26-27`, `:159-173`), vs the spec's `["release", id, refcount]`, which has been in the spec since at least v0.2.0 (`protocol.md` at tag `v0.2.0`, line 87). It invents `["call",…]` and `["capref",…]` (`wire.rs:71-85`; issue #20 "Where did capref come from?"). No `bytes`, `bigint`, `undefined`, streams or `stream`/`pipe`; `remap` kept as raw JSON (`wire.rs:35-86`). Issue #21: compliance tests file is `.disabled`. Issue #22 (2025-10-11): "This codebase is complete garbage… worst pathologies of overexuberant LLM usage". **Do not use.** |
| `capnweb_types` 0.5.0 (`Dr-Emann/capnweb_types`) | Serde types for `Message` and `Expression` only. No session, tables or evaluator (`cwt:src/lib.rs:1-9`, `:50-110`). MIT OR Apache-2.0. The author is a capnweb contributor (4 commits per the GitHub contributors API). | 1 star, 1136 downloads, last release 2026-05-27 | Matches the spec for `release` (`cwt:src/lib.rs:93-108`, `:163-166`) and includes stream, pipe and blob. Lacks `set`, `map`, `regexp`, `url` (0.11/0.13 additions; grep of `src/` finds none). **Usable as a starting codec,** after extending it. |
| `dot-do/sdks` `packages/rust/capnweb` | Part of a 20-language SDK dump made in 2 days (repo languages API) | 0 stars, 74 open issues, last push 2026-01-13 | Self-described: "This SDK is currently a stub implementation for conformance testing" (`packages/rust/capnweb/src/lib.rs:25-28`) |
| `cloudflare/forge` `src/bin/forge/capnweb.rs` | A Rust program that *generates a TypeScript* Cap'n Web server ("thin layer over the TypeScript SDK", lines 1-20) | Active (2026-11-10) | Not a protocol implementation |
| `wolfymaster/woofx3-ui` `companion/core/src/host/capnweb.rs` | App-internal HTTP-batch client subset: push/pull/resolve/reject, goldens captured from capnweb 0.4.0 (lines 1-13) | 1 star | Subset, not a library. Shows a hand-rolled subset is small. |
| `changesbyjames/iroh-capnweb` `packages/browser/wasm` | iroh transport in wasm (ALPN `capnweb/iroh/1`) | — | Transport only |

**Other languages, for scale:** `flaticols/capnweb-go` (~180 kB Go source), `nnevatie/capnwebcpp`
(~190 kB C++), `abilian/py-capnweb` (38 stars). Sizes are from each repo's GitHub languages API.
None were reviewed for conformance.

**Upstream direction.**

- Issue #179, "Cap'n Proto bridge" (dimitropoulos, 2026-05-22), proposes a `.capnp`-defined
  interface, generated TS types and a protocol-translating proxy. The heavy lifting is said to exist
  in workerd, but it is not public.
- On 2026-08-21 kentonv replied: "The team plans to work on this in the coming months… we generally
  prefer to keep it in house."
- Issue #181, "Cap'n Proto in Wasm" (same author), has no comments.
- Neither has shipped as of `0b19df9`.

### Effort to write a Rust peer on tokio (estimate, not measured)

- The reference runtime is about 6.3k lines of TypeScript including comments (`wc -l` of `core.ts`,
  `rpc.ts`, `serialize.ts`, `map.ts`, `streams.ts`, `batch.ts`, `websocket.ts`, `messageport.ts`).
- vexdom needs **both roles**. The daemon calls the extension (`invoke` → `result`,
  `VX:packages/extension/src/lib/bridge/socket.ts:737-742`), and the extension calls the daemon. The Rust side is
  therefore a full symmetric session, not just a server.
- The control socket (CLI/MCP/GPUI, ADR-014) would also need the Rust *client* to talk Rust↔Rust,
  which is exactly the case Cap'n Proto already covers.

| Piece | Notes | Rust LOC (est.) |
| --- | --- | --- |
| Codec | Expression ↔ value tree: array escaping; `bytes` (base64 std, no pad, optional type tag); `undefined`/`inf`/`nan`/`bigint`/`date`; `error` with optional `stack`/`props`. Start from `capnweb_types` and add `set`/`map`/`regexp`/`url`. Prototype-key filtering is irrelevant in Rust, but unknown tags must be rejected the same way. | 600-1,000 |
| Session core | Import/export tables with sign conventions. Export refcount by introductions; reverse map hook→ID; `release` with counts. Pull tracking; resolve/reject routing; `abort`; broken-callback fan-out. | 900-1,400 |
| Dispatch + pipelining | `RpcTarget`-like trait `call(path, args) -> Future<Result<Value>>` plus property-path traversal on results. Queued calls on unresolved promises. Promise-in-args resolution before delivery. Capability values inside `serde_json`-like trees. Likely a derive macro to map method names. | 1,000-1,800 |
| `remap` evaluator | Captures (negative), input (0), prior results (positive); per-element on arrays; null passthrough. We would never *produce* recordings from Rust. | 300-500 |
| Streams (`stream`/`pipe`/`writable`/`readable`) | Skippable if our API uses callback capabilities instead of `ReadableStream` | 0 or 500-800 |
| WS transport | axum/tokio-tungstenite text frames, one message per frame. HMAC handshake first. Ping handling. | 150-300 |
| Conformance harness | Node running the official `capnweb` as the peer; golden transcripts for each message type; refcount leak checks via `getStats()` (`rpc.ts:1153-1163`); fuzzing malformed input | 600-1,200 |

**Total:** about 3.5k-6k Rust LOC, roughly **4-8 engineer-weeks** to a conformant MVP without
streams, plus tracking upstream wire additions (six releases added wire forms in 14 months, see
above). This is a judgment from the piece list, not a measurement. Two parts are the uncertain bulk:
embargo-free ordering subtleties and the ownership and dispose rules.

## 8. Head-to-head for vexdom

| Concern | capnp-rpc (Rust daemon) + capnp-rpc in wasm (extension) | Cap'n Web |
| --- | --- | --- |
| Rust daemon side | Mature crate. Deps are only `futures` (std) and capnp (`CPR:capnp-rpc/Cargo.toml:17-24`). Level 1 including embargoes: `handle_disembargo` (`CPR:capnp-rpc/src/lib.rs:22`, `rpc.rs:902`, `:1197`). Two-party `VatNetwork::new(AsyncRead, AsyncWrite, side, opts)` (`twoparty.rs:214-252`). | Nothing usable. We write and own a 3.5k-6k LOC peer (§7). |
| Control socket (Rust↔Rust) | Native fit: one schema and one bootstrap (ADR-014, `DECISIONS.md:111-115`) | We would need a Rust client too. Upstream says use Cap'n Proto for non-JS backends (`comparisons.mdx:129-133`). |
| TS extension side | Two options. (a) The endpoint runs in wasm behind a binding. capnp-wasm.md measured the capnp-rpc runtime plus a WS adapter at 341 KB raw / 111 KB gzip after wasm-bindgen, with no schema code and no wasm-opt; it needs `'wasm-unsafe-eval'`. (b) TS-native capnp RPC. ADR-014's "JS capnp libs … have no RPC" (`DECISIONS.md:108-110`) is outdated: `capnp-ts` 0.9.3 (beta) lists "RPC Level 1: implemented (… browser-compatible WebSocket transport)" (github.com/jdiaz5513/capnp-ts README, "Project Status"). | Native: about 16-17 kB gzip pure JS, default CSP, no wasm (§6) |
| ADR-015 (logic moves into wasm) | The RPC endpoint itself can live in `ext-core`, giving typed Rust on both ends with no JS hop for RPC payloads | The endpoint stays in TS. Payloads cross JSON → JS objects → binding → wasm. Upstream warns that a Rust/WASM core "would spend most of its size marshalling values across the JS/WASM boundary" (`comparisons.mdx:143-146`). |
| Binary screenshots | `Data`: raw bytes in a binary frame | base64 in text JSON (+33%, as today) unless we build a custom CBOR transport on both ends (§4) |
| Debuggability | Binary frames need `vexdom debug decode` (`DECISIONS.md:116`) | Readable JSON in a WebSocket inspector (`protocol.mdx:35`, `values.mdx:64-67`). Whether DevTools for the extension SW shows WS frames: unverified. |
| Front-door WS proxy (plan risk, `~/.claude/plans/m-nh-mu-n-rewrite-browsentic-eager-dawn-of-0b7c5175-24ee-41a7-9bd4-2325aa850cab.md:133`) | Must pass binary frames | Frames stay JSON text, so the current pass-through assumption holds |
| Schema / type safety | `.capnp` is the single source of truth. Rust codegen via `capnpc`. TS types come from the binding, not from the schema. | TS interface only. Rust mirror written by hand; runtime validation via zod or `capnweb-validate` (§5) |
| Versioning | Field ordinals and schema evolution rules (capnproto.org/language.html, "Evolving Your Protocol"; not re-read for this note). Our handshake carries `protocolVersion` (`VX:packages/extension/src/lib/actions/handshake.ts:14-20`). | No schema-evolution mechanism (`sessions.mdx:156-158`). No wire-version negotiation. Unknown tags abort the session (§1). Our handshake `protocolVersion` would have to pin the Cap'n Web wire level. |
| HMAC handshake before bootstrap | Run the JSON `hello`/`challenge`/`prove`/`welcome` frames (`socket.ts:576-590`, `:676-731`), then start `VatNetwork` on the same socket through the byte-stream adapter | Same shape: run the handshake, then wrap the open socket (`websocket.ts:13-22`) or use a custom transport. See trap 2 for the welcome race. |
| MV3 SW kill / reconnect | Caps die with the connection. The extension re-dials and re-bootstraps. `auto_reconnect` helpers exist (`reconnect.rs:318`). | Caps die with the session. No reconnect helper (`sessions.mdx:67-79`). Same re-dial, then rebuild the root stub. |
| Maturity | capnp-rpc 0.28.0, about 4.9M downloads (crates.io API, 2026-11-25); C++ reference implementation used by Cloudflare Workers | 0.13.1, 4,047 stars, 23 open issues + 20 PRs (2026-11-25). Blog: "still new and experimental" / "highly experimental". 113 of the commits are by kentonv (contributors API). |

## Semantic traps

1. **Release refcounts must be exact.**
   - A `release` larger than the export's count throws "refcount would go negative" inside the
     receive loop and aborts the session (`rpc.ts:578-580`).
   - Re-exports bump the count (`rpc.ts:541-544`). A Rust peer must count introductions per ID
     exactly like `ImportTableEntry.remoteRefcount` (`rpc.ts:191-192`, `:292-297`).
2. **The welcome race on handoff.**
   - The extension handles frames with `void handle(...)` (`socket.ts:592-594`). The `welcome`
     branch awaits `crypto.subtle` before it finishes (`socket.ts:691-731`).
   - If the daemon starts RPC (e.g. calls `invoke` on the extension's main) right after `welcome`,
     those frames reach the old JSON handler.
   - The transport must be created, buffering, *before* the welcome is verified, or the daemon must
     wait for the extension's first RPC message.
3. **No foreign frames on the session socket.** Any non-RPC message (ping, legacy frame) throws
   `bad RPC message` and aborts the session (`rpc.ts:1137`). Keepalive must be an RPC method or be
   filtered out in a custom transport.
4. **Unknown wire tags are fatal.**
   - Newer peers emit new forms such as `map`, `set` and `regexp` (0.13.0). An older peer aborts the
     session on them (`serialize.ts:1352`, `rpc.ts:524`).
   - Upgrading the npm package can therefore break a Rust peer that has not caught up. Pin the exact
     version and gate it in the handshake.
5. **Embargoes are not implemented** (`rpc.ts:201-208`). When a promise resolves to a capability
   that points back to the caller, call ordering is not guaranteed the way Cap'n Proto's
   `Disembargo` guarantees it (`CPR:rpc.rs:902`). This is rare in a two-party daemon↔extension
   setup, but it is a behavioral difference if we ever port semantics across.
6. **Callbacks need `.dup()`.** A stub received as a param is disposed when the call returns
   (`disposal.mdx:57-71`).
   - Watch or stream callbacks the daemon keeps must be dup'd, and later released, on the Rust side.
   - Otherwise the extension's callback dies right after `subscribe()` returns, or leaks forever.
     There is no export-count limit (`sessions.mdx:197-200`).
7. **Stubs are functions.** `typeof stub === 'function'` because a stub is a `Proxy` over a function
   (`sessions.mdx:120-124`). Anything that sniffs "is this a callback?", including our own
   serializers and React state, misbehaves.
8. **`pull` is lazy.** Results are not sent until awaited (`protocol.mdx:131-133`). A Rust client
   that never pulls never gets errors, and a Rust server must not assume a `pull` will come.
9. **Errors lose their stacks** by default (`values.mdx:61-62`, `rpc.ts:443-453`). Error `props`
   that cannot be represented are silently dropped (`protocol.mdx:408-412`).
10. **Limits apply after buffering.** `maxMessageSize` counts UTF-16 code units, not bytes, and is
    checked after the whole frame is buffered (`serialize.ts:56-63`, `rpc.ts:1014-1025`). Set
    socket-level frame limits on both ends for large screenshots.
11. **MV3 globals vanish** when the SW is killed (Chrome lifecycle doc). Every stub, export and
    callback registration is gone, so the daemon must treat a reconnect as a brand-new session and
    re-subscribe from a cursor (`sessions.mdx:138-152`).

## Spike checklist

Run this as **S1b**, only if S1 (capnp-wasm) fails a gate: wasm compile, SW load under CSP, bundle
budget, or reconnect. Use throwaway code in `spikes/capnweb/`.

1. **TS-only feasibility (half a day).** Add `capnweb@0.13.1`, pinned exactly, to a WXT MV3 test
   extension. Open a `newWebSocketRpcSession` from the SW to a Node `ws` server running the official
   `capnweb`.
   - Pass: it loads under the default MV3 CSP with no CSP violation in `chrome://extensions` errors,
     and a call round-trips.
   - Pass: the SW bundle grows by ≤ 20 kB gzip.
   - Fail: either one does not hold.
2. **Handshake handoff.** Run the existing `hello`/`challenge`/`prove`/`welcome` on the raw socket.
   Then swap to a custom `RpcTransport` that buffers from the moment `welcome` arrives (trap 2).
   - Pass: across 1,000 scripted reconnects where the daemon calls the extension's main immediately
     after `welcome`, there are zero lost or misrouted frames.
3. **Rust peer MVP on tokio.**
   - Scope: codec (start from `capnweb_types`), tables, push/pull/resolve/reject/release/abort,
     `pipeline`/`import`/`export`/`promise`, `bytes`/`error`/`undefined`.
   - Stack: tokio-tungstenite or axum WS.
   - Conformance: drive it from the official TS client with a test API covering:
     - plain calls
     - a pipelined chain of 3 calls in one round trip, with one WS frame batch observed
     - a callback passed TS→Rust and invoked later after `.dup()`
     - a Rust→TS call on the extension's main
     - error propagation
   - Pass: all cases pass, and after disposing everything `session.getStats()` on the TS side
     returns `{imports: 1, exports: 1}` (main only; `rpc.ts:1153-1163`).
   - Fail: any refcount leak or session abort.
4. **Screenshot payload.** Round-trip a 2 MB JPEG as `Uint8Array` both ways. Record p50 and p95
   latency and bytes on the wire against capnp `Data` from S1.
   - Pass: p95 within 1.5× of capnp, or under 100 ms on loopback.
5. **AX snapshot payload.** Send a ~2k-node snapshot (the same fixture as S1). Record encode, wire
   and decode time against JSON v26 and capnp.
   - Pass: no worse than JSON v26.
6. **SW kill.** Stop the worker via `chrome://serviceworker-internals` → Stop.
   - Pass: `onRpcBroken` fires on the TS side when the SW returns. The extension re-dials,
     re-handshakes and re-subscribes from a cursor with no duplicated or lost events. The Rust side
     drops the old session's exports with no leak (count of live sessions = 1).
7. **Keepalive.** Model the 20 s ping as an RPC method on the extension's main.
   - Pass: the SW stays alive 10 minutes idle on Chrome ≥ 116 with no other events.
8. **Firefox MV2 background page.** Repeat steps 1, 3 and 6.
   - Pass: identical results.
9. **Robustness.** Feed the Rust peer 10k fuzzed frames: malformed JSON, unknown tags, wrong
   refcounts, deep nesting.
   - Pass: no panic. It sends `abort` and closes.
10. **Effort gate.**
    - Fail S1b if step 3 takes more than 2 engineer-weeks.
    - Fail S1b if conformance needs changes to the upstream `capnweb` package.

## Recommendation

**Keep ADR-014 (Cap'n Proto) as the plan. Do not adopt Cap'n Web for the control socket under any
outcome.** If S1 fails its wasm or SW gates, try fallbacks for the *extension link only*, in this
order:

1. TS-native `capnp-ts` RPC. It keeps the single `.capnp` schema and leaves the daemon untouched (see
   capnp-wasm.md, Recommendation).
2. Cap'n Web (this spike, S1b).

Reasons:

- There is no conformant Rust implementation. The only "full" one is non-conformant (§7). We would
  own a 3.5k-6k LOC symmetric peer and chase upstream wire additions with no version negotiation
  (traps 1 and 4).
- Upstream itself steers non-JS backends to Cap'n Proto (`comparisons.mdx:129-133`). Its planned
  Cap'n Proto bridge (#179) is not public.
- ADR-015 moves logic into Rust/wasm. capnp-rpc lets the RPC endpoint live there. Cap'n Web pins the
  endpoint in TS and adds a marshalling hop (`comparisons.mdx:143-146`).
- Cap'n Web's real advantages are about 16-17 kB of pure JS, no CSP change, readable frames and no
  binary-proxy concerns. Those matter only if capnp-in-wasm proves too big or too fragile in the SW.
  S1 measures exactly that.

Revisit if Cloudflare ships the Cap'n Proto ↔ Cap'n Web bridge. A `.capnp` schema generating TS types
plus a proxy would let the daemon stay on capnp-rpc while the extension uses Cap'n Web natively.
