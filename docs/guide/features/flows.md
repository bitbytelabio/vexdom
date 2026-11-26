# Saved flows

Record a task once while an agent (or you, through `vexdom`) does it in the browser, save the site's
own API requests behind it as a named flow, and rerun that flow later as direct requests from the
tab instead of clicking through the page again. Recording needs Chromium.

```sh
vexdom flow record start
vexdom open https://shop.example/        # … do the task with ordinary page commands …
vexdom flow record stop
vexdom flow steps
vexdom flow save search-items --param q=shoes --description "Search the catalog"
vexdom flow run search-items --param q=boots --fields name,price --limit 5
```

---

## Commands

| CLI | MCP |
| --- | --- |
| `vexdom flow record start\|stop\|status` | `act {"do":"flowRecord","op":"start"}` (`"stop"`, `"status"`; default `status`) |
| `vexdom flow steps` | `act {"do":"flowSteps"}` |
| `vexdom flow save <name> [--from n] [--to n] [--param k=v]... [--output id] [--description t] [--timeout ms]` | `act {"do":"flowSave","name":"…","from":1,"to":4,"params":{"q":"shoes"},"output":"s2r1","description":"…","timeoutMs":25000}` |
| `vexdom flow run <name> [--param k=v]... [--origin h] [--allow-write all\|id,id] [--where f=v]... [--fields a,b] [--limit n] [--timeout ms]` | `act {"do":"flowRun","name":"…","params":{…},"origin":"…","allowWrite":"all","where":{…},"fields":["a","b"],"limit":5,"timeoutMs":25000}` |
| `vexdom flow list [--origin h]` | `act {"do":"flowList","origin":"…"}` |

The Bridge answers all of them (`bridge.flowRecord`, `bridge.flowSteps`, `bridge.flowSave`,
`bridge.flowRun`, `bridge.flowList`). Every call but `flowList` also takes `tabId` (MCP only);
without it, the active tab (or the active tab's recording). `flowSteps` and `flowList` are
read-only. `--json` prints the `{"ok":true,"data":…}` envelope instead of the text shown below.

---

## Recording

`flow record start` starts a HAR recording on the tab (`page.harStart` with `content: "text"`, the
same recorder as [Network](network.md#har-recording)), so Chrome shows its debugging bar until the
recording stops. While it runs, the Bridge also logs each page action a control client (the
`vexdom` CLI or an MCP client) runs on that tab, with when it started and ended. A request belongs to
the action whose window, from its start to 1.5 s after its end, holds the request's start, unless
another action began since.

What becomes a step:

- page actions sent through the Bridge, not clicks you make by hand in the browser: traffic with no
  action to attach to belongs to no step and is never saved;
- actions that change the page, plus `waitFor` and `waitForElement`. Reads (`snapshot`, `query`,
  `readNetwork` …) and the capture, setup and inspection actions (`harStart`, `cookies`, `storage`,
  `emulate`, `routeNetwork`, `initScript`, `screenshot`, `pdf`, `traceStart`, `recordStart`,
  `tabKeep` and similar) are left out.

| Op | Result |
| --- | --- |
| `start` | `{"started":true,"tabId":12,"url":"https://shop.example/"}` |
| `stop` | `{"stopped":true,"tabId":12,"steps":5,"requests":38,"unsaved":5}` |
| `status` | `{"recordings":[{"tabId":12,"capturing":true,"steps":5,"requests":38,"unsaved":5,"startedAt":"…"}]}` |

The CLI prints these results as JSON on one line.

Starting on a tab whose recording is still running fails with `HAR_ACTIVE`; starting again after a
stop replaces the tab's previous recording, saved or not. `stop` or `steps` with no recording on the
tab answers `FLOW_NOT_CAPTURING`. Recordings live in the Bridge's memory, so a Bridge restart loses
what was not saved.

On Firefox `flow record start` answers `UNSUPPORTED`: HAR recording needs Chrome's debugger
(ADR-030).

### `flow steps`

```text
  1 ok     navigate https://shop.example/  https://shop.example/  requests: 3
  2 ok     fillInput Search  https://shop.example/  requests: 0
  3 ok     pressKey Enter  https://shop.example/search?q=shoes  requests: 4
Last save covered step 0; 3 step(s) not saved.
```

Each line is the step number, `ok` or `FAILED`, the command (action plus what it names), the page
it ended on, how many requests it is credited with, and `(saved)` once a save covered it.

---

## Saving

`flow save <name>` turns a range of the recording into a flow file. It works while the recording
runs (it first waits up to `--timeout` ms, default 25000, for the page to go quiet and reads the HAR
so far) or after it stopped. A name is letters, digits, `-` and `_`.

- `--from` / `--to` pick the steps. Without `--from`, the save starts at the last `navigate` after the
  previous save (else right after it); without `--to`, at the last step.
- The kept requests are first-party (the start page's base domain), `XHR` or `Fetch`, JSON, status
  2xx, and not telemetry (`/collect`, `/track`, `/beacon`, `/analytics`, `/pixel`, `/telemetry`,
  `/metrics`, `/log`, `/rum`, `/sentry`). For server-rendered sites, a JSON block embedded in a
  first-party HTML document (`__NEXT_DATA__`, JSON-LD) also counts. Each gets an id `s<step>r<n>`.
- `--output <id>` names the request whose response the flow answers with. The default is the largest
  response among requests that use a param, else the largest.
- `--param k=v` turns the value `v`, as the recording sent it, into the param `{{k}}`. Without it,
  values typed into ordinary fields become params named after the field, and query values of the
  start URL that the requests also sent become params. A value an earlier response supplied is kept
  as a reference (`{{@s1r1.token}}`) and filled at run time.
- `--description` is what `flow list` shows; without it, "Runs the task recorded on ‹start URL›".

```text
Saved flow 'search-items' for shop.example to /Users/ada/.vexdom/flows/shop.example/search-items.json
    s1r1 GET https://shop.example/api/session (812 bytes)
  * s3r1 GET https://shop.example/api/search?q={{q}} (5120 bytes)
  * = output. Change it with `flow save --output <id>`.
Params: q
Not captured:
  step 2 `fillInput Search`: no network traffic while this command ran
Not saved (taken from the live session at run time): cookie, authorization
Unverified until the first successful `flow run`.
```

The JSON report holds `name`, `origin`, `path`, `from`, `to`, `steps`, `requests` (`id`, `method`,
`url`, `bytes`), `output`, `params`, `detectedParams`, `references`, `writes`, `gaps`
(`step`, `command`, `reason`), `droppedHeaders` and `verified: false`.

A save fails with `ACTION_FAILED` when nothing qualifies ("no first-party JSON requests were
recorded for ‹site›; this flow has nothing to replay"), when the range is outside the recorded steps,
or when `--output` names an unknown id.

### What is never saved

- Headers `cookie`, `authorization`, `proxy-authorization`, and any header whose name contains
  `token` or `csrf`. A run gets them from the live session in the browser.
- Values typed into a field that looks like a password or a code, sealed handles (`⟦kind:id#tag⟧`),
  and the password an [auth vault](auth-vault.md) login typed during the recording. Requests that
  carry one are dropped and listed as a gap; the step is marked redacted and never replayed.
- Credential query parameters (`token`, `password`, `apikey`, `signature`, `jwt` and similar) in the
  pages the browser visited: stored as `<redacted>`.

---

## Storage

One file per flow at `~/.vexdom/flows/<host>/<name>.json`, where `<host>` is the start page's host.
Files are written mode `0600` in `0700` folders, because they hold recorded URLs and form values.
`VEXDOM_HOME` moves them with the rest of `~/.vexdom`.

The file is JSON, `version: 1`, and an unknown field is an error. It records `name`, `origin`,
`startUrl`, `allowedHosts`, `params` (each `name`, `example`, `source`: `typed`, `query` or
`response`), `output`, `itemFields`, `writes`, `verifiedAt` and the `steps` with their commands and
requests (method, URL template, headers, body, the recorded response's status, MIME type and shape).
A run refuses a file whose folder, `origin` and start URL disagree, whose version is not 1, or that
sends a write it does not list ("re-save it").

### `flow list`

```text
search-items (shop.example) params: q verified
```

`--origin <host>` keeps flows on that site's base domain. The JSON form gives, per flow: `name`,
`origin`, `valid`, `params`, `examples`, `description`, `inputSchema` (a JSON Schema of the params,
the shape WebMCP tools declare), `paramSources`, `itemFields`, `recordedOn`, `provenance` and
`verifiedAt`. With no flows: "No saved flows. Run `vexdom flow record start`, do the task, then
`vexdom flow save <name>`."

---

## Running

`flow run <name>` sends only the output request and the requests it takes values from, in order,
from the tab. Each request goes through two tiers:

1. **page**: the tab's own `fetch` (through `page.evaluate`), with the browser's cookies. The tab
   must already be on the flow's site; redirects are not followed. In Chrome this needs **Allow
   User Scripts** ([Install](../install.md)).
2. **replay**: the recorded commands run again in the tab (only `navigate`, clicks, hover, focus,
   fill, type, keys, select, check, submit, scroll and waits, and only to URLs on the flow's site),
   and the answer is read from a HAR of what they sent. Firefox has no HAR, so there only the page
   tier works.

There is no plain-HTTP tier: cookie values never leave the browser, so a request goes out from the
page or not at all. A request that worked on the replay tier starts there next time.

| Flag | Input | Effect |
| --- | --- | --- |
| `--param k=v` | `params` | Values for the params; one left out uses its recorded value. A param typed during the recording has none and must be passed. An unknown name is `INVALID_INPUT` |
| `--origin h` | `origin` | Picks the site when flows on several sites share the name |
| `--allow-write all\|id,id` | `allowWrite` | Writes this run may send without asking (below). Ignored from MCP clients and side-panel runs |
| `--where f=v` | `where` | Keeps list items whose field contains the text, case-insensitive |
| `--fields a,b` | `fields` | Keeps only these fields of each item; dotted paths work |
| `--limit n` | `limit` | At most `n` items |
| `--timeout ms` | `timeoutMs` | The whole run, default 25000 |

`where`, `fields` and `limit` apply to the output's main list (the body if it is an array, else its
heaviest list of objects). A field no item has fails with `INVALID_INPUT`, naming the fields there
are.

Every URL is checked before it is sent: it must be `http(s)`, on the start page's base domain (and
inside `allowedHosts`), not a link-local or cloud-metadata address, and not on your
[blocked sites](blocked-sites.md).

```text
Flow 'search-items' ran in 412ms (s3r1 via page in 388ms), verified
{
  "list": "results",
  "total": 48,
  "matched": 48,
  "items": [ … ]
}
```

A request that fell back adds `after page: ‹reason›`. The status reads `unverified: ‹reason› on the
‹tier› tier` when a response changed shape, or `no results, so the flow's verified state is
unchanged` when the answer was empty.

The JSON result:

```json
{
  "name": "search-items",
  "origin": "shop.example",
  "ok": true,
  "verified": true,
  "ms": 412,
  "steps": [{ "id": "s3r1", "step": 3, "tier": "page", "status": 200, "ms": 388, "error": null }],
  "data": { "list": "results", "total": 48, "matched": 48, "items": [] }
}
```

`data` is the output response as is when no `where`, `fields` or `limit` was given. A step that
fell back carries `fellBack: [{"tier","error"}]`. `empty: true`, `drift: {"tier","step","reason"}`
and `saveError` (the flow file could not be updated) appear when they apply. A run that returns data
with the recorded shape sets the file's `verifiedAt`; one whose response changed shape clears it.

---

## Writes

A saved request is a write when its method is not `GET`, `HEAD` or `OPTIONS` (a `POST` that only
runs GraphQL queries is a read), or, marked `suspected`, when it is a first-party `GET` whose path
has a segment `logout`, `signout`, `unsubscribe`, `delete` or `remove`. The save report lists every
write with its `id` (the request's own, or `s<step>w<n>` for one only the replay sends), `method`,
`url`, `kind` and `step`.

A write goes out only when `--allow-write` covers it (`all`, or its id), or when you approve it.
An MCP client's or a side-panel run's `allowWrite` is dropped before the run starts, so every write
an agent's run would send asks you.

- Before the run starts, each write the run would send waits for approval. When the replay tier
  starts, so does each write only the replay would send.
- The prompt reads `flow '‹name›' sends ‹METHOD› ‹url›` for the action `bridge.flowRun` on the flow's
  origin. Answer it in the Vexdom app or with `vexdom requests`, `vexdom approve c-… [--always]`,
  `vexdom deny c-…` ([Approvals](../approvals.md#from-the-cli-or-an-mcp-client)), or, when an MCP
  client ran the flow, in the client's own approval prompt (elicitation). It waits 120 s.
  `guardrails.unattended: "allow"` does not skip it.
- `--always` (**Always on ‹site›**) grants `bridge.flowRun` on that host, so later runs of any flow
  there send writes without asking. `vexdom approvals clear <host>` takes it back.
- A refusal fails the run with `DECLINED`, no answer with `APPROVAL_TIMEOUT`.
- `--allow-write` naming an id that is not one of the flow's writes is `INVALID_INPUT`.

The run's `--timeout` clock starts before the approval wait, so give a run that will ask a timeout
long enough to cover your answer.

A write is sent at most once per run. Once accepted, it is not sent again by another tier.

---

## When a run fails

| Code | When |
| --- | --- |
| `AUTH_EXPIRED` | A 401 or 403: signed out or no access. Sign in again in the browser, then rerun |
| `CREDENTIAL_NOT_SENDABLE` | A 401 or 403 for a request that needed a credential the file cannot hold |
| `REQUEST_SIGNED` | A 401 or 403 for a request with a recorded per-request signature or timestamp |
| `RATE_LIMITED` | A 429; the message adds `retry after …` when the site said |
| `BLOCKED_BY_SITE` | A bot challenge page instead of data |
| `OUTCOME_UNKNOWN` | A write was sent and no answer came back. It is not sent again; check the site |
| `WRITE_APPLIED` | A write was accepted but its answer is not the recorded one. Do not repeat it |
| `FLOW_DRIFTED` | A response no longer has the recorded shape; the flow is marked unverified |
| `ACTION_FAILED` | Any other failure, including a host outside the flow's site and an unreadable flow file |
| `FLOW_NOT_FOUND` | No saved flow by that name |
| `INVALID_INPUT` | A bad name, an unknown param or field, or a name that exists on several origins without `origin` |
| `EXTENSION_OFFLINE`, `NO_ACTIVE_TAB` | No browser to send from |
| `DECLINED`, `APPROVAL_TIMEOUT` | A write was refused or not answered |

Each stop's message names the step and tier, for example
`flow 'search-items' step 3 (page): … Sign in again in the browser, then rerun`.

---

## The playbook

The `flows` playbook (`crates/vexdom-core/assets/skills/flows/SKILL.md`) teaches an agent to check
`flowList` before driving a page, how to record and save, and what each stop code means. MCP clients
get it as the prompt `flows`, the resource `skill://flows/SKILL.md`, and through
`guide {"topic":"flows"}`.

---

## See also

- [Network](network.md): the HAR recorder a flow recording uses
- [Auth vault](auth-vault.md): signing in before a run, without the password reaching a flow
- [Approvals](../approvals.md): answering write prompts, standing approvals
- [MCP clients](../mcp-clients.md): `act` and `guide`
- [reference/errors.md](../../reference/errors.md)
