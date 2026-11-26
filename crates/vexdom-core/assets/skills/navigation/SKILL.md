---
name: navigation
description: Move around the browser — tabs with labels, new windows, iframes, single-page-app routes, background tabs kept live or frozen — and wait for the page to get where you sent it.
---

# Navigation and waiting

Every action runs on the tab vexdom is working in: the active tab, or the one the last `openTab`,
`switchTab` or `openWindow` moved to. `tabId` on any action reaches another tab without switching.

## Pages

- `act {"do":"navigate","url":"https://…"}` loads a URL; `{"do":"navigate","action":"back"}`
  (`forward`, `reload`) walks history. The result carries the new tree.
- `act {"do":"pushState","url":"/settings"}` changes a single-page app's route without a reload
  (Next.js's router when present, else `history.pushState` plus `popstate`). Same origin only; use
  `navigate` to leave the site. Use it when a reload would lose state the user built up.

## Tabs and windows

- `act {"do":"switchTab"}` with no input lists the open tabs with their ids, titles, URLs and labels.
- `act {"do":"openTab","url":"https://docs.example.com","label":"docs"}` opens a tab and works in
  it (`active:false` opens it behind). A label is letters, digits, `-` and `_`, starts with a
  letter, is unique, and is never `t1`-style.
- `act {"do":"switchTab","label":"docs"}` — or `tabId`, or `match` (a label first, then title or
  URL text) — brings a tab to the front and works in it.
- `act {"do":"closeTab","label":"docs"}` closes one; with no input it closes the current tab and
  follows the browser to whichever tab comes forward.
- `act {"do":"openWindow","url":"https://…"}` opens a new window and works in its tab.

Label tabs when a task juggles several sites; refs are per tab, so snapshot after switching.

## Frames

`act {"do":"getPageInfo"}` lists the frames a document embeds. `act {"do":"switchFrame","frame":"e13"}`
makes every later action run inside that iframe; `{"do":"switchFrame","to":"parent"}` or
`"to":"top"` steps back out. A snapshot already shows iframe content with its own refs, so you only
need `switchFrame` for actions that work on "the page" (`evaluate`, `waitFor`, site tools).

## Background tabs (Chromium only)

`act {"do":"tabKeep","mode":"live","tabId":12}` keeps a tab behind another running at full speed —
timers, animations and focus as if in front — for a dashboard or a job you watch from another tab.
`"frozen"` pauses a background tab completely (it refuses the visible tab: `TAB_VISIBLE`); any later
action on it thaws it first. `"throttled"` hands it back to the browser's normal throttling.

## Waiting

- `act {"do":"waitFor","text":"Order placed"}`; also `url` (a glob: `**` any path, `*` one
  segment, none means "contains"), `load` (`load`, `domcontentloaded`, `networkidle`) and `fn`
  (a JavaScript expression that must turn truthy). Every condition given must hold; `timeoutMs`
  defaults to 10 s, at most 60 s.
- `act {"do":"waitForElement","target":"e5","state":"visible"}` (`attached`, `hidden`,
  `detached`) waits for one element.

Wait on what you expect to see rather than for a fixed time. After a click that navigates, wait on
the new URL or a heading, never on something the old page also had.

## From a shell

`vexdom open <url>`, `vexdom back|forward|reload`, `vexdom pushstate /settings`,
`vexdom tab` (list), `vexdom tab new --label docs <url>`, `vexdom tab docs`, `vexdom tab close docs`,
`vexdom tab keep live|throttled|frozen <tab>`, `vexdom window new [url]`, `vexdom frame @e13`,
`vexdom frame main`, `vexdom wait --text "Order placed"`, `vexdom wait --url "**/checkout/*"`,
`vexdom wait --load networkidle`, `vexdom wait --fn "window.ready === true"`,
`vexdom wait @e5 --state hidden`.
