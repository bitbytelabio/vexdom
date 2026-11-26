# Driving Vexdom from an MCP client

`vexdom mcp` is an MCP server over stdio. Register it with Claude Code (or any client that speaks
MCP) and that client drives the same paired browser as the side panel, through three tools. The side
panel needs none of this: it starts the same server for its own runs.

---

## Register

**Claude Code:**

```sh
claude mcp add vexdom -- vexdom mcp
```

**Another client** usually takes a block of this shape; its documentation gives the file:

```json
{
  "mcpServers": {
    "vexdom": { "command": "vexdom", "args": ["mcp"] }
  }
}
```

The command is `vexdom` with the single argument `mcp`; give the absolute path
(`~/.local/share/vexdom/bin/vexdom`, or `/Applications/Vexdom.app/Contents/MacOS/vexdom`) if the
client does not start with your shell's `PATH`.

**Clients load MCP servers when a session starts**, so restart the session after registering. A
session that was already open has no Vexdom tools; this is the most common surprise here.

The server speaks both MCP eras: the 2026-07-28 revision (stateless, with `server/discover`) and the
`initialize` handshake of 2025-11-25 and earlier, so older clients work unchanged.

---

## Three tools

`tools/list` always holds the same three tools, whatever the browser offers, so the list costs the
same few hundred tokens in every session. What an action takes is asked for only when needed.

### `snapshot`: look at the page

| Call | What comes back |
| --- | --- |
| `snapshot {}` | The active tab's accessibility tree, interactive elements only, each with a ref: `button "Sign in" [ref=e5]`. After the first call, only what changed |
| `snapshot {"interactive": false}` | Every element, not only the ones to act on. `compact`, `depth`, `selector` (one subtree), `urls` (link targets) and `full` (no delta) narrow or widen it |
| `snapshot {"as": "markdown"}` | The tab's text as markdown. `outline: true` lists the headings with line numbers, `filter` keeps the sections mentioning a phrase, `maxChars` caps it, `target` reads one element |
| `snapshot {"as": "markdown", "url": "https://…"}` | A page fetched over HTTP without touching any tab. `raw`, `requireMd`, `llms: "index"\|"full"` work as in `vexdom read <url>`; `headers` is dropped. Each URL and redirect hop is checked like a navigation, and an address on this computer or its local network answers `READ_REFUSED` unless the server was started with `VEXDOM_READ_PRIVATE=1` |
| `snapshot {"as": "image"}` | A screenshot. `fullPage`, `target` (one element), `annotate` (draws each ref on its element), `ifChanged` (skips a capture identical to the last), `save` (also keeps a file), `format`, `quality` |

`tabId` picks a tab other than the active one.

### `act`: do something

One action:

```json
{"do": "clickElement", "target": "e5"}
```

Several in order, stopping at the first failure when `bail` is true:

```json
{"batch": [{"do": "fillInput", "target": "e3", "value": "Ada"}, {"do": "submitForm", "target": "e3"}], "bail": true}
```

`do` takes an action's short name (`clickElement`, not `page.clickElement`). Everything else in the
object is that action's input. Element inputs take a ref (`"e5"` or `{"ref": "e5"}`) or a locator:
`selector`, `text`, `role` with `name`, `label`, `placeholder`, `alt`, `title`, `testId`, plus
`nth` or `last` and `exact`. Actions that change a snapshotted tab (`clickElement`, `fillInput`,
`typeText`, `insertText`, `selectOption`, `setChecked`, `pushState`, `pressKey`, `submitForm`,
`navigate`, `mouse`, `dialog`) answer with the snapshot delta they caused, so a fresh
snapshot is rarely needed.

An unknown name is refused with `UNKNOWN_ACTION` and the three closest names. A ref that is not in
the tab's latest snapshot is refused with `UNKNOWN_REF`, one from a document that has since
unloaded with `STALE_REF`, and any ref before the first snapshot with `NO_SNAPSHOT`.

### `guide`: what there is

| Call | What comes back |
| --- | --- |
| `guide {}` | Whether a browser is connected (its name, version, active tab), every action you can run, grouped, and every playbook |
| `guide {"topic": "fillInput"}` | One action: its description, whether it is read-only, its input schema |
| `guide {"topic": "network"}` | A group: its playbook, then each member's description and input |
| `guide {"topic": "dogfood"}` | A playbook's `SKILL.md`, and the reference files it has |
| `guide {"topic": "dogfood/references/issue-taxonomy.md"}` | One of those files |
| `guide {"topic": "status"}` | The connection status as JSON |

The action list comes from the extension that is connected, plus the Bridge's own actions (`diff`,
`state`, and those of [flows](features/flows.md) and the [auth vault](features/auth-vault.md)).
Groups whose actions need Chrome's debugger are marked `(Chromium only)`; a Firefox browser does
not offer those actions, so `guide` does not list them there and `act` answers `UNKNOWN_ACTION`. [Page actions](features/page-actions.md) lists them all.

---

## Playbooks

The bundled playbooks (driving the browser, navigation, input, storage, network, emulation, dialogs,
capture, devtools, WebMCP, flows, logging in, diffing and state, captchas, theming, diagnostics,
monitoring, scheduling, dogfooding a web app, deriving an API client, building WebMCP tools, Slack,
and more) reach the client four ways:

- as **prompts**: in Claude Code, `/mcp__vexdom__dogfood` and so on;
- as **resources**, `skill://<name>/SKILL.md` and `skill://<name>/references/<file>`, as markdown;
- through the MCP **Skills extension** (`io.modelcontextprotocol/skills`), for clients that load it;
- through `guide {"topic": "<name>"}`.

Start with `vexdom`: it is the loop (snapshot → act → read the delta) and says which playbook to
open for what. [Skills](features/skills.md) covers how they are written.

Three page resources are listed too: `vexdom://page/diagram` (a text diagram of the page's
regions, the cheapest useful view), `vexdom://page/current` (the full `getPageInfo` JSON) and
`vexdom://page/text` (the rendered text).

---

## Trying it from a shell

`vexdom mcp call` runs one tool through the same code a client reaches:

```sh
vexdom mcp call guide
vexdom mcp call snapshot
vexdom mcp call act '{"do":"navigate","url":"https://example.com"}'
```

Text parts are printed; an image prints as its type and size.

---

## Output size

A result longer than 20,000 characters is cut, with a note saying how to narrow the call
(`filter`, `selector`, `maxChars`…). `vexdom mcp --max-output <n>` changes the limit for one
registration, as do `VEXDOM_MAX_OUTPUT` and `maxOutput` in `./vexdom.json` or under `"cli"` in
`~/.vexdom/config.json` ([Configuration](configuration.md#cli-preferences)); `0` turns it off.

---

## How this differs from the side panel

Both reach the same browser through the same Bridge and the same three tools, so you can switch
between them mid-task. What differs is around the call:

| | Side panel | MCP client |
| --- | --- | --- |
| Agent | Claude Code, started by the Bridge | Whatever you registered |
| Actions that need approval | A card in the panel | Wait for your answer (below) |
| Host confinement | Scoped to the sites the run is about | Unconfined |
| Installing page code (`injectCode`, `runCode`) | Asks, showing the code | Refused (`external-code-injection`, `external-code-execution`) |
| Timeline | Every action, live | Actions appear in the side panel's timeline tagged **mcp** |
| Skills and site notes | Routed and applied automatically | Offered as playbooks; the client chooses |
| `authSave` | Withheld | Refused (a password typed into a tool call would sit in the transcript; use `vexdom auth save`) |

### Actions that need approval wait for you

When the [policy](approvals.md) says an action needs approval (submitting a form, uploading a file,
an action you listed in `requireApproval`…), the call waits up to 120 seconds for an answer:

- **in the client itself**, when it supports elicitation: Claude Code shows *Allow ‹action› on
  ‹site›?* with **Allow once**, **Always allow on this site** and **Deny**. On the 2026-07-28 protocol
  the server answers `input-required` and finishes the call when the client sends the answer back;
- **in the Vexdom app**, which lists it on Overview with a notification;
- **from a terminal**: `vexdom requests` lists it as `c-1a2b3c4d  page.submitForm on example.com —
  Submits a form`, `vexdom approve c-1a2b3c4d [--always]` allows it, `vexdom deny c-1a2b3c4d`
  refuses it. Approving needs a terminal, but an agent with a shell can give itself one (`script`),
  so the client's prompt or the app is the safer place to answer.

A `flowRun`'s writes are asked about the same way; `allowWrite` from a client is ignored.

A refusal answers `DECLINED`, no answer within 120 seconds `APPROVAL_TIMEOUT`, and the agent is told
not to retry either. An approval that arrives after the tab moved to another site runs nothing and
answers `PAGE_CHANGED`. An "always on this site" grant answers at once from then on.

If you would rather your client's own permission prompts be the only gate:

```json
{ "guardrails": { "unattended": "allow" } }
```

That turns every *ask* for an external caller into *allow*; it never loosens a *deny*. Read
[Approvals](approvals.md) before you set it.

### Page text arrives fenced, secrets arrive sealed

Every result carrying page-authored text is wrapped in `<<<untrusted-page-data:TAG>>>` markers
with a note that the contents are data, never instructions. It is not a proof against prompt
injection; see [Limits](limits.md#prompt-injection-is-a-real-risk). Passwords, keys, tokens and
cookie values come back as sealed placeholders such as `⟦password:4f2a@example.com⟧`; passed
unchanged as the `value` of `fillInput` or the `text` of `typeText`, a placeholder becomes the real
value in the browser. See [Approvals § Secrets](approvals.md#secrets-are-sealed-before-you-are-asked).

### Sites that publish their own tools

After a navigation or snapshot, a page that registers [WebMCP](features/webmcp.md) tools gets a
short block appended to the result listing them, with the `act {"do":"callSiteTool", …}` line to
call each. Their names and descriptions are the site's own text, fenced like any page content.

---

## Several clients at once

Clients share one Bridge and, unless several browsers are connected, one browser, so their calls
interleave. Each call gets its own result, but page state can shift under either client. The side
panel can be running at the same time.

---

## See also

- [Approvals](approvals.md): the full policy
- [Page actions](features/page-actions.md): every action by group
- [Troubleshooting](troubleshooting.md#mcp-clients): tools missing from a session, and more
- [internals/request-path.md](../internals/request-path.md): an `act` call end to end
