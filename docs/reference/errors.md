# Errors

Every error code Vexdom returns, where it comes from, and what to do next. Each code below appears
in the source under `crates/`, `app/` or `packages/extension/src`.

An error is `{ok: false, error: {code, message}}` at every layer. An MCP client gets a tool result
marked `isError` with the text `CODE: message`; the CLI prints `CODE: message` on stderr and exits
`1`, or prints the envelope on stdout with `--json`. The message usually says the fix.

**Origin:** *Bridge* is the `vexdom` daemon and its MCP server; *Extension* is the browser
extension (its background or a page); *CLI* is the `vexdom` command itself.

---

## Connection

| Code | Origin | Meaning and next move |
| --- | --- | --- |
| `EXTENSION_OFFLINE` | Bridge, Extension | No paired browser is connected, or the extension has no Bridge. Open the browser, or pair it (`vexdom pair`). Retrying does not help |
| `DAEMON_UNREACHABLE` | CLI, Bridge | The Bridge did not answer. `vexdom status`, then `vexdom restart` |
| `BRIDGE_ERROR` | Extension | The extension could not read a reply from the Bridge. Usually a version mismatch: update both |
| `BAD_RESULT` | Bridge | The Bridge could not decode the extension's reply. Update both |
| `UNAUTHORIZED` | Extension | A page call came from a Bridge connection that has not proved itself. Re-pair |
| `PAIRING_FAILED` | Extension | Pairing did not complete; the message says why. Get a new code with `vexdom pair` |
| `INVALID_REQUEST` | Extension | The side panel sent a message the background does not know. Reload the extension |

## Actions and input

| Code | Origin | Meaning and next move |
| --- | --- | --- |
| `UNKNOWN_ACTION` | Bridge | No offered action has that name. Over MCP the message lists the three closest names; `guide` lists them all. A Chromium-only action on Firefox, a refused one (`authSave`, `checkSites`) or a Bridge-internal one also lands here |
| `UNKNOWN_TOOL` | Bridge, Extension | MCP: the tool is not `snapshot`, `act` or `guide`. Extension: a saved code tool is gone |
| `UNKNOWN_TOPIC` | Bridge | `guide` has no such action, group or playbook. The message lists the closest and the index |
| `INVALID_INPUT` | Bridge, Extension, CLI | The input failed validation; the message names the field. Over MCP the action's input schema is appended |
| `INVALID_TARGET` | Extension, Bridge | A target with nothing to locate by, or (`authLogin`) a page or a given `url` whose origin is not the login's |
| `UNSUPPORTED` | Extension | The action cannot run here: a Chromium-only action on Firefox, an action that must go through the Bridge, a non-http(s) URL. There is no fallback |
| `ACTION_FAILED` | Extension, Bridge, CLI | The action threw, or a file could not be written. Read the message. Also a flow run that failed for no more specific reason |
| `TIMEOUT` | Extension, Bridge | A wait expired, or an agent or analyst took too long |
| `RESULT_TOO_LARGE` | Bridge (MCP) | Only when `VEXDOM_RESULT_BYTES` is set: the call went through but its reply is over that size. Do not repeat it; ask for less (`maxChars`, `filter`, `selector`, a `target`) |
| `READ_FAILED` | CLI, Bridge | `read <url>` / `snapshot {"as":"markdown","url":…}` could not fetch or convert the page |
| `READ_REFUSED` | Bridge (MCP) | `snapshot {"as":"markdown","url":…}`: the URL or a redirect hop is, or resolves to, a loopback, private or link-local address. An agent reads public pages only; the user can start `vexdom mcp` with `VEXDOM_READ_PRIVATE=1`, or run `vexdom read <url>` |

## Refs and targets

| Code | Origin | Meaning and next move |
| --- | --- | --- |
| `NO_SNAPSHOT` | Bridge | A ref was used on a tab with no snapshot. Take one |
| `UNKNOWN_REF` | Bridge | The ref is not in the tab's latest snapshot. Take a new snapshot |
| `STALE_REF` | Extension | The ref's document unloaded or its element is gone. Take a new snapshot |
| `BAD_SNAPSHOT` | Bridge | The page's snapshot could not be read. Retry; report it if it persists |
| `TARGET_NOT_FOUND` | Extension, Bridge | Nothing matched. The page changed: take a fresh snapshot |
| `AMBIGUOUS` | Extension | `selectOption`: the text is one option's value and another's label. Say `value`, `label` or `index` |
| `OPTION_NOT_FOUND` | Extension | No option matches; the message lists some |
| `OPTION_DISABLED` | Extension | That option is disabled |
| `NOT_MULTIPLE` | Extension | Several values for a single `<select>` or a single-file input |
| `NOT_CHECKABLE` | Extension | `setChecked` on something that is not a checkbox, switch or radio, or unchecking a radio |
| `NOT_EDITABLE` | Extension | `insertText` with no editable element focused. Give a `target` |
| `NOT_A_LINK` | CLI | `click --new-tab` on an element without `href` |
| `CLIPBOARD_FAILED` | Extension | The browser refused to write the clipboard |
| `PICK_CANCELLED` | Extension | The user dismissed A-Eye. Ask in words instead of asking them to point again |
| `NO_FOCUS_SHOT` | Bridge | `focusShot` in a run whose instruction carried no A-Eye pick |

## Tabs, frames and pages

| Code | Origin | Meaning and next move |
| --- | --- | --- |
| `NO_ACTIVE_TAB` | Extension, Bridge | No focused tab in the current window. Open one |
| `TAB_UNREACHABLE` | Extension, Bridge | The page refuses content scripts, or the frame is gone. Navigate to an http(s) page, or re-snapshot |
| `FRAME_UNREACHABLE` | Extension | That iframe cannot be entered (sandboxed, or no script context) |
| `TAB_VISIBLE` | Extension | `tabKeep frozen` on the tab in front. Only a background tab can be frozen |
| `TAB_IN_USE` | Extension | Another Vexdom conversation is working in that tab |
| `SESSION_TAB_CLOSED` | Extension | Every tab this conversation worked in was closed |
| `SESSION_LIMIT` | Extension | Too many tab sessions are open; the message says how many. Close one |
| `SITE_BLOCKED` | Extension, Bridge | The site is on the user's Blocked sites list (or the list cannot be read). Final: do not retry or reach it another way; tell the user. [Guide](../guide/features/blocked-sites.md) |

## Approvals and guardrails

| Code | Origin | Meaning and next move |
| --- | --- | --- |
| `DECLINED` | Bridge | The user refused. Final: do not seek another route to the same effect |
| `APPROVAL_TIMEOUT` | Bridge | Nobody answered within 120 seconds, so nothing was done. Say what was waiting, then stop. [Approvals](../guide/approvals.md) |
| `PAGE_CHANGED` | Bridge | The user approved, but the tab moved to another site while they were asked, so nothing was done. Ask again if it is still needed |
| `CONFIRM_EXPIRED` | Bridge (MCP) | An MCP client answered an approval that is no longer waiting. Call the tool again |
| `BLOCKED` | Bridge | A `deny` rule matched (or `includeBodies` without `network-body-read`); a run tried to change its own settings or save a login. The message says which |
| `SECRET_NOT_RELEASABLE` | Extension, Bridge | A sealed placeholder was used outside `fillInput`'s `value` or `typeText`'s `text` |
| `SECRET_EXPIRED` | Extension | That placeholder is no longer held. Read the value again |
| `USER_SCRIPTS_DISABLED` | Extension | `evaluate` needs the browser's user-scripts permission; the message says how to turn it on |
| `LIVE_TOOLS_OFF` | Bridge | A side-panel run called `injectCode`/`runCode` with live tools off |
| `TOOLKIT_SCOPE` | Extension | The code toolkit was approved for another origin or path. Install it again here |
| `TOOLKIT_MISSING` | Extension | `runCode` with no toolkit installed in the tab. Call `injectCode` first |
| `UNKNOWN_FUNCTION` | Extension | The toolkit has no function of that name; the message lists them |
| `CODE_ERROR` | Extension | Toolkit code threw while installing, or registered no functions |
| `EVAL_FAILED` | Extension | `evaluate`: a syntax error or an exception in the page; the message carries it |

## Debugger features (Chromium)

| Code | Origin | Meaning and next move |
| --- | --- | --- |
| `DEBUGGER_UNAVAILABLE` | Extension | Chrome refused to attach, usually because DevTools is open on the tab. Close it and retry |
| `DEBUGGER_DETACHED` | Extension | The debugger left the tab mid-feature. Call again to re-attach |
| `DIAGNOSTICS_NOT_FOUND` | Extension | Nothing is recording, or that id is unknown. Call `startDiagnostics` before reproducing the problem (the CLI starts one for you) |
| `DIAGNOSTICS_IN_PROGRESS` | Extension | The tab is already being recorded. Read it, or stop it |
| `DIAGNOSTICS_LIMIT` | Extension | Two tabs are already being recorded. Stop one |
| `HAR_ACTIVE` | Extension, Bridge | A HAR or flow recording already runs on this tab. Stop it first |
| `NO_HAR_RECORDING` | Extension | `harStop` with no HAR recording. Start one |
| `NO_DIALOG` | Extension | No dialog is open on the tab |
| `UNKNOWN_DEVICE` | Extension | `emulate` device name not known; the message lists the supported ones |
| `TRACE_ACTIVE` | Extension | A trace or profile already runs on the tab |
| `NO_TRACE` | Extension | `traceStop` with nothing running |
| `TRACING_UNAVAILABLE` | Extension | Neither tracing nor the profiler is available on this tab |
| `RECORDING_ACTIVE` | Extension, Bridge | A video recording is already running. Stop it |
| `NO_RECORDING` | Extension, Bridge | `recordStop` with nothing running |
| `FFMPEG_MISSING` | Bridge | Video needs ffmpeg: `brew install ffmpeg` |
| `ARTIFACT_TOO_LARGE` | Extension, Bridge | A file to write (PDF, HAR, trace, video) is over 48 MiB |
| `REACT_HOOK_MISSING` | Extension | Run `react install` (reloads the tab) first |
| `AXE_UNAVAILABLE` | Extension, Bridge | axe-core could not be provided; `a11yAudit` must go through the Bridge |
| `CAPTCHA_NOT_FOUND` | Extension | No captcha on the page (or it is gone); whatever blocks the page is something else |
| `CAPTURE_FAILED` | Extension | A screenshot came out empty or did not render. Bring the tab to the front and retry |
| `CAPTURE_UNSUPPORTED` | Extension | The browser cannot capture this page (e.g. a browser-internal page) |
| `SCREENSHOT_FAILED` | Extension | An annotated screenshot failed; the message carries the reason |

## WebMCP site tools

| Code | Origin | Meaning and next move |
| --- | --- | --- |
| `NO_SITE_TOOLS` | Extension | The site registers no WebMCP tools. Use the ordinary actions |
| `SITE_TOOL_NOT_FOUND` | Extension | No tool of that name on this page |
| `SITE_TOOL_FAILED` | Extension | The site's tool threw, or returned something that is not JSON |
| `SITE_TOOL_INVOCATION_GONE` | Extension | No such detached call any more. Call the tool again |

## Monitors, timers and tasks

| Code | Origin | Meaning and next move |
| --- | --- | --- |
| `MONITOR_IN_PROGRESS` | Extension | The tab is already watched. Stop that monitor, or watch another tab |
| `MONITOR_LIMIT` | Extension | Three monitors already run. Stop one |
| `MONITOR_NOT_FOUND` | Extension | No such monitor, or it already finished |
| `TIMER_LIMIT` | Extension | Five timers already scheduled. Cancel one |
| `TIMER_NOT_FOUND` | Extension | No such timer, or it already ended |
| `NO_CONVERSATION` | Extension | A timer outside a side-panel conversation. Pass `deliver: "notify"`, or use your client's scheduler |
| `TASK_NOT_FOUND` | Bridge | No scheduled task with that id |
| `TASK_LIMIT` | Bridge | 25 tasks are already scheduled. Delete one |
| `REPLAY_UNAVAILABLE` | Extension | The task's recording cannot be replayed; the message says why |

## Flows

| Code | Origin | Meaning and next move |
| --- | --- | --- |
| `FLOW_NOT_FOUND` | Bridge | No saved flow of that name. `vexdom flow list` |
| `FLOW_NOT_CAPTURING` | Bridge | No flow recording on this tab. `vexdom flow record start`, do the task, `flow record stop` |
| `FLOW_DRIFTED` | Bridge | A response changed shape: the site changed. Do the task in the page and save the flow again |
| `AUTH_EXPIRED` | Bridge | A 401/403: signed out. Sign in again in the browser, then rerun |
| `CREDENTIAL_NOT_SENDABLE` | Bridge | The request needs a credential a flow cannot hold. Use the page for this task |
| `REQUEST_SIGNED` | Bridge | The request carries a recorded signature or timestamp that cannot be replayed. Use the page |
| `RATE_LIMITED` | Bridge | A 429. Wait before rerunning |
| `BLOCKED_BY_SITE` | Bridge | A bot challenge came back instead of data. Continue in the page, or let the user clear the challenge |
| `OUTCOME_UNKNOWN` | Bridge | A write was sent and no answer came. Check the site; the write is not sent again |
| `WRITE_APPLIED` | Bridge | The site accepted the write but answered differently than recorded. It was applied: do not repeat it |

See [Flows](../guide/features/flows.md).

## Auth vault and plugins

| Code | Origin | Meaning and next move |
| --- | --- | --- |
| `AUTH_PROFILE_NOT_FOUND` | Bridge | No saved login of that name. `vexdom auth list` |
| `VAULT_KEY_MISSING` | Bridge | The vault key is missing or cannot open the saved logins; the message says where it looked |
| `PLUGIN_FAILED` | Bridge, CLI | A plugin failed, stopped, or its manifest could not be read. With `plugin add`, `--capability` skips manifest discovery |

See [Auth vault](../guide/features/auth-vault.md) and [Plugins](../guide/features/plugins.md).

## Files, downloads and states

| Code | Origin | Meaning and next move |
| --- | --- | --- |
| `NO_DOWNLOAD_STARTED` | Extension | The click opened a page instead of downloading. Check where the tab landed, or pass the file's URL |
| `DOWNLOAD_FAILED` | Extension | The browser stopped the transfer; the message gives its reason |
| `DOWNLOADS_UNAVAILABLE` | Extension | The extension lacks the downloads permission. Reload it and accept the prompt |
| `DOWNLOAD_OFF_SCOPE` | Bridge | The file came from a host the run was not asked about. It was deleted |
| `DOWNLOAD_REFUSED` | Bridge | An executable. Deleted; not overridable |
| `DOWNLOAD_TOO_LARGE` | Bridge | Over the capture limit (deleted), or too large to attach |
| `DOWNLOAD_NOT_FOUND` | Bridge | No captured download with that id. `listDownloads` |
| `DOWNLOAD_MISSING` | Bridge | Captured, but no longer on disk. Capture it again |
| `DOWNLOAD_SAVE_FAILED` | Bridge | The file could not be moved into the download folder |
| `FILE_NOT_FOUND` | CLI | A path given to `upload`, `diff --baseline` or `state load` does not exist |
| `FILE_TOO_LARGE` | Bridge | Files to attach are too large together, or an attached file is over the size the analyst reads |
| `DIFF_FAILED` | CLI, Bridge | `diff` could not compare; the message says why |
| `STATE_FAILED` | CLI, Bridge | `state` could not save or load; the message says why |

## Side-panel runs

| Code | Origin | Meaning and next move |
| --- | --- | --- |
| `RUN_IN_PROGRESS` | Bridge, Extension | This conversation (or scheduled task) is still running. Cancel it first |
| `RUN_LIMIT` | Bridge | `maxConcurrentRuns` runs already going (default 3, at most 8) |
| `RUN_INACTIVE` | Bridge | The run ended while a call was in flight |
| `CANCELLED` | Bridge | The run, or a file's analysis, was cancelled |
| `NO_SKILL` | Bridge | No skills found. Reinstall, or add one |
| `SKILL_UNKNOWN` | Bridge | The attached agent skill is gone or grew past the size cap. Choose it again |
| `NAME_TAKEN` | Bridge | A skill or site map of that name exists |
| `NOT_FOUND` | Bridge | No such mapped site or staged map |
| `TOO_MANY_SKILLS` | Bridge | The uploaded-skill limit is reached. Remove one |
| `WRITE_FAILED` | Bridge | A skill or site map could not be written or removed |
| `MAPPING_NOT_REQUESTED` | Bridge | A mapping run was started without the user asking for one |
| `MAPPING_REFUSED` | Bridge | The mapping run could not start; the message says why |
| `MAPPING_READ_ONLY` | Bridge | A mapping run may only read pages |
| `MAPPING_OFF_SITE` | Bridge | A mapping run may only visit the mapped origin, by absolute URL |
| `MAPPING_BUDGET` | Bridge | The page or screenshot budget is spent. Write up with `saveSiteMap` |
| `MAPPING_TAB_CHANGED` | Extension | The tab the mapping run started in was closed |
| `MAPPING_REPORT_TOO_LARGE` | Bridge | The site map report is too large. Summarize each page in a sentence |
| `ALREADY_SUBMITTED` | Bridge | The run already wrote its map |
| `RECORDING_IN_PROGRESS` | Extension | A browsing recording already runs. Stop it first |
| `RECORDING_LIMIT` | Extension | The recording hit its size limit |
| `RECORDING_NOT_FOUND` | Extension | No saved recording with that id |
| `RECORDING_NOT_READY` | Extension | The recording has not been turned into steps (yet, or it failed) |
| `RECORDING_TOO_LARGE` | Bridge | The recording is too large to summarize |
| `INVALID_WORKFLOW` | Bridge | The analysed recording is not a usable workflow; the message says what is missing |

## Agent

| Code | Origin | Meaning and next move |
| --- | --- | --- |
| `AGENT_MISSING` | Bridge | Claude Code is not installed, or not on the Bridge's `PATH`. Install it, or set `agents.claude.bin` to an absolute path |
| `AGENT_UNUSABLE` | Bridge | The agent CLI is there but could not start; the message carries the error |
| `AGENT_UNSAFE` | Bridge | Vexdom refused to start the agent without its containment. A bug in Vexdom: report it |
| `AGENT_FAILED` | Bridge | The file analyst returned no usable report. Remove the file and attach it again |

## Attached files

These are reasons on a file's analysis report, shown on its chip, not tool errors.

| Code | Meaning |
| --- | --- |
| `UNSUPPORTED_TYPE` | Not text, PDF or an image |
| `FILE_TOO_LARGE` | Over the size limit for its kind |
| `EMPTY` | The file has no bytes |
| `UNREADABLE` | Opened but could not be read |
| `TIMEOUT` | The analysis took too long |
| `CANCELLED` | The file was removed before it was read |
| `AGENT_FAILED` | The analyst returned no usable report |

## `vexdom update`

| Code | Origin | Meaning and next move |
| --- | --- | --- |
| `UPDATE_UNMANAGED` | CLI | This copy was not installed from a release. Reinstall with `install.sh`, or rebuild |
| `UPDATE_UNREACHABLE` | CLI | GitHub could not be reached, or no release is published |
| `UPDATE_ASSET_MISSING` | CLI | The release has no file for this platform |
| `UPDATE_DOWNLOAD_FAILED` | CLI | A download failed; the message has the URL |
| `SIGNATURE_INVALID` | CLI | `SHA256SUMS` is not signed by the release key. Nothing was replaced |
| `CHECKSUM_MISMATCH` | CLI | The download is not the file the release signed. Nothing was replaced |
| `UPDATE_ARCHIVE_INVALID` | CLI | The archive is empty or lacks `bin/vexdom` or `extension/chrome-mv3` |

---

## See also

- [Tools](tools.md) and [CLI](cli.md): where these come from
- [Troubleshooting](../guide/troubleshooting.md): symptom, cause and fix
- [Limits](../guide/limits.md): errors that mark a boundary rather than a bug
- [internals/guardrails.md](../internals/guardrails.md): how a guardrail decision is reached
