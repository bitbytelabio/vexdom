---
name: page-state
description: Save a tab's cookies and web storage under a name and load them back later to skip a login or reset a test, and compare a page with an earlier snapshot, a saved screenshot or another URL.
---

# Saved state and diffs

## Save and restore a session

`act {"do":"state","op":"save","name":"work"}` keeps the tab's cookies and localStorage /
sessionStorage under a name (letters, digits, `-`, `_`). Values that read as credentials are sealed
in the file: they can be released only into the browser session that sealed them.

`act {"do":"state","op":"load","name":"work"}` puts them back — the tab first goes to the saved
origin — then reload or navigate to see the signed-in page. Confirm it worked (a heading only a
signed-in user sees); a server-side session that expired stays expired.

- `{"op":"list"}` — the saved states.
- `{"op":"show","name":"work"}` — counts only, never values.
- `{"op":"rename","name":"work","newName":"work-old"}`, `{"op":"clear","name":"work"}` (or
  `"all":true`), `{"op":"clean","olderThanDays":30}`.

Use it to log in once and reuse the session across tasks, or to reset a test to a known starting
point. Loading a state replaces the site's current cookies: say so when the user is signed in as
someone else. For logging in from scratch, see the `auth` playbook.

## What changed

- `act {"do":"diff","kind":"snapshot"}` — the accessibility tree now against the last snapshot of
  the tab: what appeared, disappeared or changed text, refs ignored. `selector` limits it to one
  element; `compact`, `depth` as for a snapshot. Use it after a long wait or a background update,
  when you did not act yourself and so got no delta.
- `act {"do":"diff","kind":"url","urls":["https://staging.example/x","https://example.com/x"]}` —
  two pages' trees side by side; `screenshot:true` compares pixels too (`full` for whole pages).
  Good for staging against production, or before and after a deploy.
- `act {"do":"diff","kind":"screenshot","baseline":"before.png"}` — pixels against a saved image,
  with the share of pixels that differ; `threshold` (0–1, default 0.1) is the colour distance
  under which two pixels count as equal.
- `baseline` for `snapshot` or `screenshot` is a file in `~/vexdom/captures/`; nothing outside
  that folder is read.

Report the change in words ("the price changed from $12 to $15; a 'Sold out' badge appeared"), not
the raw diff.

## From a shell

`vexdom state save|load|show <name>`, `vexdom state list`, `vexdom state rename <old> <new>`,
`vexdom state clear <name>|--all`, `vexdom state clean --older-than 30`;
`vexdom diff snapshot [--baseline FILE]`, `vexdom diff screenshot --baseline FILE [-o diff.png] [--threshold 0.1]`,
`vexdom diff url <u1> <u2> [--screenshot]`. Make a baseline with
`vexdom snapshot --full > ~/vexdom/captures/before.txt` or
`vexdom screenshot -o ~/vexdom/captures/before.png`.
