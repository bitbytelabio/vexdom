# Tools

What `vexdom mcp` serves: three tools (`snapshot`, `act`, `guide`), the page and playbook
resources, and the playbooks as prompts. An MCP client you register and the side panel's own agent
see the same three tools. Every browser action runs through `act`; this page ends with the list of
them.

Source: [`app/src/mcp/host.rs`](../../app/src/mcp/host.rs) (tools) and
[`app/src/mcp/mod.rs`](../../app/src/mcp/mod.rs) (server, resources, prompts). The actions come
from [`tools.chromium.json`](../../crates/vexdom-core/assets/tools.chromium.json),
[`tools.firefox.json`](../../crates/vexdom-core/assets/tools.firefox.json) and
[`bridge-actions.json`](../../crates/vexdom-core/assets/bridge-actions.json).

---

## The three tools

| Tool | Title | Annotations | Does |
| --- | --- | --- | --- |
| `snapshot` | Look at the page | `readOnlyHint: true`, `openWorldHint: true` | The active tab as an accessibility tree with refs, as markdown, or as an image |
| `act` | Act in the browser | `readOnlyHint: false`, `destructiveHint: true`, `openWorldHint: true` | Runs one action, or several in order |
| `guide` | Browser actions and playbooks | `readOnlyHint: true`, `openWorldHint: false` | Lists the actions and playbooks; describes one |

`tools/list` always returns these three, whichever browser is connected. Any other tool name is
refused with `UNKNOWN_TOOL`.

---

## `snapshot`

Every property is optional. `as` picks the mode; the other properties apply to the mode named in
their description.

| Property | Type | Description |
| --- | --- | --- |
| `as` | `"tree"` \| `"markdown"` \| `"image"` | Default `tree`. Anything else: `INVALID_INPUT` |
| `interactive` | boolean | tree: only elements to act on (default true) |
| `compact` | boolean | tree: compact rendering (default false) |
| `depth` | integer | tree: how deep to go (default: no limit) |
| `selector` | string | tree: CSS selector of the subtree to read |
| `urls` | boolean | tree or markdown: show link targets (markdown: `[text](absolute url)`) |
| `full` | boolean | tree: the whole tree, not a delta |
| `find` | string | tree: only lines containing this text (case-insensitive), with their parents; the next delta still compares against the last full view |
| `viewport` | boolean | tree: only what is on screen, plus a last line counting the page regions above, below and to the side |
| `outline` | boolean | markdown: headings only, with line numbers |
| `filter` | string | markdown: keep sections mentioning this text |
| `maxChars` | integer | markdown: chars per answer, at most the server's limit |
| `offset` | integer | markdown: start at this char; a cut answer names the next offset |
| `url` | string | markdown: fetch this page over HTTP instead of reading the tab |
| `raw` | boolean | markdown with `url`: the body as served |
| `requireMd` | boolean | markdown with `url`: fail unless the site serves markdown |
| `llms` | `"index"` \| `"full"` | markdown with `url`: the site's `llms.txt` or `llms-full.txt` |
| `target` | string or object | markdown or image: one element (a ref or a locator) |
| `fullPage` | boolean | image: the full scroll height |
| `annotate` | boolean | image: draw each ref on its element |
| `ifChanged` | boolean | image: skip a capture identical to the last |
| `save` | boolean | image: also keep a file and report `savedTo` |
| `format` | `"png"` \| `"jpeg"` | image |
| `quality` | integer | image |
| `tabId` | integer | A tab other than the active one |

- **tree** returns the tree with a ref per element (`button "Sign in" [ref=e5]`). After the first
  call on a tab, it returns only what changed, unless `full` is true: `snapshot r3 (delta from
  r2):` and one or more hunks, `@ lines a-b replaced by:`, `@ inserted after line n:` or
  `@ lines a-b removed`, numbered by the previous tree's lines. A change of URL or options, or one
  that would take 70 % of the tree, returns the whole tree. In interactive mode table cells get no
  ref (the controls inside them do), and a ref'd name longer than 150 characters is cut with `…`.
  `find` answers `find "text": n of m lines match` and the matching lines; its refs are valid, and
  it does not move the delta baseline.
- **markdown** without `url` reads the tab's text. With `url` it fetches the page over HTTP and
  touches no tab; a failure there keeps its own code, or reads `READ_FAILED`. Agent-supplied
  `headers` are dropped. The URL and each redirect hop are checked like a navigation to them
  (blocked sites, scope, URL payload; `page.checkSites`), and a host that is, or resolves to, a
  loopback, private or link-local address is refused with `READ_REFUSED` unless `vexdom mcp` was
  started with `VEXDOM_READ_PRIVATE=1`.
- **image** runs `page.screenshot` and returns an MCP image block plus a short text note.

---

## `act`

```json
{"type": "object", "properties": {"do": {"type": "string"}, "batch": {"type": "array", "items": {"type": "object"}}, "bail": {"type": "boolean"}}, "additionalProperties": true}
```

**One action.** `{"do": "<action>", ...its input}`. Every key other than `do` is the action's
input.

```json
{"do": "clickElement", "target": "e5"}
```

**Several.** `{"batch": [{"do": …}, …], "bail": true}`.

```json
{"batch": [{"do": "fillInput", "target": "e3", "value": "Ada"}, {"do": "submitForm", "target": "e3"}], "bail": true}
```

- The steps run in order. The reply is one line per step: `1. fillInput ok: <result>` or
  `2. submitForm failed: CODE: message`.
- Without `bail`, every step runs and the reply is not marked as an error, even when a step failed.
- With `bail: true`, the batch stops at the first failed step, adds `Stopped after step N.`, and the
  reply is marked as an error.
- An empty `batch` is refused with `INVALID_INPUT`, as is an `act` call with neither `do` nor
  `batch`.

### Naming an action

`do` takes the short name (`clickElement`). The full name (`page.clickElement`,
`bridge.flowRun`) and the older forms `page_clickElement` and `mcp__vexdom__page_clickElement` resolve
to the same action.

A name that matches no offered action is refused:

```
UNKNOWN_ACTION: There is no action "clikElement". Closest: clickElement, …. Call guide for the list.
```

The three closest names are picked by edit distance, compared case-insensitively.

When an action refuses its input with `INVALID_INPUT`, the reply adds the action's input schema:
`<name> takes: {…}`.

### Element arguments

`target`, `from` and `to` take a ref (`"e5"` or `{"ref": "e5"}`) or a locator object
(`selector`, `text`, `role`, `label`, `nth`, and the other locator fields the action's schema
lists). Refs stay valid until the next snapshot of that tab:

| Code | When |
| --- | --- |
| `NO_SNAPSHOT` | A ref was used on a tab with no snapshot yet |
| `UNKNOWN_REF` | The ref is not in the tab's latest snapshot |
| `STALE_REF` | The ref's document unloaded, or its element is gone |

Actions on a snapshotted tab that change it (click, hover, fill, type, select, press, submit,
scroll, drag, wait, navigate) answer with the result followed by the snapshot delta they caused.
The snapshot first gives a load the action started up to 500 ms, then waits for 50 ms without a
DOM change (at most 500 ms); if the navigation replaces the page mid-snapshot, it waits for the new
page to load and reads that. A batch answers with one delta, after its last page-changing step.
Over MCP, element actions leave out
the element's `selector`, `bounds` and `tag` from their result.

`clickElement`, `hoverElement` and `setChecked` wait up to `timeoutMs` (default 2000) for the
element to be enabled, visible and uncovered before they refuse it with `INVALID_TARGET`.
`submitForm` returns `submitted: false` and the first `invalid` fields, each with its
`ValidityState` reason (`valueMissing`, `typeMismatch`, …), when the browser's validation stopped
the form.

### What `act` will not run

| Action | Why |
| --- | --- |
| `bridge.authSave` | `REFUSED_ACTIONS`: a password typed into a tool call would sit in the agent's transcript. Use `vexdom auth save` |
| `page.checkSites` | `REFUSED_ACTIONS`: never offered over MCP |
| `page.injectCode`, `page.runCode` | Left out for an external caller while the guardrail policy denies them, which it does by default (`external-code-injection`, `external-code-execution`; see [Approvals](../guide/approvals.md)) |
| `page.snapshot`, `page.readHtml` | The Bridge's own; never offered or forwarded |
| Chromium-only actions on Firefox | Not in the Firefox extension's list, so not offered: `UNKNOWN_ACTION` |

A refused action is absent from `guide` and answers `UNKNOWN_ACTION`.

Inside a side-panel run (`VEXDOM_AGENT_RUN` set), the run decides the offer: `authSave` is withheld,
`injectCode` and `runCode` stay out unless live tools are on for that conversation, and two
reserved actions are added: `focusShot` (the A-Eye pick's picture) always, `saveSiteMap` in mapping runs.

### Approvals

An action the [policy](../guide/approvals.md) holds for approval makes the call wait up to 120
seconds. A client that supports elicitation is asked directly (**Allow once**, **Always allow on
this site**, **Deny**); otherwise answer in Vexdom.app or with
`vexdom approve c-… [--always]` / `vexdom deny c-…`. A refusal answers `DECLINED`, no answer
`APPROVAL_TIMEOUT`, and an approval given after the tab moved to another site `PAGE_CHANGED`
(nothing was run). A `flowRun`'s writes are asked about the same way; its `allowWrite` is dropped.
On the 2026-07-28 protocol the call returns `input-required` and finishes when the client sends the
answer; an answer to an approval that is no longer waiting gets
`CONFIRM_EXPIRED`.

---

## `guide`

```json
{"type": "object", "properties": {"topic": {"type": "string"}}}
```

**No topic** (`guide_index`): one line on the connected browser and its active tab, then:

```
Actions (call act {"do":"<name>", ...}; guide {"topic":"<name>"} gives one's input):
core: getPageInfo, scrollTo, clickElement, …
captcha — get past a “verify you are human” check: reCAPTCHA, hCaptcha, Turnstile: findCaptcha, solveCaptcha (Chromium only)
…
Playbooks (guide {"topic":"<name>"}):
vexdom — …
```

- `core` lists the offered actions that belong to no group.
- Each group line carries the group's one-line index text from its manifest (`toolGroups`), then
  its offered actions. `(Chromium only)` marks a group whose actions are all Chromium-only.
- A group with nothing offered (on Firefox, for example) is left out of the index.

**With a topic** (`guide_topic`), tried in this order:

| Topic | Returns |
| --- | --- |
| `status` | The connection status as JSON |
| An offered action, any accepted spelling | `<name>: <description>`, `read-only: yes\|no`, and `input:` its schema with shared-argument descriptions removed |
| A group name (`network`, `flows`…) | `# The <group> guide`: the bodies of the group's playbooks, then each offered member described as above. With none offered: *None of its actions are offered here*, plus *they run only in a Chromium browser* when that is why |
| A playbook name (`vexdom`, `dogfood`…) | Its `SKILL.md`, and the names of its reference files |
| `<playbook>/<path>` (`dogfood/references/issue-taxonomy.md`) | That file |

Anything else is refused with `UNKNOWN_TOPIC`, the three closest topics, and the index.

---

## Resources and prompts

| URI | MIME type | Content |
| --- | --- | --- |
| `vexdom://page/current` | `application/json` | `page.getPageInfo` of the active tab (`maxPerKind: 30`), pretty-printed |
| `vexdom://page/diagram` | `text/plain` | The `layout.diagram` text of `page.getPageInfo` (`maxPerKind: 1`) |
| `vexdom://page/text` | `text/plain` | `page.extractText` with `format: "text"` |
| `skill://<name>/SKILL.md`, `skill://<name>/<path>` | `text/markdown` | Every file of every MCP playbook |

Page resources are sealed and fenced like tool results. An unknown URI is an `invalid_params`
error.

The playbooks are the bundled skills listed in `MCP_SKILLS`
([`skills.rs`](../../crates/vexdom-core/src/skills.rs)), from
[`crates/vexdom-core/assets/skills/`](../../crates/vexdom-core/assets/skills/). They reach a client
as:

- **prompts**: one per playbook, named after it, whose message is its `SKILL.md`;
- **resources**: the `skill://` files above;
- **Skills extension** entries, for clients that load it;
- `guide {"topic": "<name>"}`.

---

## Results

- Page text is wrapped in `<<<untrusted-page-data:TAG>>>` markers.
- Passwords, keys, tokens and cookie values are replaced by sealed placeholders such as
  `⟦password:4f2a@example.com⟧`. Passed unchanged as `fillInput`'s `value` or `typeText`'s `text`,
  a placeholder becomes the real value in the browser; anywhere else it is refused.
- A page that registers [WebMCP](../guide/features/webmcp.md) tools gets a block appended listing
  them.
- An error is a tool result marked `isError` with the text `CODE: message`. The codes are in
  [Errors](errors.md).

### Output limit

A text block longer than the limit is cut to that many characters of its body (inside the fence),
followed by `[truncated: N of M chars. Ask for less: …]`.

| Set by | Value |
| --- | --- |
| Default | 20,000 characters (`DEFAULT_MAX_OUTPUT`) |
| `vexdom mcp --max-output <n>` | Overrides everything below |
| `VEXDOM_MAX_OUTPUT` | Overrides the files |
| `maxOutput` in `./vexdom.json`, or under `"cli"` in `~/.vexdom/config.json` | See [Configuration](../guide/configuration.md#cli-preferences) |

`0` turns the limit off. `vexdom mcp call` always uses 20,000.

When `VEXDOM_RESULT_BYTES` is set, a reply longer than that many bytes is replaced whole by
`RESULT_TOO_LARGE`, and `extractText`'s `maxLength` is capped at a quarter of it.

---

## Actions

Every action `act` can run, with its group and whether it needs Chromium. On Firefox the
Chromium-only ones are not offered to MCP clients (`UNKNOWN_ACTION`), and the CLI gets
`UNSUPPORTED` for them. For an action's input, run `guide {"topic": "<action>"}`, or
`vexdom tools` for the page actions' full JSON schemas.

### Page actions (82)

Run by the extension. `page.<name>` in the manifests.

| `do` | Group | Chromium only | Does |
| --- | --- | --- | --- |
| `getPageInfo` | core |  | Snapshot the current page: metadata, viewport and scroll state, a text diagram of the landmark regions, the heading outline, and an inventory of interactive elements |
| `scrollTo` | core |  | Scroll the page, or one scrollable element, to an element, an absolute position, or by pixels or one viewport in a direction |
| `clickElement` | core |  | Click an element like a user would, with the full pointer and mouse event sequence: left, right or middle, double click, modifier keys |
| `trustedClick` | core | yes | Click with a real browser-level mouse event: `isTrusted` is true, exactly as if the user had clicked |
| `hoverElement` | core |  | Hover an element to trigger menus, tooltips, and other hover states |
| `focusInput` | core |  | Focus an input or editable element and place the caret, or select all its content |
| `fillInput` | core |  | Fill a text input, textarea, or contenteditable element like a user typing |
| `typeText` | core |  | Type text into a field one key event per character at a human pace, with varied pauses |
| `selectOption` | core |  | Choose an option in a native `<select>` (several for `<select multiple>`) or a custom ARIA listbox or combobox, by value, visible label, or position |
| `setChecked` | core |  | Check or uncheck a checkbox, switch or radio (or the control a label names) |
| `selectText` | core |  | Select text on the page, from a target element or by finding an exact phrase |
| `extractText` | core |  | Read the rendered text or raw HTML of an element or the whole page, one group at a time |
| `query` | core |  | Read one fact about an element or the page: text, form value (never a password's), an attribute, count, box, styles, visible/enabled/checked; or the title or url |
| `pickElement` | core |  | Ask the user to point at an element (A-Eye) |
| `pressKey` | core |  | Send a keyboard key press, with optional modifiers, to an element on the page |
| `submitForm` | core |  | Submit a form, firing its submit event and validation as if the user pressed Enter |
| `waitForElement` | core |  | Wait until an element reaches a state: attached, visible, hidden, or detached |
| `injectCode` | core | yes | Install a small toolkit of JavaScript functions into the page, to be called later with `runCode` |
| `runCode` | core | yes | Call one function from the toolkit `injectCode` installed in this tab, with fresh arguments |
| `listSiteTools` | core |  | List the tools this site offers to agents through WebMCP (`document.modelContext`), with the origin and frame that registered each and, when asked, its input schema |
| `callSiteTool` | core |  | Call one of the tools this site offers through WebMCP, with arguments matching the schema `listSiteTools` reported |
| `siteToolResult` | core |  | The outcome of a site tool started with `callSiteTool` and `detach: true` (running, done, failed or cancelled), waiting up to `waitMs` |
| `cancelSiteTool` | core |  | Stop waiting for a site tool started with `detach: true` and drop whatever it returns |
| `checkSites` | core |  | Whether the user's Blocked sites list allows these addresses: answers allowed, or fails with `SITE_BLOCKED`. Never run over MCP |
| `findSearch` | core |  | Report how this site can be searched: its search boxes (including one behind a toggle), the buttons that reveal them, links to a search page, and the URL template |
| `highlightElement` | core |  | Highlight an element with a temporary outline overlay and optional caption |
| `searchSite` | core |  | Search the site you are on, using that site's own search rather than a web search engine |
| `navigate` | core |  | Navigate the current tab to a URL, or go back, forward, or reload in its history |
| `openTab` | core |  | Open a URL in a new browser tab |
| `switchTab` | core |  | Bring another open tab to the front, making it the tab every later page action targets |
| `switchFrame` | core |  | Step into an `<iframe>` so every later page action runs inside it as if it were the page, or step back out |
| `closeTab` | core |  | Close an open tab |
| `screenshot` | core |  | Capture the tab as a JPEG/PNG image: the viewport by default, the full scroll view, or one targeted element |
| `listFiles` | core |  | List the files the user has stored in Vexdom, with their AI-generated summaries |
| `attachFile` | core |  | Attach a file to a file input: one stored in Vexdom (`fileId`), one captured with `captureDownload` (`downloadId`), or local files by path (`paths`) |
| `listRecordings` | core |  | List the browsing sessions the user recorded in Vexdom, with the goal and step count of each |
| `readRecording` | core |  | Read one saved browsing recording in full: its goal, the values it needs supplied, and its ordered steps |
| `findCaptcha` | captcha | yes | Report what captcha is on the page, without touching it: the vendor, how deep it sits, its bounds, whether it is satisfied, and an open image challenge |
| `solveCaptcha` | captcha | yes | Get past a captcha |
| `readTheme` | theming |  | Measure the page's theme: luminance of background and text, light or dark, the painted palette, design tokens, the type scale, and any dark/light hook its stylesheets define |
| `auditContrast` | theming |  | Score the readability of the page against WCAG contrast rules |
| `applyTheme` | theming |  | Retheme the page, or put it back |
| `startDiagnostics` | diagnostics | yes | Start recording console messages, uncaught exceptions and every request the tab makes |
| `readConsole` | diagnostics | yes | Read the console messages and uncaught exceptions since `startDiagnostics`: level, text, source file and line, and a stack for errors |
| `readNetwork` | diagnostics | yes | Read the requests the page made since `startDiagnostics`: method, URL, status, type, timing, size, and the browser's error text for failures |
| `stopDiagnostics` | diagnostics | yes | Detach the debugger and take Chrome's "Vexdom is debugging this browser" bar away |
| `findProgress` | monitoring |  | Scan the page for progress bars, percent readouts, spinners and busy regions, each with a selector for `startMonitor` |
| `startMonitor` | monitoring |  | Watch one tab in the background until a progress condition completes: an upload reaching 100%, a build log announcing success, a spinner disappearing |
| `monitorStatus` | monitoring |  | Report on background monitors: phase, percent, ETA, how long since anything changed, and the latest log lines |
| `awaitMonitor` | monitoring |  | Block until a background monitor completes, then return its final state with the full log |
| `stopMonitor` | monitoring |  | Stop a background monitor before it completes |
| `startTimer` | monitoring |  | Schedule work for later, once or repeating |
| `timerStatus` | monitoring |  | Report on scheduled jobs: fires so far, fires skipped while the conversation was busy, the next one due, and the latest log lines |
| `stopTimer` | monitoring |  | Cancel a scheduled job before it has run out |
| `captureDownload` | downloads |  | Make the page download a file and keep it |
| `listDownloads` | downloads |  | List the files captured with `captureDownload`, newest first, with notes and where each was saved |
| `dragElement` | drag |  | Drag one thing onto another: reorder a list, move a card between columns, pull a slider handle, draw on a canvas |
| `storage` | storage |  | Read or change the page's localStorage or sessionStorage (of the frame in focus): get all or one key, set, remove, or clear |
| `cookies` | storage |  | Read, set or clear the browser's cookies for a URL (the tab's, unless `url` is given), including HttpOnly ones |
| `clipboard` | storage |  | Read the system clipboard, write text to it, copy the current selection, or paste at the caret |
| `waitFor` | navigation |  | Wait until every condition given holds: text appears, the URL matches a glob, the page reaches a load state, or a JavaScript expression is truthy |
| `openWindow` | navigation |  | Open a new browser window, on a URL or the new-tab page, and make its tab the one you work in |
| `pushState` | navigation |  | Change a single-page app's route without a reload: Next.js's router when the page has one, else `history.pushState` plus a `popstate` event |
| `tabKeep` | navigation | yes | Choose how a tab behind another one runs: `live` (as if in front), `frozen` (paused), or `throttled` (the browser's normal background throttling) |
| `insertText` | input |  | Insert text at the caret without key events, as a paste or input method would |
| `mouse` | input | yes | Drive the real mouse at viewport coordinates: move, press or release a button, or turn the wheel |
| `evaluate` | code |  | Run JavaScript in the page (the frame in focus) and get its value back as JSON: an expression, or statements ending in `return`; `await` works |
| `initScript` | code | yes | Run a script in every document the tab loads from now on, before the page's own scripts: add one, remove one by its identifier, or list them |
| `routeNetwork` | network | yes | Answer or block the tab's requests whose URL matches a pattern: abort them, or reply with a mocked status, body and headers |
| `unrouteNetwork` | network | yes | Remove the route with this exact pattern, or every route of the tab when none is given |
| `harStart` | network | yes | Start recording the tab's requests and responses as a HAR |
| `harStop` | network | yes | Stop the HAR recording and write it as a HAR 1.2 file: to `path` when given (absolute), else into `~/vexdom/captures` |
| `dialog` | dialogs | yes | See or answer the alert, confirm, prompt or leave-page dialog that blocks the tab (`status`, `accept`, `dismiss`), or answer them automatically (`auto`) |
| `emulate` | emulation | yes | Make the tab pretend: a viewport size, a device, a location, offline, extra headers, HTTP basic credentials, print media or a colour scheme |
| `pdf` | capture | yes | Save the page as a PDF: to `path` when given (absolute), else into `~/vexdom/captures` |
| `recordStart` | capture | yes | Record a video of the tab into `path` (absolute, `.webm` or `.mp4`) until `recordStop` |
| `recordStop` | capture | yes | Stop the video; reports the file, its frames and length, and the contact sheet if asked for |
| `traceStart` | devtools | yes | Start a performance trace of the tab (mode `trace`, for chrome://tracing or Perfetto) or a CPU profile (mode `profile`) |
| `traceStop` | devtools | yes | Stop the trace or profile and write it as JSON: to `path` when given (absolute), else into `~/vexdom/captures` |
| `vitals` | devtools | yes | Measure the page's Core Web Vitals (TTFB, LCP, CLS, FCP, INP), and React hydration timing on a profiling build |
| `react` | devtools | yes | Look inside a React app: install the DevTools hook (reloads the tab once), read the component tree, inspect one component, record re-renders, or list Suspense boundaries |
| `a11yAudit` | devtools | yes | Audit the page's accessibility with axe-core: each rule it breaks, how badly, how many elements and which ones |

### Bridge actions (12)

Run by the Bridge itself. `bridge.<name>` in the manifest; none needs Chromium, though the browser
steps they take (a flow's requests, a login's clicks) run in the connected tab.

| `do` | Group | Chromium only | Does |
| --- | --- | --- | --- |
| `flowRecord` | flows |  | Start, stop or check a flow recording on a tab |
| `flowSteps` | flows |  | List the steps of the tab's flow recording: number, command, page it ended on, requests sent, success, and whether a save covers it |
| `flowSave` | flows |  | Save a range of the tab's recording as a named flow: its first-party JSON requests become a reusable call; credentials, cookies and secret-field values are never stored |
| `flowRun` | flows |  | Run a saved flow: its requests are sent with the page's own fetch from a tab on the flow's site, falling back to replaying the recorded page actions |
| `flowList` | flows |  | List saved flows, optionally for one site: params and example values, an input JSON Schema, item fields, the page, and when a run last verified it |
| `authSave` | auth |  | Save a login in the encrypted vault on this computer (AES-256-GCM, `~/.vexdom/vault`). Never run over MCP; use `vexdom auth save` |
| `authLogin` | auth |  | Sign in with a saved login: open its login page, fill username and password from the vault (or a credential plugin), submit, and wait for the next page |
| `authList` | auth |  | List the saved logins: name, username and login URL, never a password |
| `authShow` | auth |  | Show one saved login: its URL, username and saved selectors, never the password |
| `authDelete` | auth |  | Delete a saved login from the vault |
| `diff` | state |  | What changed on a page: the tree against the last or a saved snapshot, pixels against a saved screenshot, or two URLs side by side. A `baseline` must be a snapshot text or a PNG in `~/vexdom/captures` |
| `state` | state |  | Save a tab's cookies and web storage under a name, load them back later, or list, show, rename and clear saved states |

### Groups and playbooks

Each group names the playbook `guide {"topic": "<group>"}` opens, from
`crates/vexdom-core/assets/skills/<name>/SKILL.md`:

| Group | Playbooks |
| --- | --- |
| captcha | `captcha` |
| theming | `page-theming` |
| diagnostics | `page-diagnostics` |
| monitoring | `monitor-progress`, `scheduled-jobs` |
| downloads | `downloads` |
| drag | `drag` |
| storage | `page-storage` |
| navigation | `navigation` |
| input | `input` |
| code | `code` |
| network | `network` |
| dialogs | `dialogs` |
| emulation | `emulation` |
| capture | `capture` |
| devtools | `devtools` |
| flows | `flows` |
| auth | `auth` |
| state | `page-state` |

---

## See also

- [MCP clients](../guide/mcp-clients.md): registering the server and using the tools
- [Page actions](../guide/features/page-actions.md): the actions by task
- [CLI](cli.md): the same actions as shell commands
- [Errors](errors.md): every code an action can return
- [internals/request-path.md](../internals/request-path.md): an `act` call end to end
