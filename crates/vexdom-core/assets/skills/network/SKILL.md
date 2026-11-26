---
name: network
description: Control and record what the tab sends — mock or block requests by URL pattern, record a HAR file, and read single requests with their bodies.
---

# Network: routes, HAR, requests

All of this runs only in a Chromium browser (Chrome, Edge, Brave) and attaches the debugger, so
Chrome shows "Vexdom is debugging this browser" while it is on. Tell the user when you start, and
remove what you set up when you are done.

## Mock or block requests

`act {"do":"routeNetwork","pattern":"*/api/items*","response":{"status":200,"body":"{\"items\":[]}"}}`
answers every matching request of the tab with that response, without it reaching the server.

- `pattern`: `*` is a wildcard; without one the URL only has to contain the text.
- `response`: `status` (200 by default), `body` (JSON gets `application/json`), `contentType`,
  `headers`, `allowOrigin` (answers CORS, preflights included, for that origin).
- `abort:true` fails the request instead — test how the page copes with an API that is down, or
  block a tracker.
- `resourceTypes:["xhr","fetch"]` limits the route to some request types (`document`, `script`,
  `image`, `stylesheet`, …).

Routes apply in the order you added them, and only to that tab. They take effect on the next
request: reload or redo the action after adding one. `act {"do":"unrouteNetwork","pattern":"…"}`
removes one; with no pattern, every route of the tab.

Never use a route to fake an outcome the user believes is real (a payment, a submitted form): say
the page ran against a mock.

## Record a HAR

`act {"do":"harStart"}` records the tab's requests and responses; `content:"text"` (default) keeps
text bodies (JSON, HTML, scripts), `"all"` every body, `"none"` only sizes and types. Do the task,
then `act {"do":"harStop","path":"/abs/path/site.har"}` writes a HAR 1.2 file (without `path`, into
`~/vexdom/captures/`) and reports its path, size and request count. Only one HAR per tab
(`HAR_ACTIVE`); `harStop` without a start is `NO_HAR_RECORDING`.

A HAR holds live session data: cookies, tokens, request bodies. Treat it as a secret, keep it out of
version control, and delete it when done. For a task you will repeat, the `flows` playbook
records and replays it for you; to write a standalone client, see `derive-client`.

## Read requests

Requests are recorded only while diagnostics run: `act {"do":"startDiagnostics"}` first, then make
the page act, then read.

- `act {"do":"readNetwork","status":"problems"}` — failed and 4xx/5xx first; `"pending"` for
  requests that never answered.
- Narrow with `urlContains`, `method`, `resourceTypes:["fetch"]`, `statusCode` (`"404"`, `"5xx"`,
  `"200-299"`), `limit`.
- `act {"do":"readNetwork","requestId":"…","includeHeaders":true,"includeBodies":true}` returns one
  request with its POST body and response body. Bodies are refused (`BLOCKED`) unless the user has
  allowed network body reading in Vexdom's guardrails; ask, do not push.
- `act {"do":"stopDiagnostics"}` when done.

For debugging a page that misbehaves (console errors plus failed requests), follow
`page-diagnostics`.

## From a shell

`vexdom network route '*api/items*' --body '{"items":[]}' [--status 200] [--content-type t] [--allow-origin o] [--resource-type fetch,xhr]`,
`vexdom network route '*tracker*' --abort`, `vexdom network unroute [pattern]`,
`vexdom network har start [--content all|text|none]`, `vexdom network har stop [path]`,
`vexdom network requests [--filter s] [--type fetch] [--method POST] [--status 4xx]`,
`vexdom network request <id>`, `vexdom console`, `vexdom errors`.
