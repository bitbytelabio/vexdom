# Spike results: S1 (capnp over WebSocket in wasm), S2 (BoltFFI), S2b (capnp behind BoltFFI)

Run 2026-11-25 on macOS arm64, rustc 1.100.0, capnp/capnp-rpc 0.28, wasm-bindgen 0.2.130,
BoltFFI 0.32.1 (+ wasm-opt 133), Chrome for Testing 158.0.8127.88 headless, Firefox 161 headless
(web-ext 10.6). Code: `spikes/capnp-ws`, `spikes/boltffi-wasm`, `spikes/capnp-bolt`.
Background research: [capnp-wasm.md](capnp-wasm.md), [boltffi.md](boltffi.md), [capnweb.md](capnweb.md).

## What was measured

The spike server (tokio, `LocalSet`, capnp-rpc two-party over tokio-tungstenite through a byte-stream
adapter) bootstraps `Bridge`; the extension attaches a `Page` it serves from wasm, so every
measured call goes daemon → extension over the same socket (the real direction of page actions).
20 rounds each; times are per call, wall clock on the server, loopback.

| Client | wasm raw / gzip | init | echo | 2,000 nodes capnp (bytes) | 2,000 nodes JSON text (bytes) | 1 MB `Data` |
| --- | --- | --- | --- | --- | --- | --- |
| native Rust (control) | — | — | 0.15 ms | 0.92 ms (293,200) | 0.64 ms (260,774) | 1.08 ms |
| S1 Chrome MV3 SW, wasm-bindgen + ws_stream_wasm | 331 KB / 123 KB | 9.1 ms | 1.76 ms | 1.35 ms | 0.96 ms | 1.34 ms |
| S1 Firefox MV2 background, same | 331 KB / 123 KB | 21 ms | 2.19 ms | 1.22 ms | 4.23 ms | 2.55 ms |
| S2b Chrome MV3 SW, BoltFFI + JS-owned socket + pump | 282 KB / 115 KB | 11.7 ms | 2.36 ms | 1.08 ms | 0.88 ms | 1.73 ms |
| S2b Firefox MV2 background, same | 282 KB / 115 KB | 3.0 ms | 2.01 ms | 2.85 ms | 4.26 ms | 2.15 ms |

The 200-node rows (not shown) are dominated by JIT warm-up in Firefox (5 ms first rounds).

S2 (BoltFFI alone, no capnp; 41.5 KB wasm + 77 KB unminified JS bundle incl. `@boltffi/runtime`),
in both browsers: strings, `Vec<u8>` both ways (1 MB in 2.3–2.7 ms Chrome), 2,000 nested `#[data]`
structs out (4.2 ms Chrome), struct in / enum out, `Result` → `CoreErrorException`, a callback trait
implemented by a plain TS object, `async` → `Promise`, and `AbortSignal` → `BoltFFICancelledError`
all worked. No `eval`, `new Function` or dynamic `import()` in the generated glue or the bundle.

## Gates

| Gate | Result |
| --- | --- |
| capnp-rpc builds for `wasm32-unknown-unknown` | pass (only `futures` underneath) |
| Bidirectional capabilities over one WebSocket (daemon calls a capability the extension passed) | pass, native, Chrome SW, Firefox |
| Runs in a Chrome MV3 module service worker with `'wasm-unsafe-eval'` | pass — **only without top-level `await`**: with it, the worker never registered and nothing ran |
| Runs in a Firefox MV2 background page | pass |
| Glue is CSP-clean (no eval) | pass, wasm-bindgen and BoltFFI |
| Daemon notices a vanished browser | pass: `RpcSystem` ended ~5 s after `kill -9` of Chrome |
| Service worker kept alive by an idle socket | not reproduced: no idle termination in 70 s headless; keep the 20 s keepalive anyway (capnp-wasm.md) |
| BoltFFI can carry capnp-rpc | pass **only** with the pump pattern: BoltFFI's exported futures must be `Send` and capnp-rpc's are not, so S2b exports synchronous `link_open` / `link_feed` / `link_pump` / `link_close` and runs the RPC system on a thread-local `LocalPool` |
| Payload size | capnp (unpacked, as the two-party transport sends it) is ~12% larger than the same nodes as JSON text; irrelevant on loopback, and the LLM never sees it |

## Findings to carry into the design

1. Module service workers must not use top-level `await` (silent failure).
2. JavaScript should own the WebSocket: it already owns the service worker lifecycle and reconnects;
   wasm only sees bytes (`link_feed`) and returns bytes (`link_pump`). This also keeps wasm-bindgen
   out of the module, so BoltFFI is the only binding (ADR-017).
3. Treat the socket as a byte stream for capnp: frame boundaries do not align with messages
   (capnp-wasm.md). Binary frames carry capnp, text frames stay JSON (handshake, keepalive).
4. BoltFFI pitfalls met: `boltffi pack wasm` needs `wasm-opt` and `tsc` on `PATH` and
   `@boltffi/runtime` resolvable next to the crate (a package.json in the crate dir, or `bun add`
   walks up to the repository root — it did once, and was reverted). `[targets.wasm.npm]
   package_name` is required. Its `web.js` fetches the wasm on import; in the extension, call
   `init()` from code instead (boltffi.md).
5. The daemon side needs a dedicated current-thread runtime + `LocalSet` for capnp (servers are `Rc`).
6. Cap'n Web: no usable Rust implementation (capnweb.md); not pursued.
