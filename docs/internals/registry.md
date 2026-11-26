# The action registry

How browser actions are defined once in the extension, how the Bridge learns them without running
any TypeScript, how names resolve, and how a build whose two halves disagree is detected.

---

## One registry in the extension

Every page action is a module in `packages/extension/src/lib/actions/page/`, made with
`defineAction` (`packages/extension/src/lib/actions/core.ts`): a name, a description, a zod input
schema, an `execute()`, and optionally `group` and `chromiumOnly`.

```ts
export const clickElement = defineAction({
  name: 'page.clickElement',
  description: '…',
  input: z.object({ target: targetSchema.describe('…') }),
  execute({ target }) { /* runs in the page */ },
});
```

`packages/extension/src/lib/actions/registry.ts` lists them all in one `actions` map (84 today).
`describeActions(target)` turns it into tool descriptors (`{name, description, inputSchema}`, the
zod schema as JSON Schema with zod's own noise cut), leaving out `chromiumOnly` actions for the
Firefox build. `describeOwnActions()` (`bridge/own-actions.ts`) picks the target from the build.

`execute()` runs in the content script for actions that act on the page. Many actions never reach
it: the background answers them itself (tabs, files, monitors, the debugger features); see
[Inside the extension](extension.md).

### Groups

An action that only some tasks need names a `group`. `packages/extension/src/lib/actions/groups.ts`
(`TOOL_GROUPS`) gives each group its line in the guide's index and the skills that teach it:
`captcha`, `theming`, `diagnostics`, `monitoring`, `downloads`, `drag`, `storage`, `navigation`,
`input`, `code`, `network`, `dialogs`, `emulation`, `capture`, `devtools`. Ungrouped actions are
the core. The MCP `guide` tool lists actions by group and opens a group's guide by name.

### `chromiumOnly`

Actions that need Chrome's debugger are marked `chromiumOnly` (26 today: `trustedClick`, the two
captcha actions, `mouse`, `initScript`, `injectCode`, `runCode`, the four diagnostics actions,
`routeNetwork`, `unrouteNetwork`, `harStart`, `harStop`, `dialog`, `emulate`, `pdf`,
`recordStart`, `recordStop`, `traceStart`, `traceStop`, `vitals`, `react`, `a11yAudit`,
`tabKeep`). The Firefox build does not list them. They stay registered there, so a call that
names one gets `UNSUPPORTED` with a hint instead of `UNKNOWN_ACTION`.

---

## The manifest the Bridge embeds

The Bridge is Rust and does not run the registry. It embeds the registry's output instead:

```sh
cd packages/extension
bun run tools:emit
```

`scripts/emit-tools.ts` bundles `src/lib/actions/emit.ts` and writes
`crates/vexdom-core/assets/tools.chromium.json` and `tools.firefox.json`. Each holds:

| Key | What |
| --- | --- |
| `hash` | The manifest hash (`hashManifest()` in `actions/manifest.ts`: FNV-1a over the descriptors sorted by name). Rust never computes it |
| `tools` | The descriptors, in registry order |
| `groups` | Action → group |
| `toolGroups` | `TOOL_GROUPS` as declared |
| `chromiumOnly` | The Chromium-only action names |

`crates/vexdom-core/src/tools.rs` embeds both with `include_str!` (`chromium()`, `firefox()`,
`bundled(hash)`). `src/lib/actions/registry.test.ts` fails when either file differs from what the
registry would emit now, so a registry change without `bun run tools:emit` does not pass the
checks.

### Bridge actions

`crates/vexdom-core/assets/bridge-actions.json` (`tools::bridge()`) lists the actions no browser
runs, named `bridge.<name>`: today `bridge.diff`, `bridge.state` and `bridge.authSave`,
`bridge.authLogin`, `bridge.authList`, `bridge.authShow`, `bridge.authDelete`. They are offered
next to the extension's. `bridge.diff` and `bridge.state` run in the caller's own process
(`app/src/cli/diff.rs`, `app/src/cli/state.rs`), because they read and write files the caller
names; the `bridge.auth*` actions are answered in the Bridge by `crates/vexdom-daemon/src/auth_vault.rs`
(see [Auth vault](../guide/features/auth-vault.md)). The MCP server never runs `bridge.authSave`
or `page.checkSites` for anyone, and an agent run is refused `bridge.authSave`.

`vexdom tools` prints the manifest.

---

## Names

Actions are named `page.<name>` or `bridge.<name>` in the manifest. Agents call them by the short
name: `act {"do":"clickElement", …}`. `resolve_action()` in `tools.rs` accepts the full name, the
short name, and the forms older catalogs and MCP clients showed (`page_clickElement`,
`mcp__vexdom__page_clickElement`), trying `page.`, then `bridge.`, then the reserved names.
`short_name()` goes the other way.

An unknown name gets `UNKNOWN_ACTION` with the three closest names; `guide {"topic":"<name>"}` gives
one action's description, whether it only reads (`READ_ONLY_ACTIONS`), and its input schema with
the shared arguments trimmed.

No action may declare an input called `do` or `batch`, because `act` reads those itself; a test in
`tools.rs` checks every manifest.

---

## What a caller is offered

`Control.describe` answers with the tools this caller may run (`describe_for()` in
`crates/vexdom-daemon/src/lib.rs`):

- **Outside a run**: the tools of the browser the next call would reach (its link's list, or the
  bundled Chromium list when no browser is connected) plus the Bridge actions, minus
  `page.snapshot` and `page.readHtml` (`DAEMON_INTERNAL_ACTIONS`, which the Bridge calls itself for
  `snapshot` and `read`), and minus `injectCode` and `runCode` when the guardrail policy denies
  them.
- **Inside a run**: that run's offer, with the actions it withholds listed separately and the
  reserved actions it may use.

The MCP server builds `act`'s list from that answer (`Host::offer()` in `app/src/mcp/host.rs`).
`tools/list` itself always shows the same three tools, whatever the browser offers.

---

## Reserved actions

Six names under the `vexdom.` prefix are not page actions (`RESERVED_ACTIONS` in `tools.rs`):

| Action | Used by |
| --- | --- |
| `vexdom.saveSiteMap` | Claude Code at the end of a mapping run; offered only inside a run |
| `vexdom.focusShot` | Claude Code, for the A-Eye pick attached to the instruction; offered only inside a run |
| `vexdom.startRecording`, `vexdom.stopRecording` | The extension's fast path, on the user's own words (`bridge/fast-path.ts`) |
| `vexdom.readSitemap`, `vexdom.readFile` | The Bridge's agent runner, as timeline entries while it fetches a sitemap or reads an attached file |

`invoke_on()` in `crates/vexdom-daemon/src/invoke.rs` refuses any action starting with `vexdom.`
with `UNKNOWN_ACTION`, so an outside client cannot reach them. A run's two are answered by the
run itself (`AgentSession::invoke_for_run`).

---

## Drift

The extension sends its manifest hash in `hello`. When the hash matches one of the two embedded
manifests, the link serves that list and the Bridge reports `manifestInSync: true`. When it
matches neither (an extension and a Bridge built from different registries), the Bridge:

1. reports `manifestInSync: false` in the `welcome` and in `vexdom status`, and logs the link as
   `DRIFTED`;
2. asks the extension for its own list (`Page.describe`) and **adopts it**, because the browser is
   the authority on what it can run; if that fails, it keeps serving the bundled list;
3. tells `manifestChanged` watchers when the list a caller would be offered changes.

`guide` then carries a hint that the two halves are different versions. The fix is to update or
rebuild both (see [guide/maintenance.md](../guide/maintenance.md)).

---

## Next

**[Request path →](request-path.md)**: how a call reaches the page.

Adding an action: [Contributing](contributing.md).
