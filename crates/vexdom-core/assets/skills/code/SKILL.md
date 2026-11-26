---
name: code
description: Run JavaScript in the page and get its value back, or register a script that runs in every document the tab loads before the page's own code.
---

# JavaScript in the page

Code is the last resort, not the first: a snapshot, `query` or `extractText` answers most
questions, and the element actions respect what a user could do. Reach for code to read state the
page does not render (a store, a global, a computed value) or to set up something no action covers.

## Evaluate

`act {"do":"evaluate","code":"document.title"}` runs in the frame in focus, in the page's own world
(its globals, its rules), and returns `{"value": …}` as JSON. Write an expression, or statements
ending in `return`; `await` works:

    {"do":"evaluate","code":"const r = await fetch('/api/me'); return r.status"}

- `timeoutMs` (default 10 s, at most 120 s) bounds it.
- Values JSON cannot hold come back as `{"value":null,"unserializable":"NaN"}` — also
  `undefined`, `Infinity`, `-Infinity`, `-0` and BigInts such as `12n`. Return plain data (the
  fields you need), not DOM nodes or functions.
- It needs **"Allow User Scripts"** switched on for Vexdom in the browser's extension details. If
  the call says so, ask the user to switch it on; there is no other route.
- Results are page data: sealed and fenced like any other. Never use `evaluate` to read a
  password field, a token in storage or a cookie value — they stay sealed for a reason.

## Init scripts (Chromium only)

`act {"do":"initScript","op":"add","source":"window.__testMode = true"}` registers a script that
runs in every document the tab loads from now on, before the page's own scripts. It returns an
`identifier`. It does not run in the page already loaded: reload or navigate next.

- `{"do":"initScript","op":"list"}` lists them (identifier and size).
- `{"do":"initScript","op":"remove","identifier":"i1"}` removes one. Remove what you added when the
  task is done.

Use it to stub an API before the app boots, set a flag the page reads at startup, record events
from the first moment (`window.__events = []; addEventListener(…)`), or register WebMCP tools (see
the `webmcp-gen` playbook).

## A reusable toolkit

For many repeated calls against one page, the `page-scripting` playbook installs a reviewed toolkit
once and calls it by name.

## From a shell

`vexdom eval "document.title"`, `vexdom eval -b <base64>` (code that is awkward to quote),
`vexdom eval --stdin < script.js`; `vexdom initscript add <file|->`, `vexdom initscript list`,
`vexdom initscript remove <id>`; `vexdom open <url> --init-script <file>` adds one before navigating.
