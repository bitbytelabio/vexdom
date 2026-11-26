# Research: the MCP crate for `vexdom mcp`

Topic `mcp-crate` (MAP.md). Question: what replaces `@modelcontextprotocol/sdk` 1.29.0 for the stdio
MCP server, so that the faithful port meets ADR-008 (same key order, error codes and error messages).
Researched 2026-11-25.

Sources. TS SDK: the installed package, `src/daemon/node_modules/@modelcontextprotocol/sdk/dist/esm/`
(version 1.29.0, `src/daemon/node_modules/@modelcontextprotocol/sdk/package.json`). rmcp: the 3.11.0
crate source as published (`https://docs.rs/crate/rmcp/3.11.0/source/<path>`; repository
`https://github.com/modelcontextprotocol/rust-sdk`). MCP spec: `https://modelcontextprotocol.io/specification/…`.
agent-browser: `../agent-browser/cli/src/mcp.rs` at v0.45.0 `f0a9835`.

Wire claims marked **observed** come from two probes run on 2026-11-25 with the same 21-line stdin
script: a TS server built from SDK 1.29.0 with this repo's capabilities and handler shapes (node 26.13.1),
and an rmcp 3.11.0 `ServerHandler` with the same handlers (`default-features = false,
features = ["server","transport-io"]`, built with cargo 1.99.1). The relevant output lines are quoted
below. The probes lived in the session scratchpad and are not part of the repo.

## 1. What the TS server uses

| Use | Where |
| --- | --- |
| Low-level `Server` (not `McpServer`), `{name:'vexdom', version}` | `src/daemon/server.ts:1,44-45` |
| Capabilities `{ tools: { listChanged: true }, resources: {} }` + `instructions` | `src/daemon/server.ts:47-60` |
| `tools/list` → dynamic list from the bridge each time | `src/daemon/server.ts:67`, `src/daemon/tool-host.ts:210-227` |
| `tools/call` → `tools.call(name, args ?? {}, extra.signal)` (abort on cancel) | `src/daemon/server.ts:69-71`, `src/daemon/tool-host.ts:154-155` |
| `resources/list` (3 static), `resources/read` (`throw new Error('Unknown resource: …')`) | `src/daemon/server.ts:16-35,73-96` |
| `notifications/tools/list_changed` on every manifest change | `src/daemon/server.ts:98-101` |
| Results: `text` and `image` blocks, `isError: true` on failures and on `RESULT_TOO_LARGE` | `src/daemon/tool-host.ts:140-145,251-268,336-368,440-446` |
| stdio transport; exit on stdin `end`/`close`, SIGINT, SIGTERM | `src/daemon/cli.ts:130-132,232-253` |

No prompts, logging, completions, tasks, sampling, elicitation or `structuredContent`.

### SDK 1.29.0 behavior the port has to reproduce

- Protocol versions: `LATEST_PROTOCOL_VERSION = '2025-11-25'`, `SUPPORTED_PROTOCOL_VERSIONS =
  ['2025-11-25','2025-06-18','2025-03-26','2024-11-05','2024-10-07']` (`types.js:2-4`). `initialize`
  echoes a supported version, otherwise answers `2025-11-25` (`server/index.js:270-281`). This follows the
  2025-11-25 rule "If the server supports the requested protocol version, it MUST respond with the same
  version. Otherwise … SHOULD be the latest version supported by the server" (spec
  `2025-11-25/basic/lifecycle#version-negotiation`).
- There is no init gate: `tools/list` before `initialize` is served (**observed**, id 0 below).
  Handlers are registered by method (`shared/protocol.js:284-285`).
- Framing: one `JSON.stringify(message) + '\n'` per message (`shared/stdio.js:28-30`). Input is split on
  `\n` with a trailing `\r` dropped, then `JSONRPCMessageSchema.parse` (`shared/stdio.js:13-27`).
- Envelope key order:
  - success `{"result":…,"jsonrpc":"2.0","id":…}` (`shared/protocol.js:373-377`)
  - error `{"jsonrpc":"2.0","id":…,"error":{code,message[,data]}}` (`shared/protocol.js:394-402`)
  - notification `{"method":…,"jsonrpc":"2.0"}`: `{...notification, jsonrpc}` with no `params` key
    (`shared/protocol.js:862-865`).
- An unknown method (any method without a handler, e.g. `prompts/list`, `resources/templates/list`,
  `logging/setLevel`, `server/discover`) gets `-32601` with the message `"Method not found"`
  (`shared/protocol.js:286-299`).
- A handler that throws gets `code` if it is a safe integer, otherwise `-32603`, with
  `message = error.message` (`shared/protocol.js:394-402`). `Unknown resource: x` is therefore `-32603`
  (**observed**). An `McpError` prefixes `MCP error <code>: ` (`types.js:2029-2031`).
- The `tools/call` result is re-parsed through zod `CallToolResultSchema` (`server/index.js:117-146`), and
  this rewrites the result.
  - Key order becomes the schema order `content, structuredContent?, isError?`: `{isError:true,content}`
    goes out as `{"content":[…],"isError":true}` (**observed**). Content blocks are `z.object`, so unknown
    keys in a block are stripped (`types.js:1015-1056`).
  - Image `data` must pass `atob()` (`types.js:750-760`). If it fails, the reply is `-32602 "MCP error
    -32602: Invalid tools/call result: [zod issues JSON]"` (**observed**).
- If the request fails to parse, the zod error has no numeric code, so the reply is `-32603` with the zod
  issue list as message. For example, `tools/call` with `params:{}` gives `{"code":-32603,"message":"[\n {\n
  \"expected\": \"string\", …"}` (**observed**). This comes from `parseWithCompat` in
  `shared/protocol.js:886-893`.
- Malformed input gets no reply at all: non-JSON, `"jsonrpc":"1.0"`, or a missing `jsonrpc` only reach
  `onerror` (`server/stdio.js:34-46`, **observed**). Unknown notifications are ignored
  (`shared/protocol.js:273-279`).
- Cancellation: `notifications/cancelled` aborts that handler's `AbortController`
  (`shared/protocol.js:169-176`), and an aborted request gets no response (`shared/protocol.js:369-372,390-392`).
- Each request runs as its own promise, so responses go out in completion order. The **observed** output
  had ids `8,9,10,…,0,1,2` for input `0,1,2,…`.
- `ping` gets the automatic answer `{}` (`shared/protocol.js:33-35`).
- Shutdown: the CLI calls `process.exit(0)` and does not wait for in-flight replies
  (`src/daemon/cli.ts:241-252`).

**Observed TS 1.29 lines** (selected):
```
{"result":{"protocolVersion":"2025-06-18","capabilities":{"tools":{"listChanged":true},"resources":{}},"serverInfo":{"name":"vexdom","version":"0.9.2"},"instructions":"hi"},"jsonrpc":"2.0","id":1}
{"result":{"content":[{"type":"text","text":"{\"k\":1}"}]},"jsonrpc":"2.0","id":3}
{"result":{"content":[{"type":"text","text":"X: y"}],"isError":true},"jsonrpc":"2.0","id":4}
{"result":{"content":[{"type":"image","data":"AAAA","mimeType":"image/png"},{"type":"text","text":"n"}]},"jsonrpc":"2.0","id":5}
{"jsonrpc":"2.0","id":8,"error":{"code":-32601,"message":"Method not found"}}
{"jsonrpc":"2.0","id":"s","error":{"code":-32601,"message":"Method not found"}}      <- server/discover
{"method":"notifications/tools/list_changed","jsonrpc":"2.0"}
```

## 2. rmcp, the official Rust SDK

- Version: latest is **3.11.0**, published 2026-11-17 (8 days ago). Earlier releases: 3.10.0 on
  2026-11-12 and 3.9.0 on 2026-11-05 (crates.io API `https://crates.io/api/v1/crates/rmcp`).
- License Apache-2.0, edition 2024, MSRV `rust-version = "1.88"`
  (`https://docs.rs/crate/rmcp/3.11.0/source/Cargo.toml` L13-29). ADR-010 already requires stable
  Rust and edition 2024.
- Features: the default set is `base64, macros, server`. `server` = `transport-async-rw, schemars, pastey,
  uuid`. `transport-io` = `transport-async-rw, tokio/io-std, tokio/net`
  (`https://docs.rs/crate/rmcp/3.11.0/features`; `Cargo.toml.orig` L144-149, L196-197).
- Needed: `default-features = false, features = ["server", "transport-io"]`. Macros are not needed
  because the tool list is dynamic. `schemars` still comes in through `server`.
- `build.rs` only sets git hooks when it is built inside its own workspace (`build.rs` L1-22).
- stdio: `transport::stdio()` returns tokio `Stdin`/`Stdout`, and every read runs on the blocking pool. "A
  pending read on Stdin cannot be cancelled … runtime shutdown waiting for stdin input or EOF"
  (`src/transport/io.rs` L9-22). `stdio_pipes()` is non-blocking on Linux only and falls back elsewhere
  (`src/transport/io.rs` L24-64).
- Line cap `DEFAULT_MAX_LINE_LENGTH = 16 MiB`. A longer line is dropped with a log entry
  (`src/transport/async_rw.rs` L58, L111-119). The TS SDK has no cap.

### Protocol versions rmcp speaks vs SDK 1.29

- rmcp `KNOWN_VERSIONS = 2024-11-05, 2025-03-26, 2025-06-18, 2025-11-25, 2026-07-28`,
  `LATEST = 2026-07-28`, `LATEST_WITH_INITIALIZE = 2025-11-25` (`src/model.rs` L168-210). It has no
  `2024-10-07`.
- The MCP spec's current revision is **2026-07-28**: "Every request declares the protocol version it is
  using via the `io.modelcontextprotocol/protocolVersion` key in its `_meta`". Clients that want to pick a
  version up front can call `server/discover` ("a mandatory RPC")
  (`https://modelcontextprotocol.io/specification/versioning`). SDK 1.29 only knows the handshake eras.
- rmcp is dual-era. Before `initialize` it answers `ping`. It answers a `server/discover` with full `_meta`
  with a `DiscoverResult`. A request carrying modern `_meta` makes the session stateless-modern. Any other
  request without `_meta` gets `-32602 "request _meta is missing or has malformed required fields: …"`
  (`src/service/server.rs` L568-678). If the first message is not a request, serving fails with
  `ExpectedInitializeRequest` (L673-677).
- `initialize` negotiation: echo the requested version if it has a handshake and is in
  `supported_protocol_versions()`. Otherwise fall back to the newest handshake version
  (`src/service/server.rs` L478-509). For `2025-11-25`/`2025-06-18`/`2025-03-26`/`2024-11-05` the
  result equals SDK 1.29. For `2026-07-28` and for unknown versions both answer `2025-11-25`. For
  `2024-10-07`, TS echoes it and rmcp answers `2025-11-25` (**observed** on both).
- Era detection on stdio: a dual-era client "SHOULD probe with `server/discover` … returns a
  `DiscoverResult`: the server is modern … returns any other error …: the server is legacy. Fall back to
  the `initialize` handshake". A recognized modern error such as `UnsupportedProtocolVersionError`
  (`-32022`) means "Do **not** fall back to `initialize`"
  (`https://modelcontextprotocol.io/specification/2026-07-28/basic/transports/stdio#backward-compatibility`).
- Result: SDK 1.29 answers the probe `-32601`, so the client falls back to legacy. Default rmcp answers
  with a `DiscoverResult`, so the client goes modern (**observed**):
  `{"jsonrpc":"2.0","id":1,"result":{"resultType":"complete","supportedVersions":[…,"2026-07-28"],…,"ttlMs":0,"cacheScope":"private","_meta":{"io.modelcontextprotocol/serverInfo":{…}}}}`.
  After that, `tools/list` and `tools/call` replies carry `resultType`, `ttlMs` and `cacheScope`
  (**observed**; `src/handler/server.rs` L307-335).
- Narrowing `supported_protocol_versions()` to the legacy versions does not fix this. The probe then gets
  `-32022` (`src/handler/server.rs` L95-103), which per the stdio binding marks a *modern* server and
  forbids the fallback. To stay legacy, `server/discover` must be answered `-32601`.

### Writing the low-level server in rmcp (abridged from the probe that compiled and ran on 3.11.0)

```rust
// rmcp = { version = "=3.11.0", default-features = false, features = ["server", "transport-io"] }
use std::sync::Arc;
use rmcp::{ErrorData as McpError, RoleServer, ServerHandler, ServiceExt, model::*,
           service::RequestContext, transport::stdio};

struct Vexdom { /* RemoteBridge, ToolHost, … */ }

impl ServerHandler for Vexdom {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools()
                .enable_tool_list_changed().enable_resources().build())
            .with_server_info(Implementation::new("vexdom", env!("CARGO_PKG_VERSION"))) // default is "rmcp"/3.11.0
            .with_instructions(INSTRUCTIONS)
    }
    async fn list_tools(&self, _: Option<PaginatedRequestParams>, _: RequestContext<RoleServer>)
        -> Result<ListToolsResult, McpError> {
        let tools = /* bridge.describe() → descriptor JSON */ vec![
            Tool::new("page_clickElement", "…", Arc::new(serde_json::from_value(schema)?))];
        Ok(ListToolsResult::with_all_items(tools))
    }
    async fn call_tool(&self, p: CallToolRequestParams, ctx: RequestContext<RoleServer>)
        -> Result<CallToolResponse, McpError> {
        // ctx.ct is cancelled by notifications/cancelled; pass it where TS passes extra.signal
        let mut r = CallToolResult::success(vec![ContentBlock::image(b64, "image/png"), ContentBlock::text(note)]);
        r.is_error = None;               // success() sets Some(false) → "isError":false on the wire
        Ok(r.into())                     // failures: CallToolResult::error(vec![ContentBlock::text("CODE: msg")])
    }
    async fn list_resources(&self, _: Option<PaginatedRequestParams>, _: RequestContext<RoleServer>)
        -> Result<ListResourcesResult, McpError> {
        Ok(ListResourcesResult::with_all_items(vec![Resource::new("vexdom://page/text", "Active page text")
            .with_description("Rendered text of the active tab.").with_mime_type("text/plain")]))
    }
    async fn read_resource(&self, p: ReadResourceRequestParams, _: RequestContext<RoleServer>)
        -> Result<ReadResourceResponse, McpError> {
        // TS throws a plain Error → -32603; rmcp's resource_not_found() would be -32002
        Err(McpError::internal_error(format!("Unknown resource: {}", p.uri), None))
    }
}

let running = Vexdom { … }.serve(stdio()).await?;           // waits for initialize
let peer = running.peer().clone();
tokio::spawn(async move { while manifest_changed.recv().await.is_ok() {
    let _ = peer.notify_tool_list_changed().await; } });     // {"jsonrpc":"2.0","method":"notifications/tools/list_changed"}
let _ = running.waiting().await;                             // returns QuitReason::Closed on stdin EOF
std::process::exit(0);                                       // don't let runtime shutdown wait on a blocked stdin read
```

Notes from the source:

- Requests are spawned one task each, so they run concurrently. On `notifications/cancelled` the
  handler's `ct` is cancelled and its response dropped (`src/service.rs` L1562-1581, L1693-1727). This
  matches TS.
- On stdin EOF rmcp waits up to 5 s for in-flight responses before closing (`src/service.rs` L1830-1880).
  TS exits immediately.
- The low-level path does not validate tool names (`get_tool` default `None` "bypasses validation",
  `src/handler/server.rs` L629-635). `structuredContent` is only emitted if set
  (`src/model.rs` L4167-4194).

### rmcp 3.11 default vs SDK 1.29 on the wire (**observed**, same script)

| Case | SDK 1.29 | rmcp 3.11 default |
| --- | --- | --- |
| success envelope | `{"result":…,"jsonrpc":"2.0","id":3}` | `{"jsonrpc":"2.0","id":3,"result":…}` (`src/model.rs` L585-590) |
| notification | `{"method":"notifications/tools/list_changed","jsonrpc":"2.0"}` | `{"jsonrpc":"2.0","method":"notifications/tools/list_changed"}` (`src/model.rs` L618-623) |
| capabilities | `{"tools":{"listChanged":true},"resources":{}}` | `{"resources":{},"tools":{"listChanged":true}}` (field order, `src/model/capabilities.rs` L224-244) |
| success tool result | `{"content":[…]}` | `{"content":[…],"isError":false}` (`CallToolResult::success`, `src/model.rs` L4268-4276) |
| `inputSchema` key order | as authored | **sorted** (`additionalProperties,properties,type`) unless `serde_json/preserve_order` is enabled in the build (rmcp's `serde_json = "1.0"` has no features, `Cargo.toml.orig` L52; `JsonObject = serde_json::Map`, `src/model.rs` L45) |
| unknown method `foo/bar` | `-32601 "Method not found"` | `-32601 "foo/bar"` (`on_custom_request`, `src/handler/server.rs` L636-648) |
| `prompts/list`, `logging/setLevel`, `server/discover` after init | `-32601 "Method not found"` | `-32601 "<method>"` (`method_not_found::<M>()` uses the method string, `src/model.rs` L721-723) |
| `resources/templates/list` | `-32601 "Method not found"` | `{"resourceTemplates":[]}` (default when `resources` is declared, `src/handler/server.rs` L512-523) |
| `tools/list` before `initialize` | served | `-32602 "request _meta is missing …"` |
| `server/discover` before `initialize` | `-32601 "Method not found"` | `DiscoverResult` (era switch, see above) |
| `"jsonrpc":"1.0"` / no `jsonrpc` | no reply | `{"jsonrpc":"2.0","error":{"code":-32600,"message":"Invalid request"}}` with no `id` (`src/transport/async_rw.rs` L255-262, L325-331) |
| non-JSON line | no reply | no reply (`src/transport/async_rw.rs` L317-324) |
| `tools/call` `params:{}` | `-32603` + zod issues text | `-32601 "tools/call"` (failed parse falls through to `CustomRequest`; `ClientRequest` union `src/model.rs` L4955) |
| `initialize` `2024-10-07` | echoes `2024-10-07` | `2025-11-25` |

Both send errors as `{"jsonrpc":"2.0","id":…,"error":…}` and both answer `ping` with `{}`.

### Bending rmcp to SDK-1.29 bytes

Each item needs a different part of rmcp:

1. Envelope order and the silent drop of malformed lines need a custom `Transport<RoleServer>`
   (`send`/`receive`/`close`, `src/transport.rs` L127-150). It re-serializes every outgoing message in TS
   order and replaces rmcp's `-32600` reply with silence.
2. "Method not found" everywhere, no discover, no template list, and the 2024-10-07 echo need either a
   direct `Service<RoleServer>` impl (`src/service.rs` L321-346) with its own dispatch, or one override per
   default method. `serve_directly` (`src/service.rs` L1341-1380) skips rmcp's pre-init gate, so
   `initialize` is answered by the `Service` itself.
3. Key order needs `serde_json/preserve_order` for the whole build (Cargo feature unification). Results
   need `is_error = None` on success.

What is left of rmcp after that is its model types and its spawn/cancel loop.

## 3. A hand-rolled JSON-RPC loop (agent-browser style)

agent-browser's `cli/src/mcp.rs`:

- Synchronous `for line in stdin.lock().lines()`, one request at a time, so there is no concurrency and
  no cancellation (L591-621). No server notifications, no resources.
- Supported versions `2025-11-25…2024-11-05`, with echo-or-latest negotiation (L24-26, L759-786).
- Bad JSON gets `-32700 "Parse error: …"` with `"id":null` (L673-684), where TS stays silent.
- A missing method gets `-32600` (L689). An unknown method gets `-32601 "Method not found: <m>"`
  (L207-212, L706-723).
- Messages are built with `json!({"jsonrpc","id","result"})` (L697-701, L5242-5258). The key order
  therefore depends on whether `preserve_order` is enabled.
- Its own tools add `structuredContent` (L2744).

None of that matches SDK 1.29 as-is. The parts to keep are the shape (one file, `serde_json::Value`
messages, no SDK) and the version list.

What vexdom's loop needs:

- An async reader that splits on `\n`, drops a trailing `\r`, and silently skips lines that are not valid
  JSON-RPC 2.0 messages.
- One `tokio::spawn` per request, with a `HashMap<RequestId, CancellationToken>` that
  `notifications/cancelled` cancels. A cancelled request sends nothing.
- A single writer task fed by an `mpsc` channel that writes `serde_json::to_string(&msg)` plus `\n` and
  flushes.
- Envelopes built in TS order: `result,jsonrpc,id` / `jsonrpc,id,error` / `method,jsonrpc`.
- Dispatch for `initialize` (with SDK 1.29's version list), `ping`, `tools/list`, `tools/call`,
  `resources/list` and `resources/read`. Everything else gets `-32601 "Method not found"`.
- A manifest-change stream that writes the `list_changed` notification at any time, as TS does.
- Exit on stdin EOF.

That is about 250 lines with `tokio` (`io-std`, `sync`, `rt`, `macros`) and `serde_json` with
`preserve_order`. Every byte in it can be checked against SDK 1.29.

## Semantic traps

1. **Era switch via `server/discover`.** If `server/discover` gets any answer other than "not found",
   dual-era clients stop sending `initialize`. Answer it `-32601 "Method not found"` like SDK 1.29. A
   `-32022` also counts as modern (spec stdio §Backward Compatibility).
2. **Envelope order is per message type**: success `result,jsonrpc,id`, error `jsonrpc,id,error`,
   notification `method,jsonrpc` (`shared/protocol.js:373-377,394-402,862-865`). serde structs and
   `json!` without `preserve_order` produce other orders.
3. **`preserve_order` is global, and it changes `remove`.** It is a Cargo feature, so enabling it in the
   binary also changes `serde_json::Map` ordering in the vendored agent-browser crates (ADR-003). Under
   `preserve_order`, `Map::remove` is `swap_remove`, which reorders the remaining keys (serde_json 1.0.154
   `src/map.rs` L160-166). The TS `{shot, ...rest}` / `{image, ...challenge}` destructuring
   (`src/daemon/tool-host.ts:372,396`) needs `shift_remove`.
4. **JS reorders integer-like keys.** `JSON.stringify(JSON.parse('{"b":1,"2":0,"10":3}'))` gives
   `{"2":0,"10":3,"b":1}` (**observed**, node 26). Every result body passes through `JSON.stringify` in
   `render()` (`src/daemon/tool-host.ts:442`), so a Rust `Value` passed straight through keeps the
   extension's order and differs. ECMA-262 OrdinaryOwnPropertyKeys orders array-index keys first.
5. **Numbers and strings re-serialize differently** (**observed**, serde_json 1.0.154 vs node 26):
   `1.0`→`1.0` vs `1`, `-0.0`→`-0.0` vs `0`. A lone surrogate `"\ud800"` is a parse **error** in serde_json
   but round-trips in JS. `1e21` gives `1e+21` in both, and control-character escapes match. Shared with
   topic `js-regex-and-utf16`.
6. **zod rewrites `tools/call` results.** Schema key order, stripped extra keys in content blocks, and
   `atob()`-checked image data. An invalid base64 image becomes `-32602 "MCP error -32602: Invalid
   tools/call result: …"` (`server/index.js:137-142`, `types.js:750-760`). `splitDataUrl`'s fallback
   `['image/png','']` passes because `atob('')` is valid (`src/daemon/tool-host.ts:427-430`).
7. **Malformed-request messages are zod text** (`-32603` + issue JSON). They cannot be reproduced
   byte-for-byte without zod. List them in `known-diffs.jsonl` (ADR-008).
8. **`isError` presence**: TS omits it on success. rmcp's `success()` emits `false`, and agent-browser
   always emits it.
9. **Error codes from thrown errors**: a plain `Error` becomes `-32603`, so `Unknown resource` is `-32603`,
   not rmcp's `RESOURCE_NOT_FOUND -32002` (`src/model.rs` L636-645). A `failure()` result from tool-host
   is an `isError` result, not a protocol error.
10. **No init gate in TS.** `tools/list` and `tools/call` before `initialize` are served, and a manifest
    change before `initialize` still sends `list_changed` (`server/index.js:173-190` only checks the
    capability).
11. **Response order is completion order** (**observed**). Golden-session diffs must match responses by
    `id`, not by line.
12. **stdout is the protocol.** A `tracing_subscriber::fmt()` default writer, a `println!`, or a panic
    message on stdout corrupts the stream (spec stdio: "MUST NOT write anything to its stdout that is not a
    valid MCP message"). Log to stderr or to the log file.
13. **Blocking stdin.** With tokio `Stdin`, a runtime shutdown can wait on the read
    (`src/transport/io.rs` L17-19). Call `process::exit(0)` after cleanup, as `cli.ts:247` does.
14. **Line limits**: rmcp drops lines over 16 MiB and TS has no cap. A tool call carrying a large
    `fillInput` value or `runCode` source would be lost silently.
15. **Default `serverInfo`** in rmcp is `{"name":"rmcp","version":"3.11.0"}` (`env!` evaluated inside
    rmcp, `src/model.rs` L1721-1730).

## Recommendation

**Faithful port (P3 `mcp` fallback through P5 W5 `server`): hand-roll the stdio JSON-RPC loop. Do not
take rmcp.**

- The TS surface is six methods plus one notification.
- To match SDK 1.29 bytes, rmcp would need a custom transport, a custom dispatch and `serve_directly`
  (§2). That keeps only its model types, while adding 2026-07-28 behavior (discover, `_meta` lifecycle,
  cache hints) that ADR-008 would have to fight.
- rmcp 3.11.0 is 8 days old, and the 3.x line has had 12 releases between 2026-08-31 (3.2.0) and
  2026-11-17 (3.11.0) (crates.io API `https://crates.io/api/v1/crates/rmcp`).

Concretely:

- Put the loop in `crates/vexdom-mcp`, using `tokio` and `serde_json` with `preserve_order`.
- Mirror SDK 1.29: version list including `2024-10-07`, echo-or-`2025-11-25`, the envelope orders from
  trap 2, `-32601 "Method not found"` for everything unhandled (including `server/discover`), silence on
  malformed lines, cancel-suppresses-reply, concurrent handlers, and exit on EOF.
- Normalize result bodies with JS semantics (traps 3–6) in the shared JSON layer, not in the MCP loop.
- Record the zod-text cases (trap 7) as known diffs.

**After the lift**: write an ADR for adopting rmcp (pinned with `=`, at a release at least two weeks old)
when vexdom wants the 2026-07-28 modern era (stateless `_meta`, `server/discover`,
`subscriptions/listen`). That change is intended, so it is not an equivalence diff. Keep the
`ToolHost` API (`list`, `call(name, args, cancel)`) independent of the JSON-RPC layer so that swap stays
local.
