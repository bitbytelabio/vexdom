# Cap'n Proto RPC in wasm over a browser WebSocket (spike S1 prep)

Scope: ADR-014 (`DECISIONS.md:106-117`) and spike S1 in the port plan. Question: can `capnp` /
`capnp-rpc` 0.28 run in the MV3 service worker (and in the Firefox MV2 background page) over the
existing daemon WebSocket, with the HMAC handshake kept in front of bootstrap?

Sources pinned for this note:
- capnproto-rust at `033d483` (2026-11-22, "prepare for capnp-v0.28.2 release"), cited below as `capnp/…`,
  `capnp-rpc/…`, `capnp-futures/…`, `capnpc/…`. Versions on crates.io: capnp 0.28.2, capnp-rpc 0.28.0,
  capnp-futures 0.28.0, capnpc 0.28.1 (https://crates.io/api/v1/crates/capnp-rpc, queried 2026-11-25).
- vexdom at `c55d7f4`.
- wasm-bindgen 0.2.130 / js-sys 0.3.107 / wasm-bindgen-futures 0.4.80 from `~/.cargo/registry/src/index.crates.io-*/`.
- Rust std from rustc 1.100.0 (`$(rustc --print sysroot)/lib/rustlib/src/rust/library/std/src`).
- wetware/wasm-capnp-async at `83b40446` (2025-10-17).
- A throwaway compile check I built for this note. It is not in the repo: `<session scratchpad>/wasmcheck`.

## TL;DR

- **It compiles.** capnp-rpc 0.28's two-party `RpcSystem`, driven over a hand-written WebSocket
  byte-stream adapter, builds for `wasm32-unknown-unknown` on stable rustc 1.100.0 with default
  features. It needs no `getrandom`, no `std::time` and no feature flags (see "Compile check").
  Nothing was run in a browser yet.
- **The wasm costs about 290 KB raw / 91 KB gzip / 75 KB brotli** for the RPC runtime alone. This is
  after wasm-bindgen, before wasm-opt, with no schema code. The full module measured 341,196 B raw.
- **Transport:** `twoparty::VatNetwork::new` takes any `futures::AsyncRead + Unpin + 'static` and
  `AsyncWrite + Unpin + 'static`, and does not require `Send` (`capnp-rpc/src/twoparty.rs:243-251`).
  Writing a WebSocket adapter of about 80 lines is enough. Two caveats:
  - **The frame boundary is not the message boundary.** `write_message` never flushes
    (`capnp-futures/src/serialize.rs:199`), and the write queue flushes once per batch of messages
    (`capnp-futures/src/write_queue.rs:83-110`).
- **Executor:**
  - In wasm, `RpcSystem` is a non-`Send` future (`capnp-rpc/src/lib.rs:231-233`). Drive it with
    `wasm_bindgen_futures::spawn_local`, which schedules through `queueMicrotask` (`js-sys/src/futures/queue.rs:63-70`).
  - On the daemon, use a current-thread runtime plus `LocalSet` (`capnp-rpc/examples/hello-world/server.rs:58-79`).
    Put it on a dedicated thread, because `LocalSet::run_until` cannot run inside a `tokio::spawn` task
    (https://docs.rs/tokio/1.54.1/tokio/task/struct.LocalSet.html, `run_until`).
- **Three MV3 musts:**
  - **CSP:** the default MV3 CSP disables WebAssembly, so the manifest must declare
    `"extension_pages": "script-src 'self' 'wasm-unsafe-eval'; object-src 'self';"`.
  - **No top-level await:** the wasm must be initialised without top-level await, from listeners
    registered synchronously.
  - **Keepalive:** a WebSocket keeps the service worker alive only while messages flow (Chrome 116+).
    Today's 20 s daemon ping (`src/daemon/extension-link.ts:21,75`) must survive the switch to binary.
- **The blog is about WASI, not browsers.** https://mikel.xyz/posts/capnp-in-wasm/ (2025-10-03) runs
  capnp-rpc 0.21 over WASI-p2 stdin/stdout on wasm32-wasip2. Its transport pattern carries over. Its
  self-waking `poll_read` must **not** be copied into a browser: it becomes an endless microtask loop
  that starves WebSocket events (see "Semantic traps").
- **Correction to ADR-014's context: JavaScript Cap'n Proto RPC exists.** ADR-014 says JS libraries
  "are alpha and have no RPC". That is outdated.
  - `capnp-ts` 0.9.3 (2026-08-21) calls itself beta. It lists "RPC Level 1: implemented (two-party;
    object references, promise pipelining, browser-compatible WebSocket transport)"
    (https://github.com/jdiaz5513/capnp-ts README, "Project Status").
  - `capnp-es` 0.0.17 (alpha) carries experimental level-1 RPC (https://github.com/unjs/capnp-es
    README, "RPC Protocol").
  - That gives a TypeScript-side fallback if the wasm path fails a gate.

## The blog (mikel.xyz) and what applies

Posts read:
- https://mikel.xyz/posts/capnp-in-wasm/ (2025-10-03).
- The tag pages https://mikel.xyz/tags/capn-proto/ and https://mikel.xyz/tags/webassembly/. Besides
  the main post, they list:
  - `/posts/perfbasics` (2025-06-24): slides only. Cap'n Proto appears only as a tag.
  - `/posts/msc-thesis` (2023-10-18): slides plus a PDF report. Its section 5.3 is cited by the main
    post for the earlier TCP-over-WASI design.
  - `/posts/keeping-scrapers-out` (2026-10-18): wasm proof-of-work. No RPC or WebSocket content.
- No post on the blog covers capnp over WebSocket or in a browser.

Each claim from the main post, checked against the source:

| Blog claim | Verified against | Applies to vexdom? |
|---|---|---|
| capnp-rust transports can be "any async IO" | `VatNetwork::new<U>(input_stream: T, output_stream: U, …)` with `T: AsyncRead + Unpin`, `U: AsyncWrite + 'static + Unpin` (`capnp-rpc/src/twoparty.rs:229-251`) | Yes. A WebSocket adapter is enough; no custom `VatNetwork` is needed. |
| Wasm guests are single-threaded | wasm32-unknown-unknown: "`std::thread::spawn` will panic" (https://doc.rust-lang.org/rustc/platform-support/wasm32-unknown-unknown.html, intro) | Yes. The `Rc`-based RPC system is the right fit. |
| `poll_read` "returns `Poll::Pending` and self-wakes when no data is ready" | PoC `wasm/src/main.rs:30-38` calls `cx.waker().wake_by_ref()` and then returns `Pending`. It cannot return `Ok(0)`, because `capnp-futures` treats a 0-byte read as EOF (`capnp-futures/src/serialize.rs:78-80`). | The constraint applies; the fix does not. In a browser, store the waker and wake it from `onmessage`. |
| `poll_write` uses `blocking_write_and_flush` "so capnp frames aren't truncated" | PoC `wasm/src/main.rs:58-90`; comment at `:64-66` warns that partial writes deadlock RPC | Partly. `WebSocket.send()` never blocks and only enqueues, so accept the whole buffer and report its full length (https://developer.mozilla.org/en-US/docs/Web/API/WebSocket/send). |
| Thousands of concurrent calls on one thread | PoC runs 10 batches × 1000 echo calls (`wasm/src/main.rs:182-183`) on `futures::executor::LocalPool` (`:173`, `:216`) | Indicative only. That was WASI stdio, not a browser event loop. |
| Host side runs capnp on tokio | Host `src/main.rs:42`: `multi_thread` runtime. The RPC provider gets its own thread with `new_current_thread()` (`:100-145`) and uses `tokio_util::compat` (`:6`, `:126-127`). | Yes, the same pattern suits the daemon. |
| Versions | PoC pins capnp 0.21.5 / capnp-rpc 0.21.0 on wasm32-wasip2 (`Cargo.toml`, `wasm/Cargo.toml`, `wasm/.cargo/config.toml`) | 0.28 changed the API: `async fn` server methods since 0.22/0.23 (`capnp-rpc/CHANGELOG.md`). Don't copy PoC code verbatim. |

## Does it compile for wasm32-unknown-unknown?

### Dependencies and features

- **`capnp` 0.28.2:**
  - The only required dependency is none. `quickcheck` and `embedded-io` are optional (`capnp/Cargo.toml:17-20`).
  - Features: `default = ["std","alloc"]`, `unaligned`, `sync_reader`, `rpc_try` (`capnp/Cargo.toml:25-42`).
  - `std` is optional; the crate supports `no_std` (`:35-37`).
- **`capnp-rpc` 0.28.0** depends on `futures` 0.3 with `default-features = false, features = ["std"]`,
  plus `capnp` and `capnp-futures` (`capnp-rpc/Cargo.toml:17-24`). It has no feature flags of its own.
- **`capnp-futures` 0.28.0** uses `futures-channel` and `futures-util` (`io`, `std`) (`capnp-futures/Cargo.toml:16-27`).
  So capnp-rpc needs `std`, which wasm32-unknown-unknown provides.
- **No getrandom, no time, no threads, no fs, no net.**
  - A grep of `capnp/src`, `capnp-rpc/src` and `capnp-futures/src` for `std::time`, `Instant`,
    `SystemTime`, `std::thread`, `getrandom`, `rand::`, `std::net` and `std::fs` finds nothing.
  - `cargo tree --target wasm32-unknown-unknown -e normal` of the compile check contains no
    `getrandom`, `rand` or `time` crates.
- **`capnpc` is build-time only.**
  - It shells out to the external `capnp` binary: `Command::new("capnp")` (`capnpc/src/lib.rs:242-255`),
    then runs `capnp compile -o -` (`:323`).
  - So `capnp` must be installed on the build host (the plan's `brew install capnp`), or the
    generated code must be committed. capnp-rpc itself commits `rpc_capnp.rs` (`capnp-rpc/src/lib.rs:76-82`).
- **Upstream CI never builds wasm.** `.github/workflows/ci.yml` runs build, test, miri, minrust,
  fmt and clippy on native targets only.
  - The only wasm example, `example/wasm-hello-world`, is `no_std` capnp serialization with no RPC
    (`example/wasm-hello-world/README.md:1,7`; `wasm-app/Cargo.toml:14-17`).
  - So "capnp-rpc on wasm" works but nothing upstream protects it.

### Compile check (done for this note)

Setup: a cdylib with these dependencies:
- `capnp = "0.28"`, `capnp-rpc = "0.28"`, `futures = "0.3"`, `wasm-bindgen = "0.2"`,
  `wasm-bindgen-futures = "0.4"`, `js-sys`, `web-sys` (`WebSocket`, `BinaryType`, `MessageEvent`).

What `connect(url)` does:
- Opens a `web_sys::WebSocket` and sets `binaryType = arraybuffer`.
- Wraps the socket in a reader. Its `poll_read` drains a `VecDeque<Vec<u8>>` that `onmessage` fills,
  stores the waker, and is woken by `onmessage`.
- Wraps it in a writer. `poll_write` appends to a `Vec`; `poll_flush` calls `send_with_u8_array`.
- Builds `twoparty::VatNetwork::new(reader, writer, Side::Client, Default::default())` and
  `RpcSystem::new(Box::new(network), None)`.
- Calls `set_background_error_handler`, then `bootstrap(Side::Server)`, then `spawn_local(rpc)`.

Results:
- `cargo build --release --target wasm32-unknown-unknown` finished with no errors, using capnp
  v0.28.2, capnp-futures v0.28.0 and capnp-rpc v0.28.0.
- `wasm-bindgen 0.2.130 --target web --remove-name-section --remove-producers-section` ran on the
  output. Build profile: `opt-level="z"`, `lto=true`, `codegen-units=1`, `panic="abort"`, no
  `wasm-opt` (not installed).

| Build | `_bg.wasm` raw | gzip -9 | brotli -q 11 | JS glue raw / gzip |
|---|---|---|---|---|
| Baseline: wasm-bindgen + web-sys WebSocket + adapter, no capnp (`--features nocapnp`) | 49,704 B | 20,465 B | 17,473 B | 16,832 / 4,467 B |
| + capnp-rpc twoparty `RpcSystem`, no schema code | 341,196 B | 111,285 B | 92,529 B | 16,960 / 4,496 B |
| **Delta (capnp runtime)** | **~291 KB** | **~91 KB** | **~75 KB** | n/a |

Before wasm-bindgen and with `strip=true`, `opt-level` s is 3% larger than z and 3 is 18% larger
(780,629 / 804,079 / 921,068 B).

**Bundle expectation:**
- 0.35-0.5 MB raw wasm once a real schema's generated code is added. This is an estimate; generated
  code size was not measured because no `capnp` binary is installed.
- Plus whatever `ext-core` logic ADR-015 moves in.
- A CRX/XPI is a zip, so the gzip column approximates the download cost.
- The SW re-instantiates the module on every wake, so cold-start time matters more than bytes
  (see Spike checklist, gate G4).

## Running twoparty over a browser WebSocket

### Two framing choices

1. **Byte-stream adapter (recommended for S1).** Implement `futures::AsyncRead` and `AsyncWrite` on
   top of the WebSocket and let `twoparty` do the framing. Facts the adapter must respect:
   - **Reads:**
     - The reader reads an 8-byte segment table with `read`, then uses `read_exact` for the
       remaining table and segments (`capnp-futures/src/serialize.rs:69-136`). So `poll_read` must
       serve partial chunks across WS messages and keep leftovers.
     - `Ok(0)` means EOF and ends the connection (`:78-80`). Return it only after `onclose`.
   - **Writes:**
     - `write_message` issues several `write_all` calls (segment table, then each segment) and "Does
       not call `flush()`" (`capnp-futures/src/serialize.rs:199-259`).
     - The write queue writes every message already queued, then flushes once (`write_queue.rs:83-110`).
       It holds the message `Rc` until the flush completes (`:119-134`).
     - Buffering in `poll_write` and sending in `poll_flush` therefore yields one WS binary frame
       per flush, carrying 1..N **whole** capnp messages, never a partial one.
     - Sending on every `poll_write` instead (what `gloo-net`'s `io-util` does,
       `gloo-net-0.8.0/src/websocket/io_util.rs:70-87`) splits one message across several frames.
   - **Ready-made adapters exist but don't fit unchanged:**
     - `gloo-net` 0.8.0 (feature `io-util`) implements `AsyncRead`/`AsyncWrite` for its `WebSocket`
       (`io_util.rs:39-96`). It can wrap an existing `web_sys::WebSocket` via `TryFrom`
       (`src/websocket/futures.rs:257-263`), which is exactly the handover we need. But its reader
       turns **text** frames into bytes (`io_util.rs:52-55`), so any JSON control frame left on the
       socket would corrupt the capnp stream.
     - `ws_stream_wasm` 0.7.5 (`WsStream::into_io`, `src/ws_stream.rs:189`) only opens its own socket
       from a URL (`src/ws_meta.rs:48`). Its last release was 2025-06-08.
     - Write our own adapter, about 80 lines, that routes text frames to the JSON handler and binary
       frames to capnp.
2. **Message-oriented `Connection` (stretch goal).** `capnp-rpc` exposes the `VatNetwork`,
   `Connection`, `OutgoingMessage` and `IncomingMessage` traits publicly (`capnp-rpc/src/lib.rs:115-218`).
   `new_stream` has a default implementation (`:178`).
   - A custom connection can map exactly one capnp message to one WS binary frame, using
     `capnp::serialize::write_message_to_words` (`capnp/src/serialize.rs:528`) and
     `read_message_from_flat_slice` (`:53`).
   - Gains: per-frame `vexdom debug decode`, no partial-read state, and `traversal_limit` checks per
     frame.
   - Costs: re-implementing about 320 lines of `twoparty.rs`, and an alignment rule. Segments read
     from an unaligned `Vec<u8>` fail with `UnalignedSegment` unless `capnp` is built with
     `unaligned` (`capnp/src/lib.rs:384`, `capnp/src/serialize.rs:342`).

### Extension (wasm) side

- **Executor:**
  - `wasm_bindgen_futures::spawn_local` re-exports `js_sys::futures::spawn_local`
    (`wasm-bindgen-futures-0.4.80/src/lib.rs:14`; `js-sys-0.3.107/src/futures/mod.rs:107-117`).
  - Tasks run from a queue drained by `queueMicrotask`, falling back to `Promise.then`
    (`js-sys-0.3.107/src/futures/queue.rs:63-70`).
- **Server objects:** they are `Rc<Self>`. Generated server methods take `self: Rc<Self>`
  (`capnp-rpc/src/lib.rs:36-46`; `examples/hello-world/server.rs:31-36`). `new_client` wraps in `Rc`
  (`lib.rs:393-398`). Nothing is `Send`, which is fine on the one JS thread.
- **Error logging:** the default background-error reaper writes to stderr (`capnp-rpc/src/lib.rs:530-539`).
  - On wasm32-unknown-unknown, std's stdio is the `unsupported` sink. `Stderr = Stdout`, and `write`
    returns `Ok(buf.len())` and drops the bytes (`std/src/sys/stdio/mod.rs:48-50`,
    `std/src/sys/stdio/unsupported.rs:5,63-67`).
  - So `set_background_error_handler` (`capnp-rpc/src/lib.rs:303`) is mandatory for any visibility.
    Route it to `console.error`.
- **Bootstrap direction:** both ends can provide a bootstrap capability.
  - `RpcSystem::new(network, Some(cap))` (`capnp-rpc/src/lib.rs:253-293`).
  - `bootstrap(vat_id)` returns the peer's capability whenever `vat_id` is not the local side
    (`lib.rs:310-326`; `twoparty.rs:295-307`).
  - So the daemon can call the extension's capability and vice versa over one socket.
- **Feature level:** capnp-rpc implements Level 1 (`capnp-rpc/src/lib.rs:22`): promise pipelining
  and capability passing between two parties. That is enough for watches as callback capabilities
  (ADR-014).

### Daemon (tokio) side

- **Reference pattern:** `tokio::task::LocalSet::new().run_until(...)`, then per connection
  `tokio_util::compat::TokioAsyncReadCompatExt::compat(stream).split()`, wrap in
  `futures::io::BufReader`/`BufWriter`, build `VatNetwork::new(..., Side::Server, ...)`, and call
  `tokio::task::spawn_local(rpc_system)` (`capnp-rpc/examples/hello-world/server.rs:58-79`).
  - All upstream examples use `#[tokio::main(flavor = "current_thread")]` (`examples/*/main.rs:27`;
    `streaming/main.rs:6`).
  - Their manifest needs only `tokio` features `net`, `rt`, `macros` and `tokio-util` with `compat`
    (`examples/hello-world/Cargo.toml`).
- **What this means for vexdom:**
  - HTTP and WS will be served by axum or tokio-tungstenite on a multi-thread runtime (plan research
    item 2). The upgraded socket is `Send`.
  - Run one dedicated **capnp thread**: a current-thread runtime plus a `LocalSet`, as wetware's host
    does at `src/main.rs:100-145`. Hand it each authenticated WS through an mpsc channel.
  - Bridge frames between the Send WS task and the local adapter with channels.
  - The control socket (CLI/MCP/GPUI over Unix socket or TCP) can use `compat()` plus
    BufReader/BufWriter directly, as in the example.
- **One thread for all connections:** every connection whose capabilities may be handed to another
  connection must live on that same thread. `Client` hooks are `Rc`, and a capability forwarded from
  the extension to a control client is proxied by the daemon vat.
- **Reconnect:**
  - `twoparty::VatNetwork` is single-use: `connect()` after disconnect panics with "tried to reconnect
    a disconnected twoparty vat network" (`capnp-rpc/src/twoparty.rs:304`).
  - Every new WebSocket needs a fresh `VatNetwork` and `RpcSystem`.
  - `auto_reconnect` / `lazy_auto_reconnect` (`capnp-rpc/src/reconnect.rs:318-349`; example
    `examples/reconnect/client.rs:24`) re-dial transparently within one process lifetime.
- **Size limits:**
  - The default `ReaderOptions` traversal limit is 8 Mi words (64 MiB), with nesting depth 64
    (`capnp/src/message.rs:116-120`).
  - A segment-table check rejects larger messages before allocating (`capnp-futures/src/serialize.rs:110-121`).
  - Full-page screenshots as `Data` fit, but confirm with the largest real capture.
  - Streaming methods use a fixed 64 KiB flow-control window (`capnp-rpc/src/flow_control.rs:11`;
    `twoparty.rs:201-206`; `set_window_size` at `:284`).

## Chrome MV3 service worker specifics

- **Lifecycle** (https://developer.chrome.com/docs/extensions/develop/concepts/service-workers/lifecycle, "Idle and shutdown", "Choose a minimum Chrome version"):
  - Termination triggers:
    - "After 30 seconds of inactivity. Receiving an event or calling an extension API resets this timer."
    - A single request taking longer than 5 minutes.
    - A `fetch()` response that takes more than 30 s to arrive.
  - "Any global variables you set will be lost if the service worker shuts down" (section "Persist
    data rather than using global variables"). That includes the wasm instance, every capability
    and the `RpcSystem`.
  - Chrome 116 changes:
    - "Active WebSocket connections now extend extension service worker lifetimes."
    - "Sending or receiving messages across a WebSocket … resets the service worker's idle timer."
  - Chrome 120: alarms can use a minimum period of 30 s.
- **WebSockets in the SW** (https://developer.chrome.com/docs/extensions/how-to/web-platform/websockets):
  - Stay alive "by exchanging messages within the 30s service worker activity window".
  - The sample sends `'keepalive'` every 20 s and recommends `"minimum_chrome_version": "116"`.
  - The page does not say whether protocol-level WS ping frames count; that is unverified and
    becomes gate G6.
  - vexdom already pings every 20 s from the daemon as a JSON text frame (`src/daemon/extension-link.ts:21,75`;
    pong at `packages/extension/src/lib/bridge/socket.ts:673-674`).
  - The built manifest sets no `minimum_chrome_version` (`dist/chrome-mv3/manifest.json`, build output).
- **CSP** (https://developer.chrome.com/docs/extensions/reference/manifest/content-security-policy, "Default policy", "Extension Pages Policy"):
  - The default `extension_pages` is `"script-src 'self'; object-src 'self';"`: "WebAssembly will be
    disabled".
  - The minimum, and the most permissive allowed, is
    `"script-src 'self' 'wasm-unsafe-eval'; object-src 'self';"`. `'unsafe-eval'` is rejected at install.
  - The policy "applies to page and worker contexts in the extension", including the background worker.
  - vexdom's `wxt.config.ts:24-56` sets no `content_security_policy`, so it must be added.
- **Module vs classic SW, and loading the wasm:**
  - Static `import` needs `"background": {"type": "module"}`.
  - "`import()`, often called a dynamic import, is not supported."
  - Classic SWs use `importScripts()`.
  - Source: https://developer.chrome.com/docs/extensions/develop/concepts/service-workers/basics, "Import scripts".
  - WXT bundles the background as an IIFE unless `type: 'module'` (`node_modules/wxt/dist/types.d.mts:593-605`,
    WXT 0.20.27). Today's output is a classic SW: `dist/chrome-mv3/manifest.json` has
    `{"service_worker":"background.js"}`.
- **Top-level await is fatal in a module SW.** The Service Worker spec's Update algorithm rejects the
  job with `TypeError` if "Is Async Module" holds for the script's module graph, that is, if any
  module uses top-level await (https://w3c.github.io/ServiceWorker/, "Update" step and "Is Async
  Module"). This rules out `wasm-bindgen --target bundler` plus vite-plugin-wasm-style TLA imports.
- **Initialise from listeners.** Listeners must be registered "at the top level of the script" and
  "synchronously on initial script execution"
  (https://developer.chrome.com/docs/extensions/develop/concepts/service-workers/events, "Declare
  extension events"). So register listeners synchronously, start `init()` without awaiting at top
  level, and have each handler await the same init promise.
- **wasm-bindgen's web loader:**
  - `--target web` emits `module_or_path = new URL('<name>_bg.wasm', import.meta.url)`, unless
    `--omit-default-module-path` is passed (`wasm-bindgen-cli-support-0.2.130/src/js/mod.rs:1951-1960`).
  - It then uses `WebAssembly.instantiateStreaming`, falling back to `instantiate` with a warning when
    the response is not `application/wasm` (`:1562-1571`).
  - Recommended: pass `--omit-default-module-path`, put the `.wasm` under WXT's `publicDir`
    (`wxt.config.ts:18`), and call `init({ module_or_path: browser.runtime.getURL('/…_bg.wasm') })`.
    This avoids `import.meta.url` in an IIFE bundle and needs no dynamic import.
  - The compile check's glue contains no `eval` or `new Function`. A grep of the generated
    `wasmcheck.js` for `eval|new Function|import(` found only `instantiateStreaming` and the
    `import.meta.url` default path.
- **MIME type and fetch time:**
  - Chrome serves extension files with `GetWellKnownMimeTypeFromFile` (`extensions/browser/extension_protocols.cc:512-515`,
    chromium `677d0dbfa4`), whose primary table maps `wasm` to `application/wasm`
    (`net/base/mime_util.cc:169-176`, chromium `fd254cf70d`). So `instantiateStreaming` on a
    `chrome-extension://` URL takes the fast path.
  - The 30 s fetch rule above is irrelevant for a local file.
- **Content scripts are out of scope.** ADR-015 already keeps wasm out of them (page CSP).

## Firefox MV2 background page

- vexdom builds Firefox as MV2 with `"background": {"scripts": ["background.js"]}` and no
  `persistent` key (`dist/firefox-mv2/manifest.json`, build output; `strict_min_version: '140.0'`
  in `wxt.config.ts:50`).
- In MV2, `persistent` defaults to `true` (https://developer.mozilla.org/en-US/docs/Mozilla/Add-ons/WebExtensions/manifest.json/background, "persistent").
  The page stays loaded until the extension is disabled, so there is no 30 s kill. The socket can
  still drop on daemon restart.
- Firefox does not support `background.service_worker` (same page, bug 1573659). The background is a
  document, so `fetch(moz-extension://…)` plus `instantiateStreaming` applies.
  - The `application/wasm` MIME type for `moz-extension://` is unverified. The glue falls back to
    `instantiate` either way.
- CSP: "Manifest V2 extensions in Firefox can use WebAssembly without the use of `'wasm-unsafe-eval'`.
  However, this behavior isn't guaranteed. See Firefox bug 1770909"
  (https://developer.mozilla.org/en-US/docs/Mozilla/Add-ons/WebExtensions/manifest.json/content_security_policy).
  Declare the MV2 string form `"script-src 'self' 'wasm-unsafe-eval'; object-src 'self';"`.

## HMAC handshake before capnp bootstrap, same socket

Today's flow, all JSON text frames:
1. The extension opens `ws://127.0.0.1:<port>/extension` (`packages/extension/src/lib/bridge/socket.ts:543`).
2. It sends `hello` (`:579-590`).
3. The daemon replies `challenge {nonce}` (`src/daemon/daemon.ts:329-366`).
4. The extension sends `prove {proof}` (`socket.ts:676-685`).
5. The daemon replies `welcome {…, protocolVersion, proof}` (`daemon.ts:452,569`).
6. The extension verifies `serverProof` and opens the sealed session key (`socket.ts:691-707`).
7. After that the link carries JSON frames parsed with `String(event.data)` / `String(raw)`
   (`socket.ts:592-594`; `extension-link.ts:69`).

The proof covers a transcript that includes the client's `protocolVersion` (`packages/extension/src/lib/actions/handshake.ts:93-111`).
The welcome claims MAC'd by `serverProof` are only `daemonVersion`, `manifestHash`, `manifestInSync`
and `sealedSessionKey` (`handshake.ts:57-65`).

Proposed handover (for S1):
1. **Negotiate in the hello.** The extension offers capnp, either by bumping
   `SOCKET_PROTOCOL_VERSION` (`packages/extension/src/lib/actions/protocol.ts:17`, currently 26) or with a new `hello`
   field. The protocol version is already inside the MAC'd transcript.
2. **Bind the daemon's answer.** The daemon's choice in `welcome` (for example `transport: 'capnp'`)
   is **not** MAC'd today. If downgrade matters, add it to the `claims` join in `serverProof`
   (`handshake.ts:58-63`). Both TS and Rust must change byte-identically (plan research item 3).
3. **Binary means capnp, text means JSON.** WebSocket messages carry an opcode per message: 0x1
   text, 0x2 binary (RFC 6455 §5.6, https://www.rfc-editor.org/rfc/rfc6455#section-5.6).
   - Keep `ping`/`pong`/`unauthorized` as JSON text frames for liveness, which also feeds Chrome's
     idle timer. Every capnp message travels as a binary frame. No sniffing is needed.
   - Set `ws.binaryType = 'arraybuffer'` at socket creation, not at handover.
4. **The extension speaks first.** The daemon starts its `RpcSystem` and sends nothing binary until
   it receives the extension's first binary frame, a `Bootstrap` or a call.
   - This removes the race between the async `welcome` verification (`socket.ts:691-697` awaits
     WebCrypto) and early daemon frames.
   - If the daemon must hold the extension's capability, the extension passes it in its first call
     (for example `attach(browser :Browser)`), or the daemon calls `bootstrap(Side::Client)` only
     after that first frame (`capnp-rpc/src/lib.rs:310-326`).
5. **Hand the socket to wasm.** Once the welcome is verified, TS passes the `WebSocket` object to
   wasm, either through `gloo_net::websocket::futures::WebSocket::try_from` with its text frames
   filtered, or our own adapter.
   - From then on, TS's `onmessage` handles only text frames.
   - `onclose` must mark the adapter closed and wake the reader, so capnp sees EOF
     (`serialize.rs:78-80`). Otherwise calls hang: `send()` on a closed socket "silently discard[s]
     the data" (MDN `WebSocket.send`).
6. **The roles stay as today.** The `/extension` path versus control socket and the hello credential
   select which bootstrap interface the daemon offers (ADR-014: one bootstrap interface, role
   decided by the handshake).

## Semantic traps

1. **Never copy the PoC's self-waking `poll_read` into a browser.**
   - `cx.waker().wake_by_ref(); return Poll::Pending` (wetware `wasm/src/main.rs:35-38`) re-queues the
     task through `queueMicrotask` (`js-sys/src/futures/queue.rs:63-70`).
   - Microtasks queued while the queue drains "execute before the next task is run". MDN warns of
     "the event loop endlessly processing microtasks"
     (https://developer.mozilla.org/en-US/docs/Web/API/HTML_DOM_API/Microtask_guide, "Tasks vs. microtasks").
   - The `message` event is a task, so it would never fire and the SW would hang. Store the waker
     and wake it from `onmessage`/`onclose`.
2. **`Ok(0)` from `poll_read` is EOF and kills the connection** (`capnp-futures/src/serialize.rs:78-80`).
   Return `Pending` when nothing is buffered, and `Ok(0)` only after close.
3. **The WS frame is not the capnp message.**
   - A flush can carry N messages (`write_queue.rs:83-110`). A per-`poll_write` sender splits one
     message over frames (`serialize.rs:199-259`).
   - Debug tooling and any proxy (the plan's front-door WS pass-through risk) must treat the binary
     stream as a byte stream, unless the message-oriented `Connection` is built.
4. **Text frames inside the capnp stream corrupt it.** `gloo-net` `io-util` maps `Text` to bytes
   (`io_util.rs:52-55`). The existing daemon ping is a text frame (`extension-link.ts:75`).
5. **Default `binaryType` is `blob`.** gloo-net's comment explains why it forces `arraybuffer`:
   converting a Blob needs a promise and breaks ordering (`gloo-net/src/websocket/futures.rs:126-130`).
   vexdom's `String(event.data)` (`socket.ts:593`) would turn binary frames into `"[object ArrayBuffer]"`.
6. **`std::time::Instant::now()` / `SystemTime::now()` panic on wasm32-unknown-unknown**
   (`std/src/sys/time/mod.rs:39-40` selects `unsupported`; `std/src/sys/time/unsupported.rs:12-13,34-35`).
   - capnp-rpc doesn't call them, but `ext-core` code or a timeout helper might.
   - Panics abort the instance: the target defaults to `-Cpanic=abort`
     (https://doc.rust-lang.org/rustc/platform-support/wasm32-unknown-unknown.html, "Unwinding").
   - After an abort, every later call into that instance fails until it is re-instantiated.
   - capnp-rpc has reachable `panic!`/`unreachable!` paths (`twoparty.rs:120,304`).
7. **capnp-rpc has no call timeouts.** No timer appears in its sources (grep above). A call to an
   unresponsive peer stays pending until disconnect. Timeouts are the application's job: JS
   `setTimeout` via a future in wasm, `tokio::time::timeout` on the daemon.
8. **Silent errors in wasm.** The default background-error reaper writes to a stderr that drops
   bytes (`capnp-rpc/src/lib.rs:530-539`; `std/src/sys/stdio/unsupported.rs:63-67`). Install
   `set_background_error_handler` before spawning (`lib.rs:295-306`).
9. **A SW restart destroys every capability.**
   - Globals are lost on shutdown (Chrome lifecycle doc).
   - The daemon sees `Disconnected` on every capability the extension exported: watch callbacks and
     the browser capability.
   - On reconnect, the extension must re-attach and re-register its callbacks. Daemon state must be
     keyed by install id, not by capability identity.
   - `auto_reconnect` (`reconnect.rs:318`) cannot survive a SW restart, because its closure lives in
     the dead instance. It only helps against daemon restarts while the SW lives.
10. **`twoparty::VatNetwork` is one-shot.** `connect()` after disconnect panics (`twoparty.rs:304`).
    Build a new network and `RpcSystem` per socket.
11. **`LocalSet::run_until` cannot run inside a `tokio::spawn` task** (tokio 1.54.1 docs, `run_until`).
    axum handlers are such tasks, so use a dedicated capnp thread.
12. **Keep all connections that pass capabilities to each other on that one thread.** `Rc` clients
    cannot cross threads (`capnp-rpc/src/lib.rs:231-233`, `:393-401`).
13. **Idle WebSockets do not keep the SW alive.** Only message traffic resets the timer (Chrome
    lifecycle and WebSocket docs). Whether a WS-protocol ping counts is unverified, so keep an
    application-level frame every 20 s or less.
14. **No top-level await anywhere in the SW module graph** (Service Worker spec, "Is Async Module").
    This includes any wasm loader or bundler plugin that injects it.
15. **The default MV3 CSP disables wasm.** A Chrome build with no CSP key fails at `instantiate`
    (Chrome CSP doc, "Default policy").
16. **The welcome race.** Daemon binary frames that arrive while TS is still awaiting `serverProof`
    (`socket.ts:691-697`) reach the JSON handler. Fix this with "extension speaks first", or by
    buffering binary frames from the socket's creation.
17. **A transport choice in `welcome` is not authenticated today** (`handshake.ts:57-65`). Changing
    the claims changes bytes that TS and Rust both compute.
18. **Capnp unpacked encoding can be larger than JSON** for text-heavy data, because of 8-byte words
    and pointers. twoparty uses unpacked `capnp_futures::serialize` (`twoparty.rs:184`), not
    `serialize_packed`. This is an unverified expectation; measure with the AX fixture (gate G5).
19. **Messages over 64 MiB are rejected** by the default traversal limit (`message.rs:116-120`;
    `serialize.rs:110-121`). Size `ReaderOptions` on both ends.
20. **`capnpc` needs the `capnp` binary at build time** (`capnpc/src/lib.rs:242-255`). Either CI and
    every contributor install it, or the generated `.rs` is committed (the plan needs `capnp` for
    `vexdom debug decode` anyway).
21. **Upstream doesn't test wasm** (`.github/workflows/ci.yml`). Releases come fast: capnp-rpc
    shipped 0.21.0 (2025-04-08) through 0.28.0 (2026-11-15), with 0.27.x patch releases about weekly
    in October and November 2026 (crates.io versions API). Pin exact versions in `spikes/` and
    in the product.
22. **The binding interacts with S2.**
    - BoltFFI async exports require `Send` futures (boltffi `03291fda`,
      `boltffi_core/src/runtime/future.rs:110-113,163-171`; `runtime/typescript/README.md:66-68`).
    - capnp-rpc's `RpcSystem` and clients are `Rc`. So an `ext-core` async export cannot await a
      capnp call directly; it needs a `spawn_local` + oneshot bridge.
    - capnp-in-wasm therefore implies `wasm-bindgen-futures` in the module whichever binding wins.
      See `research/boltffi.md`, §4 and trap 1.

## Spike checklist (S1)

Work in `spikes/s1-capnp/`, which is throwaway. Prerequisites: `rustup target add
wasm32-unknown-unknown`, `cargo install wasm-bindgen-cli --version <matching>`, `brew install capnp`.

1. **Schema.** Write `link.capnp` with:
   - `interface Daemon { attach @0 (browser :Browser) -> (session :Session); }`
   - `interface Browser { invoke @0 (action :Text, args :Data) -> (result :Data); watch @1 (cb :Watcher) -> (); }`
   - `interface Watcher { event @0 (e :Data) -> (); }`
   - A `Snapshot` struct mirroring an AX node.
   Generate it with `capnpc` in `build.rs`.
2. **Wasm crate.**
   - Pin `capnp`, `capnp-rpc`, `wasm-bindgen` and `wasm-bindgen-futures` exactly.
   - Write the adapter from "Framing choice 1": text frames go to a JS callback, binary frames to
     capnp; waker stored; `onclose` sets EOF.
   - Install `set_background_error_handler` writing to `console.error`.
   - Export `start(ws: WebSocket, browserImpl: JsValue)`.
   - Build with `wasm-bindgen --target web --omit-default-module-path`.
   - **G1 (pass/fail):** `cargo build --target wasm32-unknown-unknown --release` is clean and
     `cargo tree -e normal` shows no `getrandom` or `time` crates.
3. **Daemon stub (tokio).**
   - axum or tokio-tungstenite on a multi-thread runtime, implementing the existing JSON
     hello/challenge/prove/welcome by porting `handshake.ts` or using a fixed test key.
   - After the welcome, send the socket over an mpsc channel to a capnp thread (current-thread
     runtime plus `LocalSet`).
   - Start `RpcSystem(Side::Server, Some(daemon))` only after the first binary frame.
   - Keep a 20 s JSON `ping` text frame.
   - **G2:** two extensions connect, each gets its own `RpcSystem`, and dropping one leaves the
     other working.
4. **Chrome MV3 load.**
   - In a scratch WXT build of the spike extension, set `background.type: 'module'` and
     `content_security_policy.extension_pages = "script-src 'self' 'wasm-unsafe-eval'; object-src 'self';"`
     and `minimum_chrome_version: "116"`.
   - Fetch the wasm via `browser.runtime.getURL`.
   - **G3:** the SW registers with no `TypeError` (no top-level await), `instantiateStreaming`
     succeeds with no MIME warning, and `chrome://extensions` shows no CSP error.
   - **G4:** cold start, from the SW event to a resolved `attach()`, is at most 150 ms on a dev
     laptop. Record median and p95 over 10 SW restarts.
5. **Firefox MV2 load.**
   - Same bundle with `content_security_policy: "script-src 'self' 'wasm-unsafe-eval'; object-src 'self';"`.
   - Run `yarn lint:firefox` on the spike output.
   - **G3-FF:** the background page instantiates and attaches, and `web-ext lint` reports no new
     errors.
6. **Round trip and payload.**
   - Time 1000 sequential `invoke("noop")` calls, plus a pipelined `attach().session.x()`, against
     the same daemon speaking JSON v26.
   - Encode a ~2k-node AX snapshot fixture in three forms: capnp unpacked, capnp packed, and the
     JSON v26 form.
   - **G5:** capnp p50 latency is within JSON p50 + 0.5 ms, and capnp unpacked bytes are at most
     1.5× JSON bytes. Record packed and encode/decode times as well; there is no gate on those.
   - Send a 10 MB `Data` screenshot. It must arrive intact, faster than base64 JSON.
7. **Keepalive and SW kill.**
   - Leave the link idle for 5 minutes with only daemon text pings. **G6:** the SW stays alive, as
     shown in `chrome://serviceworker-internals`.
   - Repeat with the JSON pings replaced by WS-protocol pings only. Record the result to settle the
     open question.
   - Stop the SW in `chrome://serviceworker-internals`, then trigger an event (alarm
     `vexdom/reconnect`, `packages/extension/src/lib/bridge/socket.ts:42`).
   - **G7:** the daemon sees `Disconnected` for in-flight calls within 1 s of the socket closing.
     Its live-connection count returns to the right number with no leaked `RpcSystem`. After the
     wake, a new socket handshakes, re-attaches, re-registers `watch`, and serves the first `invoke`
     within 2 s.
8. **Failure paths.**
   - Kill the daemon mid-call: the wasm side gets `Disconnected` rather than a hang.
   - Send a message over 64 MiB: it is rejected cleanly.
   - Trigger a deliberate Rust panic: record what the TS layer sees, then re-instantiate.
   - **G8:** no hang in any case.
9. **Size.**
   - **G9:** `_bg.wasm` with the schema is at most 600 KB raw and at most 200 KB gzip, before
     `wasm-opt`. Report `wasm-opt -Oz` numbers if binaryen is available.
10. **Write the results into ADR-014:** numbers for G4/G5/G9, the answer to G6, and go or no-go.

## Recommendation

**Go into S1 as planned. The compile and size risks are retired; the remaining risks are lifecycle
and handover.** The design to spike:
- An in-house byte-stream adapter on both ends: binary frames for capnp, text frames for JSON control.
- Handover after a verified `welcome`, with the extension speaking first.
- A dedicated current-thread runtime plus `LocalSet` thread in the daemon for every capnp connection.
- `--target web --omit-default-module-path`, loaded from `runtime.getURL` in an ES-module SW with
  `'wasm-unsafe-eval'`, `minimum_chrome_version` 116, and the 20 s keepalive kept.
- The message-oriented `Connection` is a stretch goal. Build it only if `vexdom debug decode` or the
  front-door proxy needs one message per frame.

If the wasm path fails G3, G4 or G7 in the SW, don't drop Cap'n Proto. Try `capnp-ts` (beta,
level-1 RPC, WebSocket transport) on the TypeScript side before falling back to JSON. Amend
ADR-014's context line, which says JS Cap'n Proto has no RPC.
