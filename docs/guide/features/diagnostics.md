# Diagnostics

Record a page's console messages, uncaught exceptions and network requests while you reproduce a
problem, so the agent can tell you why the page broke as well as what it shows. Chromium only.

---

## The short version

From the CLI:

```sh
vexdom errors              # first call starts recording on this tab
… reproduce the problem …
vexdom errors              # uncaught exceptions and console.error
vexdom network requests --status 4xx
```

Over MCP:

```
act {"do":"startDiagnostics","reload":true}
… reproduce the problem …
act {"do":"readConsole","level":"error"}
act {"do":"readNetwork","status":"problems"}
act {"do":"stopDiagnostics"}
```

---

## Why it is a session and not a read

`Runtime.consoleAPICalled` and `Network.responseReceived` are delivered **only while Chrome's
debugger is attached**, and no backlog is kept. So diagnostics works like
[monitoring](monitoring.md), with start, read and stop: **start it before the thing you are
diagnosing happens.**

`reload: true` covers the case where the failure is the load: it reloads once recording has started.

The CLI reads (`vexdom console`, `vexdom errors`, `vexdom network requests`,
`vexdom network request <id>`) start a recording themselves when the tab has none, answering
"Capture started on this tab; nothing was recorded before now. Run the command again after the page
does something."

| CLI | MCP |
| --- | --- |
| first `console`/`errors`/`network requests` | `startDiagnostics {capture, reload, tabId, timeoutMs}` |
| `vexdom console [--level L] [--contains s] [--limit n] [--clear]` | `readConsole {level, contains, limit, drain}` |
| `vexdom errors [--clear]` | `readConsole {"level":"error"}` |
| `vexdom network requests …`, `network request <id>` | `readNetwork` ([Network](network.md#reading-requests)) |
| | `stopDiagnostics {diagnosticsId}` |

---

## The bar across your browser

While a recording runs, Chrome shows **"Vexdom is debugging this browser"** with a **Cancel**
button. The recording holds a `diagnostics:<id>` lease on the tab's one debugger attachment, shared
with the other debugger features, so the bar stays while any of them holds one.

| | |
| --- | --- |
| **A timeout** | `timeoutMs`, five minutes by default, thirty at most. It detaches on its own |
| **The turn** | Inside a side-panel conversation, a recording ends when the turn that started it ends |
| **Cancel** | Detaches at once; what was collected stays readable |

An MCP client or the CLI has no turn to end, so its recordings last until `stopDiagnostics` or the
timeout. That lets one recording span several calls.

---

## What comes back

**`readConsole`** returns the level, the text, the file and line that logged it, and a stack for
errors. Kinds: `console`, `exception` (uncaught) and `browser` (Chrome's own reports: CSP
violations, mixed content, failed loads). `level` is the lowest level reported (`all`, `debug`,
`info`, `warn`, `error`); `limit` defaults to 50.

**`readNetwork`** returns the method, URL, status, resource type, timing, size, and the browser's
error text for failed requests. `status: "problems"` is everything that failed or came back 4xx/5xx;
`"failed"` the requests that never completed; `"pending"` those with no response yet. It also filters
by `urlContains`, `method`, `statusCode`, `resourceTypes` and `requestId`.

`drain: true` forgets what was returned, so the next call reports only what happened since.

500 console entries and 1,000 requests are kept per recording; past that the oldest are dropped, and
every read reports `droppedConsole` and `droppedNetwork`.

---

## What network data reaches the agent

| | |
| --- | --- |
| **Metadata** (method, URL, status, type, timing, size) | Free, like any read |
| **Headers** | Off by default; `includeHeaders: true` adds them |
| **Bodies** | `includeBodies: true` fetches the bodies of the 5 most recent requests returned, cut to 4,000 characters. **Denied by default** by the `network-body-read` rule (`BLOCKED`) |

Everything returned passes through the same secret sealing as every other result: a cookie, bearer
token or API key becomes a placeholder such as `⟦token:4f2a@example.com⟧`. To allow bodies:

```json
{ "guardrails": { "rules": { "network-body-read": "allow" } } }
```

or set the row in the side panel's guardrail settings. Bodies exist only while the recording is
attached, so a `stopDiagnostics` before the read means no bodies whatever the policy.

---

## Edges

| Code | When |
| --- | --- |
| `UNSUPPORTED` | Firefox, or a page that is not http(s) |
| `DEBUGGER_UNAVAILABLE` | DevTools is open on that tab; close it and retry |
| `DIAGNOSTICS_LIMIT` | Two tabs are already being recorded |
| `DIAGNOSTICS_NOT_FOUND` | No recording by that id, or nothing is being recorded |
| `INVALID_TARGET` | Several recordings run and no `diagnosticsId` was given |

Only the top-level frame's console is collected.

---

## The playbook

The `page-diagnostics` playbook (`crates/vexdom-core/assets/skills/page-diagnostics/SKILL.md`) is
a side-panel base skill and is served over MCP as the prompt `page-diagnostics`, the resource
`skill://page-diagnostics/SKILL.md`, and through `guide {"topic":"diagnostics"}`.

---

## See also

- [Network](network.md): routes, HAR files, and the request filters in detail
- [DevTools](devtools.md): performance, React and accessibility
- [Approvals](../approvals.md): the `network-body-read` rule
- [reference/errors.md](../../reference/errors.md)
