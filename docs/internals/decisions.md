# Decisions

Architecture decisions for the vexdom port. Numbered in order, never renumbered; a changed decision
gets a new ADR that supersedes the old one. This file is the source of truth for the port.

## ADR-001: Scope of the Rust rewrite
Status: accepted (2026-11-25)
Context: Browsentic is an extension (WXT, TypeScript), the Bridge daemon + CLI + stdio MCP server
(Node, ~20.8k LOC in `src/daemon` plus ~9k LOC of `src/lib` it bundles), and two desktop apps (Swift,
Tauri). Extension code must run in the browser.
Decision: Rust replaces the Bridge daemon, the CLI, the MCP server and the agent runner. The
extension stays TypeScript (forked and edited in `src/extension` + `src/lib`).
Consequences: the extension is a `keep` unit for the daemon port; its pure logic moves later (ADR-015).

## ADR-002: rust-anything drives the port
Status: accepted (2026-11-25)
Context: the port needs measured behavior equivalence, not a rewrite from memory.
Decision: follow rust-anything 4.1.0's whole-app flow: record → front door → per-unit bridge/record/
port/diff/gate → wire → replay → lift.
Consequences: every unit claim, gate and replay result goes in `MAP.md`; `PORT_MAP.md` is generated.

## ADR-003: Repository layout
Status: accepted (2026-11-25)
Context: rust-anything writes into `<repo>/rust/` beside the original source.
Decision: vexdom is a fork of Browsentic with its full history (remote `upstream`, v0.9.2 @
`9c0892a`). All Rust lives in `rust/`. Code vendored from agent-browser (`../agent-browser`,
v0.45.0 @ `f0a9835`, Apache-2.0) lives in `crates/` with attribution in `NOTICE`; agent-browser's
`axe.min.js` (MPL-2.0) is not taken.
Consequences: the TypeScript daemon stays as reference and fallback until the lift.

## ADR-004: Full rebrand, done in TypeScript first
Status: accepted (2026-11-25)
Context: compatibility with the store Browsentic extension was explicitly dropped.
Decision: binary `vexdom` (subcommands `mcp`, `daemon`, `native-host`), state `~/.vexdom`, files
`~/vexdom`, env `VEXDOM_*`, ports 8785–8787 (so it runs beside an installed Browsentic on
8765–8767), native host `com.vexdom.bridge`, handshake labels `vexdom/…`, tool prefix `vexdom_*`,
resource URIs `vexdom://`, a new extension id. The rebrand is made on the TypeScript fork first and
that rebranded build is what gets recorded.
Consequences: Browsentic's store extension ids are not trusted by the vexdom native host or daemon.

## ADR-005: Snapshot with refs from a content-script accessibility tree
Status: accepted (2026-11-25)
Context: `page.getPageInfo` is a DOM walk returning JSON with a CSS path per element (~35–70 tokens
each, 3.5–8K tokens a page). agent-browser's snapshot (`- button "Send" [ref=e4]`) costs ~8–15 tokens
an element. Chrome's debugger shows an infobar on every attach and Firefox has none.
Decision: a new extension action `page.snapshot` computes an accessibility-lite tree in the content
script (role, accessible name, states; open shadow roots; same-origin iframes; cursor-interactive
elements) and returns AXNode-shaped JSON. The Rust daemon renders it with the pipeline vendored from
agent-browser. A ref is a per-document durable id assigned by the content script (`e{n}`), usable as
`target.ref` by every element action.
Consequences: Chrome and Firefox both work, no infobar; fidelity is lower than CDP's AX tree.

## ADR-006: Two agent interfaces over one control socket
Status: accepted (2026-11-25)
Decision: keep the stdio MCP server (with a slimmed catalog) and add agent-browser-style shell
commands (`vexdom snapshot -i`, `vexdom click @e3`, `fill`, `read`, `screenshot`, `batch`,
`vexdom skills get core`). Both reach the daemon through the control socket.

## ADR-007: Token-saving features
Status: accepted (2026-11-25)
Decision: snapshot + refs; a snapshot delta in the reply of click/fill/type/select/press/submit/
navigate (full snapshot when the delta is ≥70% of it); a slim catalog (one shared `target`
definition, short descriptions, shared arguments stated once in the server instructions); the fence
note stated once with a marker per result; `--max-output`; markdown `read` with `--outline` and
`--filter`; screenshot `--if-changed` and `--annotate`; `batch`. Each is measured against the
baseline in `bench/tokens`.

## ADR-008: Equivalence bar
Status: accepted (2026-11-25)
Decision: for the faithful port, output must be equal after normalizing nonces, tokens, uuids, pids
and timestamps; JSON key order, exit codes, error codes and error messages must match exactly.
Output changed on purpose (slim catalog, fence) is listed in `known-diffs.jsonl` with this ADR as
its reason.

## ADR-009: Trimmed scope
Status: accepted (2026-11-25)
Decision: drop npm self-update and npx-cache handling, desktop-app detection/launching, Android
(daemon `android/`, the extension's phone mirror), and every agent runner except Claude Code.
Antigravity (`agy`) and oh-my-pi (`omp`, via `omp acp` or `--mode json`) come back as Rust runners
after the lift (ADR-011 order). The removed code stays in git history and in `../browsentic`.
Amended 2026-11-26: Claude Code stays the only runner, and no ACP transport is built; Antigravity
and oh-my-pi do not come back (user decision; `omp acp` 19.2.1 reaches MCP tools only as bash
commands, HANDOFF.md).

## ADR-010: Distribution and targets
Status: accepted (2026-11-25)
Decision: GitHub Releases for macOS arm64/x64, Linux x64/arm64 (gnu and musl) and Windows x64, plus
an install script. Rust stable, edition 2024. The extension installs unpacked through
`vexdom setup --unpacked`; a signed unlisted Firefox add-on comes later; store listings are not
planned.

## ADR-011: Token work early, in parallel with the faithful port
Status: accepted (2026-11-25)
Decision: the Rust front door comes up early; the snapshot/refs/delta/read pipeline is new Rust code
wired into it from the start, together with `page.snapshot` in the extension. Every other unit is
ported wave by wave.

## ADR-012: Checkpoints
Status: accepted (2026-11-25)
Decision: stop for the user's go-ahead after P0, P3, P4 and after every wave of P5. Commits and
pushes only with the user's approval at a checkpoint.

## ADR-013: Desktop GUI in GPUI
Status: accepted (2026-11-25)
Decision: the desktop app (formerly Swift on macOS, Tauri on Windows) is rewritten in GPUI, sharing
crates with the bridge and driving the daemon over the control interface. After the lift (P10).
The side panel stays in the extension.

## ADR-014: Cap'n Proto for the extension link and the control socket
Status: accepted (2026-11-25); spike S1 passed (research/spike-results.md), refined by ADR-018
Context: `capnp` / `capnp-rpc` 0.28 (MIT) depend only on `futures`, implement RPC level 1 between two
parties, and need an `AsyncRead`/`AsyncWrite` transport. JavaScript Cap'n Proto libraries are alpha
(corrected by research/capnp-wasm.md: capnp-ts 0.9.3, beta, does implement RPC level 1 over WebSocket).
Decision: spike in P1 (S1). The faithful port keeps the JSON v26 protocol so the protocol suite
stays the proof. After the lift: one `.capnp` schema for the extension link and the control socket
(one bootstrap interface, role decided by the handshake, which stays HMAC and runs before
bootstrap); watches and streams become callback capabilities; screenshots travel as `Data`. The
daemon speaks JSON and Cap'n Proto side by side until every client moved.
Consequences: no effect on LLM tokens; binary frames need `vexdom debug decode`.

## ADR-015: Rust in the extension, as much as possible
Status: accepted (2026-11-25)
Decision: the pure logic of `src/lib` (bridge, secrets, intent, recordings, schedules, settings…)
moves into `ext-core`, compiled to wasm and run in the service worker, offscreen document and
extension pages. TypeScript keeps the browser-API layer, the React UI and the content scripts' DOM
code (wasm in a content script can be blocked by the page's CSP, and the binding cannot touch the
DOM). Units are ported with rust-anything's wasm bridge for record/diff/gate.

## ADR-016: Binding for the extension core
Status: accepted (2026-11-25); spikes S2 and S2b passed, refined by ADR-017
Decision: BoltFFI (0.32, MIT, pre-1.0) behind spike S2; any failed gate switches to wasm-bindgen.
`ext-core`'s API is written so the binding can change without touching its logic.

## ADR-017: ext-core is BoltFFI-only, driven by JavaScript
Status: accepted (2026-11-25)
Context: spikes S2 and S2b (research/spike-results.md). BoltFFI 0.32 handles every shape ext-core
needs in a Chrome MV3 service worker and a Firefox MV2 background page, CSP-clean. Its exported
futures must be `Send`; capnp-rpc's are not (`Rc` servers).
Decision: `ext-core` uses BoltFFI as its only binding (no wasm-bindgen in the module). Anything that
runs on capnp-rpc lives on a thread-local `LocalPool` inside wasm and is driven by synchronous
exports: JavaScript owns the WebSocket and the service-worker lifecycle, feeds each binary frame in
and sends whatever the pump returns. Async exports are used only for `Send` work. Pin BoltFFI to an
exact version and regenerate bindings on every upgrade; `boltffi pack wasm` runs with `wasm-opt`,
`tsc` and `@boltffi/runtime` resolvable inside the crate's own package.json.
Consequences: wasm stays 282 KB / 115 KB gzip with capnp-rpc; reconnect and keepalive stay in
TypeScript; wasm-bindgen remains the fallback only if BoltFFI breaks a release.

## ADR-018: Cap'n Proto transport rules
Status: accepted (2026-11-25)
Context: spikes S1/S2b and research/capnp-wasm.md.
Decision: the WebSocket is a byte stream for capnp — binary frames carry capnp bytes with no
message alignment, text frames carry JSON (the HMAC handshake first, then keepalive pings every
20 s). The extension sends the first binary frame after `welcome`. The daemon runs capnp on a
dedicated current-thread runtime with a `LocalSet`. Extension service workers use no top-level
`await`; the manifest sets `'wasm-unsafe-eval'` and `minimum_chrome_version` 116.
Consequences: `vexdom debug decode` reads the binary frames; the handshake's MAC'd claims must be
extended before the transport choice is negotiated in `welcome` (research/capnp-wasm.md).

## ADR-019: Hand-rolled MCP server with the TypeScript SDK's bytes
Status: accepted (2026-11-25)
Context: research/mcp-crate.md. rmcp 3.11 answers the 2026-07-28 `server/discover` probe, which
moves clients to the new protocol era, and differs from SDK 1.29 on the wire (envelope and
capability key order, `isError:false`, sorted schema keys, unknown-method text, templates/list,
pre-initialize and malformed-line replies). ADR-008 asks for the original's bytes.
Decision: `vexdom mcp` is an async, concurrent JSON-RPC loop of our own (~250 lines) that answers
exactly as SDK 1.29 does, `server/discover` included (-32601). rmcp is reconsidered in a new ADR
after the lift, if vexdom wants the 2026-07-28 era.
Consequences: the slim catalog (ADR-007) changes `tools/list` content, not the envelope; MCP golden
sessions recorded in P2 pin the envelope.

## ADR-020: axum listener and front door
Status: accepted (2026-11-25)
Context: research/http-ws-upgrade-proxy.md, with an end-to-end proxy probe.
Decision: axum 0.8.9 (tokio-tungstenite 0.29) for `/health` and WebSocket upgrades on 127.0.0.1,
ports tried in order with the TypeScript's error text when all are busy. Host, Origin and
constant-time bearer checks run before the handshake is validated, refusing with 403 and
`X-Vexdom-Reason`. Frames up to 100 MiB, as `ws` allows. In the P3 front door, `/health` reports the
front door's own pid (the lockfile's), the inner TypeScript daemon runs with `VEXDOM_PORTS=0` and
`VEXDOM_LOCKFILE=<private>`, it is dialed before the client gets its 101, control frames are relayed
with the token swapped (selected ops answered in Rust), and extension connections are spliced
byte for byte.
Consequences: Windows busy-port options, non-WebSocket upgrades and header whitespace are recorded
in P2 before the listener is ported; supervising the inner daemon is designed in P3.

## ADR-021: Who assigns refs, and how the page tree is computed (refines ADR-005)
Status: accepted (2026-11-25)
Context: research/content-script-ax.md and research/agent-browser-vendoring.md. Refs must stay unique
across frames and documents, and agent-browser's pipeline reads `backendDOMNodeId`, not refs. On a
4,925-element fixture, Playwright's `roleUtils.ts` agreed with Chrome's own tree on 2,145 of 2,147
nodes in 13 ms (Chrome isolated world); `dom-accessibility-api` missed shadow-DOM text and several
roles. Chrome has no `runtime.getFrameId`; `documentId` needs Firefox 153 while vexdom supports 140.
Decision: the content script gives every element a durable per-document id (WeakMap + Map of
WeakRefs) under a random per-instance document key, and walks the page with Playwright's
`roleUtils.ts`/`domUtils.ts` (Apache-2.0, vendored with `cssTokenizer.ts`, CC0) patched for `inert`
and modal-blocked content, closed shadow roots (`chrome.dom.openOrClosedShadowRoot`,
`element.openOrClosedShadowRoot` in Firefox) and `summary` → button. It emits a compact schema; the
daemon converts it to AXNode, maps (frameId, docKey, id) to `backendDOMNodeId`, and agent-browser's
`RefMap` prints `e{n}` unchanged. Each frame is snapshotted by its own content script and stitched
in the background (frame ids on Firefox, a postMessage nonce on Chrome). Stale refs follow
agent-browser's refusals, with its unique role+name recovery.
Consequences: ADR-005's "content script assigns `e{n}`" is replaced by "content script assigns ids,
the daemon prints refs". NOTICE gains Playwright when it is vendored (P4).

## ADR-022: Cargo workspace layout (refines ADR-003)
Status: accepted (2026-11-25)
Context: P3 brings the first Rust crates (`app`, `vexdom-door`); later waves add first-party
crates and rust-anything unit crates. rust-anything defaults new crates to `rust/<name>`, but its
`app wire` and `map` take explicit paths and `map` walks folders recursively (rust-anything 4.1.0
`src/front_door.rs:877-905`, `src/port_map.rs`). rust-anything runs cargo with `--manifest-path`
from the caller's cwd, and rustup finds a toolchain file only from the cwd upward; the machine's
default toolchain is a nightly.
Decision: `Cargo.toml` is a virtual workspace (`resolver = "3"`, `members = ["app",
"crates/*"]`, `exclude = ["spikes"]`) with one committed `Cargo.lock` and one `target/`.
`app` is the `vexdom` binary (front door now, the whole app after the lift), kept thin.
`crates/` holds every library crate: first-party `vexdom-*` crates (P3: `vexdom-door`; later
`vexdom-snapshot`, `vexdom-mcp`, `vexdom-schema`, `ext-core`) and rust-anything unit crates, bridged
with `rust-anything bridge . <unit> --out crates/<unit>-rs` and wired with
`rust-anything app wire . crates/<unit>-rs`. The workspace shares `[workspace.package]`
(version, edition 2024, rust-version, license, repository, `publish = false`),
`[workspace.dependencies]` for every external crate (members use `x.workspace = true`) and
`[workspace.lints]`; first-party crates set `[lints] workspace = true`, generated unit crates keep
their generated manifests. `rust-toolchain.toml` sits at the repository root and pins `1.100.0`.
Spikes stay throwaway in `spikes/` (own workspaces and lockfiles, excluded). Port docs
(`MAP.md`, `DECISIONS.md`, …) stay in `rust/`; `bench/` holds non-cargo benchmarks.
Consequences: one `cargo build`/`clippy`/`test` in `rust/` covers every crate; every rust-anything
command that creates a crate needs an explicit `--out crates/…`.

## ADR-023: How the TypeScript reference is recorded
Status: accepted (2026-11-25)
Context: the CLI reads and writes `~/.vexdom`, starts a Bridge for many commands, and detects
agents and browsers on PATH and in `/Applications`. rust-anything 4.1.0 keeps one `run` in
`app-cases/rust-anything.json`, and http mode sticks once set (`app.rs` `record()`,
`http::merge_config`), so `app record --http` would replace the CLI recording.
Decision: CLI cases (`app-cases/`) are recorded and replayed through `scripts/app-env.sh`
(fixed sandbox: `HOME=/tmp/vexdom-app-home`, a PATH with only `node` and system dirs,
`VEXDOM_PORTS=0`, `TZ=UTC`), serially, with `--db-reset scripts/app-reset.sh` (stops the case's
Bridge, fresh HOME, seeds `home/` from the case files) and `--db-dump scripts/app-dump.sh`
(stops the Bridge, prints what the case left in HOME), so per-case state is compared and no daemon
leaks. Cases are macOS-recorded; `browsers`, `setup` and `update` depend on the browsers installed
in `/Applications`. The Markdown examples (`docs/guide/*.md`) are recorded automatically, normalized,
including those that start a Bridge. The HTTP surface (`/health`, refusals, upgrades, busy port) is
pinned by the protocol suite (`src/daemon/test/http-surface.test.ts`), not by `app record --http`.
The protocol suite runs against an external daemon when `VEXDOM_DAEMON_BIN` holds a command line
(whitespace-split, repo-relative paths allowed, a `*.js` first word runs under node,
`--no-idle-exit` appended): `node src/daemon/dist/daemon-main.js` for the TypeScript daemon,
`target/debug/vexdom daemon` for Rust. `VEXDOM_MCP_BIN` does the same for the MCP server
(`node src/daemon/dist/cli.js mcp`, `target/debug/vexdom mcp`). This replaces PLAN.md's
`VEXDOM_DAEMON_BIN=target/debug/vexdom` example. MCP golden sessions live in
`src/daemon/test/golden/mcp/*.jsonl`; the offline token baseline (`bench/tokens/baseline.{json,md}`)
is derived from them; live `claude -p` usage is measured in P4, before and after.
Consequences: replay must run through `app-env.sh`; tests that fake the daemon's clock or modules
skip in external mode; the busy-port check runs only in external mode.

## ADR-024: P3 front door daemon and its supervision (closes ADR-020's open question)
Status: accepted (2026-11-25)
Context: ADR-020 left supervision of the inner TypeScript daemon to P3. The TypeScript
`ensure-daemon` spawns `daemon-main.js` directly.
Decision: `vexdom daemon` (and argv0 `vexdom-mcpd`) binds the public listener (`VEXDOM_PORTS`, the
TypeScript's busy-port text) and writes its own lockfile with its own pid and a fresh token; the
inner TypeScript daemon runs with `VEXDOM_PORTS=0`, `VEXDOM_LOCKFILE=<stateDir>/daemon-inner.json`
and `--no-idle-exit`. The front door owns idle exit (30 min with zero open proxied sockets, unless
`--no-idle-exit`). The inner daemon dying ends the front door (exit 1, no restart loop);
SIGTERM/SIGINT (caught from before the inner daemon is spawned) send the inner SIGTERM, wait 3 s,
then kill: shorter than the 5 s `vexdom stop` gives the front door before its own SIGKILL, which
would orphan the inner daemon. `/health` is answered by the front door
from the inner's, with its own pid. Control frames are relayed with the token swapped and the
`port` of the `status` reply and of a `grant` reply's `outcome.grant` rewritten to the public port
(a grant's port is where the extension connects next); extension upgrades are spliced byte for byte.
The TypeScript `ensure-daemon` keeps spawning the TypeScript daemon until W4 ports it; no
TypeScript hook is added.
Consequences: the front door is reached only through `vexdom daemon` / `vexdom-mcpd` (and the
protocol suite's `VEXDOM_DAEMON_BIN`) during P3–W4; if the front door is SIGKILLed, the inner
daemon keeps running (no idle exit) until stopped by hand.

## ADR-025: Rust-only Bridge over Cap'n Proto, token MVP (re-scoped P4)
Status: accepted (2026-11-25)
Context: P4 was the token MVP on top of a faithful TypeScript port (ADR-008, ADR-011/012 waves),
with JSON on the wire until lift (ADR-014) and the front door relaying to the TypeScript daemon
(ADR-024). The user re-scoped it: Cap'n Proto replaces the JSON WebSocket protocol on both the
extension link and the control socket now, the `vexdom` daemon/CLI/MCP/native host become Rust
only with every current feature, and the TypeScript output is no longer an oracle.
Decision:
- Supersedes ADR-008 (no equivalence bar; new tests only), ADR-011/012's P5 waves, ADR-014's
  timing ("JSON until lift"), ADR-018's text frames and ADR-024 (front door). P5, P6 and P8 fold
  into P4; `src/daemon/` and `crates/vexdom-door` are deleted at the end of P4.
- **Wire.** Every WebSocket message between extension↔daemon and CLI/MCP↔daemon is a binary
  frame carrying capnp bytes as an unaligned byte stream (`schema/vexdom.capnp`, ADR-018 adapter).
  No text frames: the HMAC handshake is the `ExtensionGate` bootstrap, keepalive is a capnp call.
  JSON stays only where an outside spec mandates it: MCP stdio JSON-RPC, native messaging stdio,
  `claude -p --output-format stream-json`, files on disk.
- **Payload rule.** Documents whose type is owned and zod-validated by TypeScript in `src/lib`
  (action input/data, `AgentState`, `SkillCatalog`, `Preferences`, tasks, `RunContext`, recordings,
  drafts, `FileReport`, `DeviceNotice`, tool descriptors) travel as capnp `Json` (`Text`) fields;
  everything protocol-level is typed capnp.
- **On-disk compatibility.** Every file under `~/.vexdom` and `~/vexdom` keeps its path, format and
  mode; written JSON keeps the TypeScript key order (`serde_json` `preserve_order`).
- **Protocol 27.** `SOCKET_PROTOCOL_VERSION` = `MIN_EXTENSION_PROTOCOL` = 27; older extensions
  cannot connect (their JSON hello is not capnp).
- **Runtime.** The daemon runs one tokio `current_thread` runtime inside a `LocalSet` (capnp servers
  are `Rc`); shared state is `Rc<RefCell<…>>`; PBKDF2 runs on `spawn_blocking`.
- **Errors.** TypeScript error codes and messages are kept; new codes `UNKNOWN_REF`, `STALE_REF`,
  `NO_SNAPSHOT`. A ref absent from the tab's latest snapshot is refused.
- Crates: `vexdom-schema`, `vexdom-core`, `vexdom-snapshot`, `vexdom-link`, `vexdom-daemon`,
  `ext-core` (wasm, excluded from default members) and `app` (binary `vexdom`).
Consequences: existing paired browsers must update the extension; the live `claude -p` bench
(`bench/tokens/live.mjs`) is the before/after measure; CLI case replay (`app-cases/`) and the
TypeScript fallback are gone.

## ADR-026: Repository layout and platforms (supersedes ADR-003's `rust/`, refines ADR-010 and ADR-022)
Status: accepted (2026-11-26)
Context: the user asked for the Rust workspace at the repository root, the TypeScript extension in
its own package folder, and macOS as the platform that matters, with Windows dropped.
Decision:
- **Layout.** The Cargo workspace is the repository root: `Cargo.toml`, `app/`, `crates/`,
  `bench/`, `research/`, `spikes/` and the port's notes (`MAP.md`, `HANDOFF.md`, `DECISIONS.md`, …).
  The extension is the Yarn project `packages/extension/` (its own `package.json`, `yarn.lock`,
  vendored Yarn, `src/`, `scripts/`); every `yarn` command runs there. `boltffi pack wasm` writes
  ext-core's package into `packages/extension/.ext-core/`, so the tsc it runs resolves
  `@boltffi/runtime` and `@types/node` from the extension's `node_modules`.
- **Platforms.** macOS first (arm64 and x64); Linux keeps building and testing in CI. Windows is not
  supported: the Task Scheduler wake-up, registry native-messaging keys, `.cmd` shim resolution,
  the command-line length check, `taskkill`/`tasklist` and the `unregister-host` command are gone;
  CI runs a macOS job in place of the Windows one.
- **Dependencies** (user, 2026-11-26): every version, and every workspace crate's path, is declared
  once in the root `[workspace.dependencies]`; members only say `{ workspace = true }` (adding
  `features` where they need more). Spikes are workspaces of their own and keep their manifests.
- **Release targets** (user, 2026-11-26): `aarch64-apple-darwin`, `x86_64-apple-darwin`,
  `x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu`; one POSIX `install.sh`.
Consequences: ADR-010's Windows x64 and musl targets and the PowerShell installer are dropped; paths
in older ADRs and research notes were rewritten from `rust/…` to the root.

## ADR-027: Bun for the extension and the token bench (amends ADR-026)
Status: accepted (2026-11-26)
Context: the user asked to move `packages/extension/` and `bench/tokens/` from Node and the vendored
Yarn 4.17.1 to Bun, with no Node left. A probe on an APFS clone with only `bun` on `PATH` passed the
install (`yarn.lock` migrated to `bun.lock`), tests with coverage, `tsc`, both WXT builds,
`web-ext lint` and the tool-manifest emit (byte-identical).
Decision:
- Bun 1.4.5 is the package manager and the runtime (`packageManager: bun@1.4.5`, text `bun.lock`,
  `bun install --frozen-lockfile` in CI through `oven-sh/setup-bun`). `yarn.lock`, `.yarnrc.yml` and
  `.yarn/` are gone from both projects.
- `packages/extension/bunfig.toml` sets `[run] bun = true`: `bun run` puts a `node` shim on `PATH`,
  so bins with a `#!/usr/bin/env node` shebang (wxt, vitest, tsc, web-ext) run on Bun.
- Vitest stays the test runner, run on Bun. `bun test` lacks vitest projects, happy-dom per project,
  `vi.hoisted`/`vi.mock`/`vi.resetModules`/`vi.stubEnv`/`vi.waitFor` and the `CoverageByArea`
  reporter the coverage floors use.
- `esbuild` and `@types/node`, used only transitively before, are direct devDependencies
  (`scripts/emit-tools.ts`; boltffi's `tsc --types node`).
Consequences: ADR-026's "Yarn project" wording reads "Bun project"; every `yarn <script>` in the docs
is `bun run <script>`. Puppeteer's Chrome download is a manual
`bun --bun node_modules/.bin/puppeteer browsers install chrome` (Bun blocks its postinstall).
One exception, found live the same day: `web-ext run` (the hand-run Firefox session, in no script and
no CI job) dies on Bun with `connect ECONNREFUSED` before Firefox listens, because web-ext catches
that error through `node:domain`, which Bun stubs. It stays `node node_modules/.bin/web-ext run`
until web-ext drops `domain`; `web-ext lint` and `web-ext sign` run on Bun.

## ADR-028: agent-browser parity without CDP
Status: accepted (2026-11-26)
Context: the user asked for agent-browser's (`f0a9835`) features that a browser extension can do
without the Chrome debugger: scope A (content script, tabs API) and scope B (the `cookies`,
`clipboardRead` and `clipboardWrite` permissions).
Decision:
- **In scope.** Locators (`label`, `placeholder`, `alt`, `title`, `testId`, `name`, `last` on every
  target; `vexdom find …`); clicks with a button, modifiers and a count; `page.setChecked`,
  `page.query` (core, ungrouped); `page.insertText`, key `phase` down/up; multi and ARIA
  `selectOption`; scrolling by pixels, sideways and inside a container; `page.storage`,
  `page.cookies`, `page.clipboard`; `page.pushState`, `page.openWindow`, `page.waitFor`;
  `page.evaluate`; `captureDownload` with no trigger (the next download); `attachFile` with local
  `paths` (read by the Bridge, several files); CLI-only `state`, `diff` (snapshot, screenshot, url),
  `wait <ms>`, `screenshot --threshold` and the other screenshot flags, `mcp call`.
- **Out of scope, needs `chrome.debugger`:** mouse, viewport/device/geolocation/offline/media/headers/
  credentials, dialogs, network routing/requests/HAR, PDF, trace/profiler, console/errors verbs.
- **Out of scope, not the extension model:** open/connect/close browser, stream, dashboard, chat,
  profiles, inspect, plugins, react, vitals, a11y (axe is MPL), video recording, tap/swipe,
  sessions, auth vault, tab `--label` (`tab <text>` matches a title or URL instead).
- **`page.evaluate` has no approval gate** (user, 2026-11-26). In Chrome it runs through
  `userScripts.execute` in the page's world, so “Allow User Scripts” must be on; in Firefox
  `tabs.executeScript` hands the code to the page's own `eval` (`window.wrappedJSObject.eval`), never
  running it in the content-script sandbox, which holds the extension's APIs (so a page CSP without
  `unsafe-eval` refuses it there). Users who want a gate list `page.evaluate` in `require_approval`.
- **New groups:** `storage`, `navigation`, `input`, `code` (no skills yet).
- **Release fields** (`vexdom-pure` `seal.rs`): besides `page.fillInput.value` and
  `page.typeText.text`, a handle may become plaintext in `page.insertText.text` and
  `page.cookies.value`. The plan also named `page.storage.value` and `page.clipboard.text`; the
  security review (2026-11-26) dropped them, because storage and the clipboard read back in
  plaintext. Cookie values read back are sealed (a record's `value` takes the kind of its parent key,
  `cookies`), `page.cookies` with a `url` counts as a navigation for the run's scope, and its result
  is sealed with that URL's host. `vexdom state` files keep sealed cookie values, released only in
  the browser session that sealed them.
- **Sealing hardening from the same review:** only a whole base64 `data:` URL skips sealing (a
  `data:` prefix used to hide any string); the vault releases a handle only when its kind and origin
  match the entry; `page.query` never returns a password field's value; `page.waitFor` re-checks the
  blocked-sites list before each poll; `attachFile` `paths` refuses files in the Bridge's state
  folder and caps the count and total size.
- **`vexdom type` no longer clears the field first** (agent-browser's behavior); `--clear` does.
- **Content scripts ask the background about blocked sites** (`siteRefusal`): the matcher is
  `vexdom-pure` through ext-core's wasm (P9), which content scripts do not load.
Consequences: the MCP catalog grew from 9,610 to 11,969 tokens and the server instructions from 684
to 760 (`bench/tokens/baseline.md`). The manifest's new permissions make Chrome ask store users to
re-approve; unpacked installs are unaffected. No coverage floor was lowered by P9.

## ADR-029: MCP 2026-07-28 through rmcp, three tools, skills over MCP (supersedes ADR-019 for MCP and ADR-007's slim catalog)
Status: accepted (2026-11-26)
Context: the hand-rolled loop of ADR-019 answers as SDK 1.29 and refuses `server/discover`, so
clients stay on the legacy era; the slim catalog still lists every page action (11,969 tokens).
`../raijin`'s MCP server shows a gateway shape: a few tools plus skills served as prompts.
Decision: `vexdom mcp` runs on rmcp 3.11 (`local` feature, dual era: 2026-07-28 stateless with
`server/discover`, legacy `initialize` for 2025-11-25 and older). It lists exactly three tools:
`snapshot` (tree, markdown, image), `act` (`{"do":"<action>", ...}` or `{"batch":[...]}`) and
`guide` (status, the action index by group, one action's schema, playbooks). Skills are served by the
MCP Skills extension (`io.modelcontextprotocol/skills`), as prompts, as `skill://` resources and
through `guide`; `vexdom skills get` is gone. Side-panel runs use the same server, so `onDemand`
and `delivery` are deleted from `describe`. Byte equivalence with SDK 1.29 (ADR-008, ADR-019) no
longer applies to MCP.
Consequences: `tools/list` stays the same size whatever the extension offers; action schemas cost
tokens only when an agent asks `guide` for them.

## ADR-030: Debugger features on Chromium (amends ADR-028's out-of-scope lists)
Status: accepted (2026-11-26)
Context: the extension already holds the `debugger` permission on Chromium; agent-browser's
network, emulation, dialog, capture and devtools features need it.
Decision: one per-tab debugger hub (`packages/extension/src/lib/bridge/cdp.ts`, leases by owner)
serves every CDP feature; they are Chromium only and answer `UNSUPPORTED` on Firefox. Mouse,
trusted keys, dialogs, emulation, network routing and HAR, init scripts, tab keep, PDF, trace and
profile, video recording (frames encoded by ffmpeg in the Bridge), vitals, React, accessibility
audits, auth vault, plugins, saved flows and tab labels are in scope. axe-core (MPL) is fetched
from the npm registry at first use into the Bridge's cache, checked against the registry's
integrity hash, and never vendored.
Consequences: Chrome shows its "is debugging this browser" bar while a lease is held.

## ADR-031: Self-update and signed releases
Status: accepted (2026-11-26)
Decision: every release carries `SHA256SUMS` and `SHA256SUMS.minisig` (minisign, ed25519);
`vexdom update` and `Vexdom.app` verify the signature against the public key compiled into
`crates/vexdom-update`, then the asset's hash. A `.dmg` joins the darwin assets; `install.sh`
also installs `Vexdom.app` on macOS (`VEXDOM_NO_APP=1` skips it).
Consequences: the release workflow needs the secret `MINISIGN_SECRET_KEY`; npm self-update stays out.

## ADR-032: External confirmations
Status: accepted (2026-11-26)
Decision: a guardrail `Confirm` for a CLI or MCP caller no longer fails at once. The call waits up
to 120 s for a decision from the desktop app, `vexdom approve|deny <c-id>`, or MCP elicitation;
a timeout is `APPROVAL_TIMEOUT`, a refusal `DECLINED`. `unattended: "allow"` still allows without
asking.
Consequences (review, 2026-11-26): the approval is tied to the site it was shown for (the action
is refused with `PAGE_CHANGED` if the tab moved meanwhile); agent runs can neither list, watch nor
answer approvals, and `vexdom approve` needs a terminal. A process running as the user can still
fake a terminal and approve, so the desktop app and the MCP client's own prompt are the trusted
places to answer; the docs say so.
