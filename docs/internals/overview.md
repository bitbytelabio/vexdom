# Overview

Vexdom is a browser extension and one Rust binary, `vexdom`. The binary runs as several
processes, each started by something different. They talk only over loopback, and Vexdom Bridge
(the daemon) sits in the middle.

---

## Why there is a daemon at all

A Manifest V3 service worker cannot listen for connections. It can only dial out, and the browser
stops and restarts it whenever it likes. A Firefox background page has no listening socket either.
So the extension has to be a client, and the meeting point has to live outside the browser.

That meeting point is the Bridge. It holds the links to every paired browser that is connected
and serves them to the side panel's agent runs, the CLI, any number of MCP clients and
`Vexdom.app`:

```
You ──type or speak──> Extension ──WebSocket (Cap'n Proto)──> vexdom daemon ──starts──> claude
                          ▲                                        ▲                     │
                          └────────────── page actions ────────────┘◄── vexdom mcp ◄─────┘

MCP client ──stdio──> vexdom mcp ──control socket──> the same daemon ──> the same browser
vexdom click … ───────────────────control socket──> the same daemon
Vexdom.app ───────────────────────control socket──> the same daemon
```

Everything binds to `127.0.0.1`. Nothing listens on a public interface, and there is no cloud
component.

---

## The processes

| Process | Started by | Lives for | Job |
| --- | --- | --- | --- |
| **Extension** (`packages/extension/`) | The browser | As long as the browser runs | Owns the tabs. Runs the side panel, the settings page, the background (service worker on Chromium, background page on Firefox) and a content script per page |
| **Bridge** (`vexdom daemon`) | The first CLI command, MCP server or native-host `ensure` that needs it (`crates/vexdom-daemon/src/ensure.rs`), or `vexdom start` | Until 30 minutes with no browser link and no control connection (`--no-idle-exit` turns that off), or `vexdom stop` | Owns the browser links, pairing, guardrails, confirmations, snapshot refs and deltas, agent runs, schedules, files written for the user |
| **MCP server** (`vexdom mcp`) | The MCP client, over stdio | The client's session | Serves three tools (`snapshot`, `act`, `guide`), the playbooks and the page resources, and turns each call into control-socket calls. One process per client |
| **Claude Code** (`claude -p`) | The Bridge, per side-panel instruction | One turn | Reasons about the instruction and calls the same three tools through its own `vexdom mcp`, started with `VEXDOM_AGENT_RUN` set to the run id |
| **Native host** (`vexdom native-host`) | The browser, through native messaging (`com.vexdom.bridge`) | One message | Answers `ensure`, `hello` or `pair`, then exits |
| **`Vexdom.app`** (`desktop/`) | You | Until you quit it | Optional macOS front end: starts and stops the Bridge, approves pairing and external confirmations, shows notices |

The MCP server holds no browser state. Killing it leaves the browser link untouched, because the
link belongs to the Bridge. The same is true of a CLI command: it connects, makes its calls and
exits.

A running Bridge keeps serving the build it was started from. After a rebuild or an update, restart
it with `vexdom restart`.

---

## What builds which process

The Cargo workspace is the repository root (`members = ["app", "crates/*"]`; `desktop/` is a
workspace of its own).

| Crate or folder | What it holds |
| --- | --- |
| `app/` | The `vexdom` binary: `src/cli/` (every command; `mod.rs` holds `usage()`, `page.rs` and `verbs.rs` the page verbs), `src/mcp/` (`mod.rs` the rmcp server, `host.rs` the three tools), `src/native_host.rs`, `src/read_url.rs` (`snapshot {"as":"markdown","url":…}` and `vexdom read <url>` without a tab) |
| `crates/vexdom-schema` | `schema/vexdom.capnp`: every message on the extension link and the control socket |
| `crates/vexdom-link` | Cap'n Proto RPC over a WebSocket, as an unaligned byte stream in binary frames; `connect_control` for clients |
| `crates/vexdom-client` | `Bridge`: a control-socket client used by the CLI, the MCP server and `Vexdom.app` |
| `crates/vexdom-daemon` | The Bridge: listener (`http.rs`), extension gate (`gate.rs`), control socket (`control.rs`), actions (`invoke.rs`, `page.rs`), confirmations, artifacts, WebMCP, video recording, axe, agent runs (`agent/`), the scheduler |
| `crates/vexdom-core` | Paths, config, the lockfile, pairing and sessions, guardrails, skills, the embedded tool manifest (`tools.rs`), project config |
| `crates/vexdom-pure` | Logic with no I/O shared with the extension's wasm: handshake crypto, sealing, intent routing, blocked sites, schedules, profile, secret detection |
| `crates/vexdom-snapshot` | Snapshot text with refs (`e5`), ref maps, deltas, markdown shaping, screenshot diffs, WebMCP text |
| `crates/vexdom-update` | Signed self-update (`vexdom update`, ADR-031) |
| `crates/ext-core` | The extension's end of the link plus the `vexdom-pure` exports, compiled to wasm with BoltFFI |
| `packages/extension/` | The extension (WXT, Bun): Chrome Manifest V3 and Firefox Manifest V2 builds from one source |
| `desktop/` | `Vexdom.app`, in GPUI |

The CLI, the MCP server and `Vexdom.app` never touch the browser directly. Each one is a control
client of the Bridge.

---

## The paths through it

| | |
| --- | --- |
| **[An MCP or CLI call](request-path.md)** | An MCP client calls `act`, or you run a page verb such as `vexdom click`. No agent runs on Vexdom's side |
| **[A side-panel run](agent-runs.md)** | You type or speak into the side panel. The instruction goes through the intent funnel, and may start Claude Code, whose tool calls come back through `vexdom mcp` |

The second path closes onto the first. Claude Code gets its own `vexdom mcp`, which connects to the
same Bridge, so a run uses the same three tools an outside client gets. Its calls carry the run id,
so the Bridge gates them as the run's own ([Guardrails](guardrails.md)) and the extension sends
them to the run's tab ([Inside the extension](extension.md)).

`Vexdom.app` is not on the request path. It drives the Bridge over the control socket: see
[The macOS app](mac-app.md).

---

## Next

**[Transport →](transport.md)**: how a client connects in the first place.
