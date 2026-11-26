# Page actions

Everything Vexdom can do to a page is a named action. The Chromium extension offers 84 page actions
(`page.<name>`); the Firefox build offers 58 of them. The Bridge adds 7 actions of its own
(`bridge.<name>`). You do not name them in an instruction, since the agent picks one, but knowing
what exists tells you what is worth asking for.

---

## Three ways to call one

| Caller | Form |
| --- | --- |
| Side-panel agent and MCP clients | `act {"do":"clickElement","target":"e5"}`, or `act {"batch":[{"do":…},…],"bail":true}` |
| CLI | a verb: `vexdom click @e5`, `vexdom fill @e3 "hi"`, `vexdom network route …` ([reference/cli.md](../../reference/cli.md)) |
| CLI, any action | `vexdom batch '{"action":"page.emulate","input":{"device":"Pixel 9"}}'` |

`act` takes the short name (`clickElement`); `page.clickElement` works too. `guide` with no topic
lists every action the connected browser offers, by group, marking the Chromium-only groups;
`guide {"topic":"<action>"}` gives one action's input and whether it only reads. `vexdom tools`
prints the whole bundled manifest without a browser.

---

## How targeting works

`snapshot` (`vexdom snapshot`) returns the page's accessibility tree with a ref per element (`e5`);
after the first call, only what changed. An element input takes a ref (`"e5"` or
`{"ref":"e5"}`, `@e5` on the CLI) or a locator: `{"selector"|"text"|"role"|"label"|"placeholder"|"alt"|"title"|"testId"|"name"|"nth"}`.
Refs stay valid until the next snapshot of that tab; an unknown or stale ref is refused with
`UNKNOWN_REF`, `STALE_REF` or `NO_SNAPSHOT`.

Actions on a snapshotted tab (click, fill, type, select, press, submit, navigate) answer with the
snapshot delta they caused, so a fresh snapshot is rarely needed.

---

## Core actions

The 39 actions in no group, which `guide` lists as `core`.

**Reading**

| | |
| --- | --- |
| `snapshot` | The accessibility tree the refs come from (MCP: the `snapshot` tool) |
| `getPageInfo` | Metadata, viewport, a landmark diagram, headings and an inventory of interactive elements with selectors; also site tools and captchas present |
| `readHtml` | HTML of the page or one element, which the Bridge turns into markdown (`snapshot {"as":"markdown"}`, `vexdom read`). Raw HTML for the agent is [denied by default](../approvals.md) |
| `extractText` | Rendered text, a group of sentences at a time, with a cursor for long pages |
| `query` | One fact, cheaper than a snapshot: text, value, attribute, count, box, styles, visible/enabled/checked, title, URL (`vexdom get …`, `vexdom is …`) |
| `waitForElement` | Wait until an element is attached, visible, hidden or detached |
| `findSearch` | Whether the site has its own search, and where |
| `pickElement` | Let you point at the element you mean; see [A-Eye](a-eye.md) |
| `screenshot` | See [Screenshots](screenshots.md) |
| `highlightElement` | A temporary outline with an optional caption |
| `checkSites` | Whether your [blocked sites](blocked-sites.md) list allows some addresses |

**Clicking and typing**

| | |
| --- | --- |
| `clickElement` | A click with the full pointer and mouse event sequence |
| `trustedClick` | A browser-level click through Chrome's debugger (`isTrusted` true). Chromium only |
| `hoverElement` | Menus, tooltips, hover states |
| `focusInput` | Focus, place the caret or select all |
| `fillInput` | Replace a field's value |
| `typeText` | Key by key, at a human pace |
| `selectOption` | A `<select>` option by value, label or position |
| `setChecked` | Check or uncheck a checkbox, switch or radio |
| `selectText` | Select text by element or exact phrase |
| `pressKey` | A key with modifiers |
| `submitForm` | Submit a form, firing its validation |

**Moving around**

| | |
| --- | --- |
| `navigate` | A URL, or back / forward / reload |
| `searchSite` | Search this site with its own search and land on the results |
| `scrollTo` | An element, a position, or a viewport |
| `openTab` / `switchTab` / `closeTab` | Tabs; `switchTab` with no input lists them |
| `switchFrame` | Step into an `<iframe>` so later actions run inside it, or back out |

**Site tools, page code, files, recordings**

| | |
| --- | --- |
| `listSiteTools`, `callSiteTool`, `siteToolResult`, `cancelSiteTool` | [WebMCP](webmcp.md) |
| `injectCode`, `runCode` | Live page code, below. Chromium only |
| `listFiles`, `attachFile` | [Files](files.md) |
| `listRecordings`, `readRecording` | [Recordings](recordings.md) |

---

## Groups

The other 45 page actions are grouped; `guide {"topic":"<group>"}` returns the group's
playbook (every group names one), then each action's description and input.

| Group | Actions | See |
| --- | --- | --- |
| navigation | `waitFor`, `openWindow`, `pushState`, `tabKeep`* | [Emulation](emulation.md) (`tabKeep`) |
| input | `insertText`, `mouse`* | |
| drag | `dragElement` | |
| storage | `storage`, `cookies`, `clipboard` | |
| code | `evaluate`, `initScript`* | |
| downloads | `captureDownload`, `listDownloads` | [Files](files.md) |
| dialogs* | `dialog` | |
| network* | `routeNetwork`, `unrouteNetwork`, `harStart`, `harStop` | [Network](network.md) |
| emulation* | `emulate` | [Emulation](emulation.md) |
| capture* | `pdf`, `recordStart`, `recordStop` | [Capture](capture.md) |
| devtools* | `traceStart`, `traceStop`, `vitals`, `react`, `a11yAudit` | [DevTools](devtools.md) |
| diagnostics* | `startDiagnostics`, `readConsole`, `readNetwork`, `stopDiagnostics` | [Diagnostics](diagnostics.md) |
| captcha* | `findCaptcha`, `solveCaptcha` | [Captchas](captcha.md) |
| theming | `readTheme`, `auditContrast`, `applyTheme` | [Theming](theming.md) |
| monitoring | `findProgress`, `startMonitor`, `monitorStatus`, `awaitMonitor`, `stopMonitor`, `startTimer`, `timerStatus`, `stopTimer` | [Monitoring](monitoring.md), [Scheduling](scheduling.md) |

\* Chromium only: a whole group, or the marked action. Those use Chrome's debugger, show its "Vexdom
is debugging this browser" bar while they hold it, and fail with `UNSUPPORTED` on Firefox.

A few of them, by CLI verb: `vexdom wait --text|--url|--load|--fn` (`waitFor`),
`vexdom window new` (`openWindow`), `vexdom pushstate` (`pushState`), `vexdom keyboard inserttext`
(`insertText`), `vexdom mouse move|down|up|wheel` (`mouse`), `vexdom storage`, `vexdom cookies`,
`vexdom clipboard`, `vexdom eval` (`evaluate`), `vexdom initscript add|remove|list` (`initScript`),
`vexdom dialog [status]|accept|dismiss|auto on|off` (`dialog`).

---

## Bridge actions

Run by the Bridge rather than the extension, called the same way.

| | |
| --- | --- |
| `diff` | What changed: the tree against the last or a saved snapshot, pixels against a baseline screenshot, or two URLs (`vexdom diff snapshot\|screenshot\|url`). An agent's baseline must be a snapshot text or a PNG in `~/vexdom/captures` |
| `state` | Save a tab's cookies and web storage under a name and load them later (`vexdom state save\|load\|show\|list\|rename\|clear\|clean`) |
| `authSave`, `authLogin`, `authList`, `authShow`, `authDelete` | The [auth vault](auth-vault.md) |

---

## Live page code

Repeating one sequence twenty times, or a job no action covers (seeking a video, reading a canvas,
driving an editor's API), is what `injectCode` and `runCode` are for. Both are Chromium only.

**They are off until you switch them on** with the composer's **Live tool** switch, the `</>`
button beside the message box. While it is off the agent is not told they exist and gets
`LIVE_TOOLS_OFF` if it tries.

`injectCode` asks to install a small toolkit of JavaScript functions in the page. **You review the
source and approve it**: the prompt has a **Review** button that opens the full code. `runCode` then
calls those functions with new arguments, as often as the job needs, without asking again. The
approval covers that code, on that tab, on that site, with no "always on this site" option.

Only a side-panel conversation can use them: for an MCP client or the CLI the
`external-code-injection` and `external-code-execution` guardrails deny both, because nobody is there
to read the code. `evaluate` (`vexdom eval`) is a separate action in the `code` group that runs one
script in the page and returns its value.

### Keeping one

A second after a toolkit installs, the panel offers to keep it as a tool of your own, named after
where it belongs (for example `youtube.com:watch:darken-page-except-video-player`). It then runs
whenever you type `/` on that site and pick it: no agent and no second approval. Only a
zero-argument function can be kept. Scope is the host plus the first path segment.

The JavaScript stays in the extension's storage; the Bridge gets only a markdown note saying the tool
exists, so the agent can point you to it. A kept tool is not a page action, so no MCP client can call
one. Type `/remove-tools` for the list, with a cross beside each.

### Running one on every visit

The same prompt has a switch, off by default: **Also run it on every visit to …**. With it on, the
tool runs each time you arrive in its scope, once the page has stopped changing (at most three
seconds). Chrome injects it as a user script, so there is no debugger bar.

**Chrome needs one switch first:** `chrome://extensions` → Vexdom's **Details** → **Allow User
Scripts** (before Chrome 138, Developer mode). Until then the panel says so and offers to open that
page.

---

## See also

- [Approvals](../approvals.md): which actions pause and ask
- [reference/cli.md](../../reference/cli.md): every CLI verb
- [internals/registry.md](../../internals/registry.md): where the manifest comes from
