# Inside the extension

The extension (`packages/extension/`, WXT, built with Bun) runs every browser action, hosts the
side panel and the settings page, draws the rail, the hands-free orb and the action cues into
pages, and keeps each conversation in its own tabs. One source builds a Chrome Manifest V3
extension (`bun run build`) and a Firefox Manifest V2 add-on (`bun run build:firefox`).

Entrypoints are in `src/extension/entrypoints/` (`background.ts`, `content.ts`, `sidepanel/`,
`options/`, `launcher/`, `newtab/`, `welcome/`, `dictation/`, `mic-permission/`); the logic is in
`src/lib/`.

---

## Background vs content script

A `Page.invoke` from the Bridge lands in the background, in `invokeForHarness()`
(`src/lib/bridge/invoke.ts`; the request path is in [Request path](request-path.md)). Not every
action can run in the page, so the background keeps many for itself:

| Where | Actions | Why |
| --- | --- | --- |
| Background, no tab | `listFiles`, `listRecordings`, `readRecording`, `readConsole`, `readNetwork`, `stopDiagnostics`, `monitorStatus`, `stopMonitor`, `awaitMonitor`, timers, `siteToolResult`, `cancelSiteTool`, `checkSites` | The data lives in extension storage and outlives any page |
| Background, tabs API | `openTab`, `openWindow`, `switchTab`, `closeTab`, `navigate` (off http(s) pages), `screenshot`, `pickElement`, `cookies` | Need `tabs`, `windows`, `scripting` or `cookies` |
| Background, debugger hub | The `chromiumOnly` actions (see [below](#the-debugger-hub)) | Need `chrome.debugger` |
| Background, scripting | `evaluate` (`userScripts.execute` on Chrome, the page's own `eval` on Firefox), `pushState`, `waitFor` | Run in the page's main world |
| Content script | Everything else | Reads and changes the DOM |

`routeInTab()` in `invoke.ts` is that table as code. Anything it does not route itself goes to
`invokeInTab()` or `invokeInFrame()` (`src/lib/actions/client.ts`), which sends
`{channel: "vexdom/action", action, input}` with `tabs.sendMessage` to one frame.

The content script (`entrypoints/content.ts`, matching `*://*/*`) runs `exposeActions`,
`exposeFrameProbe` and `exposeCues` in any frame it is loaded into, and the recorder, monitor,
indicator, rail, toast and hands-free orb only in the top frame. `exposeActions()`
(`actions/host.ts`) first asks the background whether its own address is blocked (`siteRefusal`:
the matcher is `vexdom-pure` in ext-core's wasm, which content scripts do not load), then
`dispatch()` (`actions/dispatch.ts`) parses the input with the action's zod schema and runs
`execute()`.

### Self-healing injection

A tab that loaded before the extension has no content script, so `tabs.sendMessage` fails with
`Receiving end does not exist`. `invokeInFrame()` then injects the manifest's content script with
`scripting.executeScript` and:

- retries at once for the reads `getPageInfo`, `snapshot`, `readHtml`, `extractText`,
  `waitForElement` and `navigate`;
- for anything else returns `TAB_UNREACHABLE`, telling the caller to look at the page again first,
  because its targets were chosen on a page it has not seen.

Pages that refuse content scripts (`chrome://`, the Web Store, the browser's own pages) stay
unreachable; `invokeForHarness` answers `TAB_UNREACHABLE` for any tab that is not an http(s) page.
`navigate` still works there, through `tabs.update` (`navigateViaTabsApi`).

---

## Frames

Each frame is snapshotted by its own content script, and the background stitches the trees
(`src/lib/bridge/snapshot-frames.ts`):

1. `childFrames()` lists every frame with `scripting.executeScript({allFrames: true})` and injects
   the content script where none answers, so every frame is listening.
2. The top frame's snapshot names each child frame by a **probe nonce**: the parent posts a fresh
   UUID into the frame element's window (`announceFrame()` in `src/lib/frames/probe.ts`), and the
   child's content script keeps it (`src/lib/frames/host.ts`).
3. `claim()` asks every frame for the nonce it holds, a few rounds 50 ms apart; a nonce held by
   exactly one frame names that frame id. This works on Chrome for Testing, which has no
   `runtime.getFrameId` in content scripts, and on Firefox alike.
4. The child's own snapshot is sealed against its own origin and hung in the slot, up to four
   levels deep, and the whole tree goes back with its `tabId`.

The Bridge renders that tree with refs (`crates/vexdom-snapshot`), so each ref carries its frame
id. An action whose target is a ref goes straight to that frame (`refFrameOf()`), whichever frame
has the focus. `page.switchFrame` moves the focus with the same nonce claim
(`src/lib/bridge/frames.ts`); the focused path is kept per tab in `storage.session` under
`vexdom/frameFocus` (`frame-focus.ts`), and blocked-sites checks cover every frame on it. A frame
that cannot be entered (sandboxed without scripts, browser-internal, not loaded) is
`FRAME_UNREACHABLE`.

---

## The debugger hub

Every feature that needs `chrome.debugger` shares one attachment per tab, in
`src/lib/bridge/cdp.ts` (ADR-030):

- `acquire(tabId, owner)` takes a **lease** named by its owner. The first lease attaches
  (protocol 1.3); later ones share it. `release(tabId, owner)` ends one lease; the last one turns
  frame auto-attach off and detaches.
- `withDebugger()` is a one-action lease (`oneshot:<uuid>`): attach or share, run, release.
- Long-lived owners keep their lease across calls: `diagnostics:<id>`, `route`, `har`, `dialog`,
  `emulation`, `init`, `record`, `trace`, `keep`, `react`, `mouse`.
- Attach and detach for one tab run one at a time. Chrome taking the debugger away (DevTools, the
  infobar's Cancel) or the tab closing ends every lease at once, and `onLost` tells each owner to
  drop its state; a later call on that feature answers `DEBUGGER_DETACHED`. An attach that fails is
  `DEBUGGER_UNAVAILABLE` (close DevTools for the tab and retry).
- On Firefox `acquire` answers `UNSUPPORTED` with the caller's hint.

Chrome shows its "is debugging this browser" bar for as long as any lease is held.

| Module | Owner | Actions |
| --- | --- | --- |
| `trusted-input.ts` | one-shot, `mouse` | `trustedClick`, `dragElement`/`pressKey`/`typeText` with `trusted: true`, `mouse` |
| `network-route.ts`, `har.ts` | `route`, `har` | `routeNetwork`, `unrouteNetwork`, `harStart`, `harStop` |
| `dialogs.ts` | `dialog` | `dialog` |
| `emulation.ts`, `devices.ts` | `emulation` | `emulate` |
| `init-scripts.ts` | `init` | `initScript` |
| `tab-keep.ts` | `keep` | `tabKeep` |
| `capture.ts` | one-shot, `trace`, `record` | `pdf`, `traceStart`/`traceStop`, `recordStart`/`recordStop` (frames sent to the Bridge with `recordFrame`, encoded there by ffmpeg) |
| `devtools.ts` | one-shot, `react` | `vitals`, `react`, `a11yAudit` |
| `diagnostics.ts` | `diagnostics:<id>` | `startDiagnostics`, `readConsole`, `readNetwork`, `stopDiagnostics` |
| `captcha.ts` | one-shot | `findCaptcha`, `solveCaptcha` |
| `code-toolkit.ts` | one-shot | `injectCode`, `runCode` |
| `screenshot.ts` | one-shot | `screenshot` of a tab that is not in front (a tab in front is captured without the debugger) |

Feature guides: [Network](../guide/features/network.md), [Emulation](../guide/features/emulation.md),
[Capture](../guide/features/capture.md), [DevTools](../guide/features/devtools.md),
[Diagnostics](../guide/features/diagnostics.md), [Captcha](../guide/features/captcha.md).

---

## The Firefox build

`bun run build:firefox` writes `dist/firefox-mv2`. Its manifest drops `sidePanel`, `debugger`,
`offscreen`, `userScripts`, `topSites` and `favicon` (Firefox has no use for most of them, and
addons.mozilla.org flags names it does not know), lists `topSites` as optional (Vexdom Home asks
for it), and carries `browser_specific_settings.gecko`: `id: vexdom@nguyenthdat.github.io`,
`strict_min_version: 140.0`, an `update_url` pointing at `updates.json` in the latest GitHub
release, and `data_collection_permissions` `websiteContent`. The sidebar is `sidebar_action`, and
the `chromiumOnly` actions are left off the list it offers ([registry](registry.md)).

Both builds allow `'wasm-unsafe-eval'` for ext-core; Chrome needs version 116 or later.

---

## The panel

**Toolbar button.** On Chromium, `serveLaunchers()` (`launchers.ts`) calls
`openPanelFromToolbar()` (`side-panel.ts`), which sets `openPanelOnActionClick`, so the button
opens and closes the side panel. Where that is missing, the button falls back to the
`launcher.html` popup. On Firefox it toggles the sidebar.

**Tabs.** The panel's strip is `PANEL_TABS` (`src/lib/rail/events.ts`): Chat, History, Skills,
Recordings, Schedules (`tasks`).

**Settings** are the options page (`entrypoints/options/`), opened with
`runtime.openOptionsPage()`, with seven sections: Extension, Profile, Guardrails, Blocked sites,
Agent, Connection, About. The guardrail policy and the theme live in the Bridge's
`~/.vexdom/config.json` and reach the page through `preferences`/`setPreference`
(`preferences.ts`); the theme is also mirrored in `storage.local` so every surface paints before
the socket exists.

**Blocked sites** (`vexdom/blockedSites` in `storage.local`) never leave the browser: no message
carries the list to the Bridge. It is enforced in `guardTarget`/`guardLanding` in `invoke.ts`, in
the content script's own check, and through `site-guard.ts` for everything that does not pass the
gate (scheduled tabs, saved tools, A-Eye, the recorder, monitors, diagnostics). See
[Blocked sites](../guide/features/blocked-sites.md).

**Rail, orb and cues.** Nothing can resize a side panel, so minimizing closes it and draws a 44 px
rail into the page (`src/lib/rail/host.ts`, served by `bridge/rail.ts`), in a closed shadow root.
Hands-free mode does the same with a microphone orb (`src/lib/handsfree/`, `bridge/hands-free.ts`),
with speech recognition in an offscreen document on Chromium. Action cues ring the element each
agent action touches (`bridge/action-cues.ts`, `src/lib/cues/plan.ts`). All overlays carry
`data-vexdom-overlay`, so A-Eye and trusted input skip them. See
[Hands-free](../guide/features/hands-free.md) and [Action cues](../guide/features/action-cues.md).

**Vexdom Home** (`entrypoints/newtab/`) is the new tab page; see [New tab](../guide/features/new-tab.md).

---

## Tab scoping

A panel conversation is bound to its tabs. The background keeps a registry in `storage.session`
under `vexdom/tabSessions` (`src/lib/bridge/tab-sessions.ts`). Each session has a main tab
(`mainTabId`), every tab it spans (`tabIds`), the tabs the user put there (`userTabIds`), the tabs
its runs opened (`openedTabIds`), the tab its next action lands on (`currentTabId`), and its
running turn, if any.

Every `Page.invoke` for a run carries the run id, and `resolveTab()` sends it to the owning
session's current tab. So a run keeps working in its own tab while you browse elsewhere, and two
conversations in two tabs act independently. Calls with no run (MCP clients, the CLI) target the
`tabId` they name, else the active tab of the current window.

| Situation | Behaviour |
| --- | --- |
| You add tabs (`addTabs` on the run port, or the right-click item) | Joined to `userTabIds` |
| A run opens a tab or window (`openTab`, `openWindow`) | Adopted into the session and recorded in `openedTabIds` |
| `switchTab` onto a tab no session holds | Adopted |
| `switchTab` onto a tab another session owns | `TAB_IN_USE` |
| Every tab of the session is gone | `SESSION_TAB_CLOSED` |
| A ninth active conversation | `SESSION_LIMIT` (`MAX_ACTIVE_SESSIONS` is 8) |

When a turn starts in a session that spans more than one tab, the run port sends the Bridge the
roster in `context.tabs` (only to a Bridge that speaks `TABS_PROTOCOL`, 26): each tab's id, sealed
title, host, which is current and which you chose; tabs on blocked sites are left out. Closing the
main tab hands the session to an heir (`heirsOf()`) or ends it; closing the side panel does not end
anything, because the tab is the anchor.

---

## Scheduled runs

The Bridge owns the schedule (`crates/vexdom-daemon/src/scheduler.rs`,
`crates/vexdom-core/src/schedules.rs`). A due task arrives as `Page.runTask`. `startTaskRun` in
`run-port.ts` opens a background tab and tags its session with the task; an instruction then runs
like a typed one, and a recording replays step by step through `invokeForHarness`. When it
settles, `finishTask` reports `taskDone` and files the transcript under `vexdom:taskRuns`. See
[Scheduling](../guide/features/scheduling.md).

---

## Waking and pairing through the native host

When no port answers, `wakeDaemon()` (`native-wake.ts`) asks the browser to run the
`com.vexdom.bridge` host that `vexdom setup` registered, which runs `ensure`. While
`vexdom stop`'s marker is in the state folder, the host starts nothing.

`auto-pair.ts` pairs without a code: `native-pair.ts` sends the host `pair`, and the grant it
returns is redeemed by `redeemGrant()` in `socket.ts` through the same handshake a typed code uses
([Transport](transport.md#pairing-without-a-code-grants)). It tries only while there is no usable
session key, spaced by `isDue()`, and never unasked once the browser was revoked or denied.

---

## Updates wait for the panel

A live Bridge link keeps the extension from ever being idle, so a store update would wait for the
next browser start. `extension-update.ts` holds it instead: `runtime.onUpdateAvailable` stores the
version under `vexdom/waitingUpdate`, the panel shows an **Update** pill while no run or recording
is going, and **Update** calls `runtime.reload()`. A Bridge newer than the extension triggers one
`runtime.requestUpdateCheck()` per Bridge version (`vexdom/updateCheckedFor`; Chrome only). An
unpacked copy compares the version in its folder's `/manifest.json`.

---

## Next

**[Agent runs →](agent-runs.md)**: the side panel's path, where an instruction becomes a Claude
Code turn.
