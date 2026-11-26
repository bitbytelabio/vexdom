# Network

Mock or block a tab's requests by URL pattern, record the tab's traffic as a HAR file, and read the
requests a page made. Chromium only.

```sh
vexdom network route '*api/items*' --body '{"items":[]}'
vexdom network har start
… use the page …
vexdom network har stop /tmp/session.har
vexdom network requests --status 4xx
```

---

## Routes: mock or block

A route answers or fails every request of the tab whose URL matches a pattern, until you remove it.

| CLI | MCP |
| --- | --- |
| `vexdom network route <pattern> --abort` | `act {"do":"routeNetwork","pattern":"…","abort":true}` |
| `vexdom network route <pattern> --body <json\|@file> [--status N] [--content-type T] [--allow-origin O] [--resource-type csv]` | `act {"do":"routeNetwork","pattern":"…","response":{"status":200,"body":"…","contentType":"…","headers":{…},"allowOrigin":"…"},"resourceTypes":["XHR","Fetch"]}` |
| `vexdom network unroute [pattern]` | `act {"do":"unrouteNetwork","pattern":"…"}` (no pattern: every route of the tab) |

How a pattern matches:

- `*` is a wildcard, and runs of `*` count as one.
- A pattern with no `*` matches any URL that **contains** it (`/api/`).
- A pattern with `*` is anchored at whichever ends have no `*`: `https://a.test/*` must start with
  `https://a.test/`.

Routes apply in the order they were added; the first one whose pattern and `resourceTypes` match
wins. A request no route matches goes through untouched.

A mocked response defaults to status 200. A body that parses as JSON gets
`Content-Type: application/json` unless you set one. Cross-origin reads of a mock are allowed only
for the one page origin named in `allowOrigin`; for it the route adds the CORS headers and answers
preflights with 204.

Each route needs `abort: true` or a `response`; one with neither is refused with `INVALID_INPUT`.
The result is `{"routed": "<pattern>"}`, and removing gives `{"unrouted": "<pattern>"|"all"}`.

Routes live in the extension and are per tab. Closing the tab, Chrome taking the debugger away, or
an extension restart drops them.

---

## HAR recording

| CLI | MCP |
| --- | --- |
| `vexdom network har start [--content all\|text\|none]` | `act {"do":"harStart","content":"text"}` |
| `vexdom network har stop [path]` | `act {"do":"harStop","path":"/abs/file.har"}` or `{"do":"harStop","inline":true}` |

`content` decides which response bodies are kept: `text` (the default: JSON, HTML, scripts and
other text types), `all`, or `none` (sizes and types only). One body is kept up to 2 MiB, and 32 MiB
across the recording; at most 5,000 entries.

`harStop` writes a HAR 1.2 file through the Bridge: to the absolute `path` you give, or to
`~/vexdom/captures/har-<unix_ms>.har`. The CLI resolves a relative path against your working
directory. The answer is `{requestCount, path, bytes}`. `inline: true` returns the HAR itself
instead of writing a file.

| Code | When |
| --- | --- |
| `HAR_ACTIVE` | A recording is already running on this tab |
| `NO_HAR_RECORDING` | `harStop` with nothing recording |
| `ARTIFACT_TOO_LARGE` | The file would exceed 48 MiB |
| `INVALID_INPUT` | `path` is relative, or inside the Bridge's own folder (`~/.vexdom`) |

The HAR holds what the page sent and received, cookies and tokens included, because it is written to
your disk rather than handed to an agent. The `derive-client` playbook builds on it: record a HAR
while using a site, then write a client for its internal API.

---

## Reading requests

`vexdom network requests` and `vexdom network request <id>` read the [diagnostics](diagnostics.md)
recording (`readNetwork`). The first call on a tab with no recording starts one and says so:
"Capture started on this tab; nothing was recorded before now."

| CLI | MCP (`readNetwork` input) |
| --- | --- |
| `--filter s` | `urlContains` |
| `--type csv` | `resourceTypes`, e.g. `["xhr","fetch"]` |
| `--method m` | `method` |
| `--status s` | `statusCode`: exact `404`, class `4xx`, or range `200-299` |
| `--clear` | `drain: true` (forget what was returned) |
| `network request <id>` | `requestId` with `includeHeaders` and `includeBodies` |

Over MCP `readNetwork` also takes `status` (`all`, `problems`, `failed`, `pending`) and `limit`
(default 50).

Response bodies are denied by the `network-body-read` guardrail, so `vexdom network request <id>`
fails with `BLOCKED` until you allow it in [Approvals](../approvals.md). Headers and URLs pass
through the same secret sealing as every result.

`vexdom console [--level L] [--contains s] [--limit n] [--clear]` and `vexdom errors` read the
console the same way; see [Diagnostics](diagnostics.md).

---

## The debugger

Routes and HAR recording each hold a lease on the tab's one debugger attachment (owners `route` and
`har`), shared with every other debugger feature. Chrome shows "Vexdom is debugging this browser"
while any lease is held: until `unrouteNetwork` removes the last route, or `harStop`.

| Code | When |
| --- | --- |
| `UNSUPPORTED` | Firefox: "Routing requests needs Chrome's debugger, which Firefox does not offer to extensions." |
| `DEBUGGER_UNAVAILABLE` | DevTools is open on the tab, or Chrome refused to attach |

---

## The playbook

The `network` playbook (`crates/vexdom-core/assets/skills/network/SKILL.md`) teaches an agent when to
route, record or read. MCP clients get it as the prompt `network`, the resource
`skill://network/SKILL.md`, and through `guide {"topic":"network"}`.

---

## See also

- [Diagnostics](diagnostics.md): console and network reads behind `vexdom console` and `vexdom network requests`
- [Emulation](emulation.md): offline mode and extra request headers
- [Approvals](../approvals.md): the `network-body-read` rule
- [reference/errors.md](../../reference/errors.md)
