---
name: derive-client
description: Reverse-engineer a site's internal API by recording the tab's traffic as a HAR while using it, then write a standalone client or CLI that calls those endpoints directly, without a browser.
---

# Derive an API client from a recorded session

Adapted from agent-browser skill-data/derive-client (f0a9835, Apache-2.0).

Driving a browser is the right tool for the first visit and the wrong tool for the hundredth. This
playbook records a site's network traffic once while you use it, then turns the captured requests
into a standalone client (script, CLI or library) that talks to the site's internal API directly.

For a task **you** will repeat through vexdom, you do not need a client: record it as a flow
(`flowRecord` → `flowSave` → `flowRun`, the `flows` playbook), which reruns the requests from the
user's tab. Use this playbook when the user wants code that runs outside vexdom.

## Workflow

```
1. Record     start a HAR, drive the flows the client should cover
2. Identify   find the real API endpoints among the noise
3. Extract    request shapes, response schemas, what auth the API wants
4. Generate   write the client, one function per flow
5. Verify     call every endpoint for real before declaring done
```

## 1. Record

    act {"do":"harStart"}                        text response bodies kept by default
    … drive the site: search, open a detail page, paginate …
    act {"do":"harStop","path":"/abs/path/site.har"}

- Exercise every flow the client should support, each at least twice with different inputs (two
  search terms, two detail pages). Diffing the recorded URLs shows which parts are parameters.
- If the site needs a login, sign in before starting the HAR so the login does not land in the
  recording.
- `content:"all"` keeps binary bodies too (base64); `"none"` keeps none. A body over 2 MB is cut.
- While recording, `startDiagnostics` + `readNetwork` (with `requestId`) answer quick questions
  interactively, but only the HAR survives navigation; prefer it for anything multi-page.
- HAR recording runs only in a Chromium browser.

## 2. Identify endpoints

Query the HAR with `jq` from a shell:

```bash
# Every JSON call: method, status, URL
jq -r '.log.entries[]
  | select(.response.content.mimeType | test("json"))
  | "\(.request.method) \(.response.status) \(.request.url)"' /abs/path/site.har
```

Ignore analytics and infrastructure noise: telemetry endpoints (`/collect`, `/track`, `/beacon`,
`/log`), third-party domains (analytics, error reporting, chat widgets) and static assets. The real
API is usually first-party, JSON, and lines up with the actions you performed.

## 3. Extract shapes and auth

```bash
# One endpoint in full: request headers, POST body, response body
jq '.log.entries[] | select(.request.url | test("api/search"))
  | {request: {method: .request.method, headers: .request.headers,
     postData: .request.postData.text},
     response: .response.content.text}' /abs/path/site.har
```

- Response schema: `.response.content.text` is the real payload; derive the types from it.
- Auth: compare request headers across endpoints — `authorization`, `cookie`, `x-csrf-token`,
  `x-api-key`, site-specific `x-*`. Note which the API needs; test by omission in step 5.
- vexdom keeps cookie and token values sealed: `cookies` will not hand them to you, and you must
  not copy any out of the HAR into code. The client loads its credential at runtime from a file or
  environment variable the user fills (from their own browser, an API token, or the site's login
  endpoint), with a clear error telling them to refresh it when it expires.

## 4. Generate the client

- One function per recorded flow (`search(query)`, `getItem(id)`), typed from the observed
  response bodies.
- Reproduce the headers the API actually requires — some sites answer 403 without a matching
  `user-agent`, `referer` or `x-requested-with`.
- Keep the pagination, sort and filter parameters seen in recorded query strings as options.

## 5. Verify

Call every generated function against the live API and compare the shape with the recording.

| Symptom | Cause | Fix |
|---|---|---|
| 401/403 | expired or missing session | the user signs in again and refreshes the credential |
| 403/419 on writes | per-session or per-form CSRF token | fetch the token endpoint first, or keep that flow in the browser |
| works, then breaks | signed or expiring request parameters | keep that step in the browser; derive the rest |
| different shape than the HAR | A/B tests, region-dependent answers | record again; treat the union as optional fields |

## Caveats

- Internal APIs are unversioned and change without notice: keep the HAR so the client can be
  derived again.
- Respect the site's terms and rate limits; add delays for bulk fetching.
- A HAR holds live session data (cookies, tokens, POST bodies). Keep it out of version control and
  delete it when done.

## From a shell

`vexdom network har start [--content all|text|none]`, `vexdom network har stop /abs/path/site.har`,
`vexdom network requests`, `vexdom network request <id>`; for a vexdom-run repeat instead:
`vexdom flow record start`, `vexdom flow save <name> --param q=shoes`, `vexdom flow run <name> --param q=boots`.
