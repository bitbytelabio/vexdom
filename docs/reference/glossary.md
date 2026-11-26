# Glossary

Each domain term of the original, what it means, and its name in Rust. Units add to this as they
bring new terms; a Rust name marked *(planned)* is not written yet.

| Term | Meaning | Original | Rust |
| --- | --- | --- | --- |
| Bridge / daemon | The long-lived local process that owns the browser link, authorization, agent runs and files | `src/daemon/daemon.ts` `startDaemon` | `vexdom daemon` *(planned)* |
| Extension link | One authenticated WebSocket from a browser's extension (`/extension`) | `extension-link.ts` `ExtensionLink` | `ExtensionLink` *(planned)* |
| Control client | A local caller (CLI, MCP server, desktop app, native host) on `/control`, Bearer token from the lockfile | `control.ts`, `remote-bridge.ts` | `ControlClient` *(planned)* |
| Lockfile | `~/.vexdom/daemon.json`: pid, port, token, protocol and daemon version | `lockfile.ts` | `Lockfile` *(planned)* |
| Wake hold | The `stopped` file a `stop` leaves so the browser does not restart the daemon | `lockfile.ts` `releaseWake` | *(planned)* |
| Handshake | hello → challenge → prove → welcome; HMAC-SHA256 proofs over a `\|`-joined transcript | `packages/extension/src/lib/actions/handshake.ts` | *(planned)* |
| Pairing code | 8 chars of `ABCDEFGHJKMNPQRSTWXYZ23456789`, 10 min, PBKDF2-SHA256 250k iterations | `auth-store.ts` | *(planned)* |
| Grant | A 26-char, 120 s pairing secret handed over by the native host | `pairing-desk.ts` | *(planned)* |
| Session key | 32 random bytes per paired browser, sealed to it in `welcome` | `auth-store.ts` | *(planned)* |
| Pairing mode | `auto` / `ask` / `code`; ask mode keeps pending requests for an approver | `packages/extension/src/lib/pairing.ts` | *(planned)* |
| Approver | A control client that decides ask-mode requests (`app` or `terminal`) | `daemon.ts`, `pairing-desk.ts` | *(planned)* |
| Native host | Native-messaging process the extension launches to wake and pair; 4-byte LE length + JSON | `native-messaging.ts`, `native-host.ts` | `vexdom native-host` *(planned)* |
| Action | A page capability with a zod input, executed by the extension (`page.clickElement`) | `packages/extension/src/lib/actions/page/*.ts` | tool descriptor JSON |
| Tool descriptor | `{name, description, inputSchema}` derived from an action; MCP tool `page_clickElement` | `packages/extension/src/lib/actions/registry.ts` | *(planned)* |
| Manifest hash | FNV-1a-32 over UTF-16 code units of the sorted descriptor JSON | `packages/extension/src/lib/actions/manifest.ts` | *(planned)* |
| Skill group | A lazily offered tool group (captcha, theming, diagnostics, monitoring, downloads, drag) opened with `vexdom_openSkill` | `packages/extension/src/lib/actions/groups.ts`, `tool-groups.ts` | *(planned)* |
| Run | One side-panel instruction executed by a spawned agent CLI; emits `RunEvent`s | `agent/service.ts` | *(planned)* |
| Runner | The adapter for one agent CLI: plan, reader, limits | `agent/runners/*.ts` | *(planned)* |
| Containment | Per-runner rules a spawn plan must keep (flags, deny lists, env, workspace) | `guardrails/spawn.ts` | *(planned)* |
| Sealed env | The parent env minus credential-shaped names not owned by the agent | `guardrails/spawn.ts` `sealEnv` | *(planned)* |
| Policy / decision | Rules mapping (action, caller, scope) to allow / confirm / deny | `guardrails/policy.ts`, `decide.ts` | *(planned)* |
| Scope | Hosts a run may act on: from URL, instruction, tabs, config | `guardrails/scope.ts` | *(planned)* |
| Standing approval | "Always on this site" approvals in `approvals.json` (max 200) | `agent/approvals.ts` | *(planned)* |
| Unattended | The policy external (MCP) callers get for `confirm` decisions (default deny) | `daemon.ts` `invokeExternal` | *(planned)* |
| Seal / placeholder | A secret replaced by `⟦kind:id@host#tag⟧`; only the extension turns it back | `guardrails/secrets.ts`, `packages/extension/src/lib/secrets` | *(planned)* |
| Fence | Untrusted page text wrapped in `<<<untrusted-page-data:TAG>>>` with a note | `guardrails/fence.ts` | *(planned)* |
| Live tools | `injectCode` / `runCode`, off unless the user switches them on | `agent/service.ts` | *(planned)* |
| Site tools | WebMCP tools a page declares (`listSiteTools`, `callSiteTool`) | `packages/extension/src/lib/bridge/site-tools.ts` | extension (TS) |
| Site map | A per-site skill drafted by a mapping run from robots.txt/sitemaps | `agent/sitemap.ts`, `site-map-store.ts` | *(planned)* |
| A-Eye pick / focus shot | An element the user pointed at, with its screenshot, attached to an instruction | `packages/extension/src/lib/bridge/aeye.ts`, `tool-host.ts` | *(planned)* |
| Task / schedule | A scheduled instruction the daemon starts through a browser link | `schedules/` | *(planned)* |
| Notice | A device notification relayed to a watching control client | `notice-desk.ts`, `packages/extension/src/lib/notices.ts` | *(planned)* |
| Snapshot | Compact accessibility text of a page, one line per node | new (agent-browser `snapshot.rs`) | `vexdom_snapshot::render` *(planned)* |
| Ref | `e{n}`, a per-document durable element id from the content script | new | `vexdom_snapshot::Ref` *(planned)* |
| Baseline | The last snapshot of a page, kept (≤32) to compute a delta | new (agent-browser `actions.rs:8704`) | *(planned)* |
| Delta | Changes since the baseline: revision + add/remove/replace lines | new | *(planned)* |
