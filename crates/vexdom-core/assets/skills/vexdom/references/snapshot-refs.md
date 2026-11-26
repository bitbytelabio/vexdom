# Snapshots and refs

- A ref (`e5`) names one element in one document of one tab. vexdom keeps a map per tab: the same
  element keeps its number across snapshots; a new element gets the next number; numbers are never
  reused. `"e5"`, `"@e5"`, `"ref=e5"` and `{"ref":"e5"}` all name it.
- Only refs present in the latest snapshot of a tab are accepted (`UNKNOWN_REF` otherwise). A ref
  from a page that has since navigated is refused as `STALE_REF`.
- If the exact element is gone but exactly one element with the same role and name exists, the
  extension uses that one. With duplicates (`nth=…` in the tree) it refuses instead.
- Deltas: after the first snapshot, `snapshot {}` returns only the lines that changed, as one
  splice: `lines a-b replaced by:`, `inserted after line n:` or `lines a-b removed`. A URL change,
  different options or `full:true` return the whole tree again.
- Actions that change the page (click, fill, type, insert text, select, check, press, submit,
  navigate, push a route, mouse, answer a dialog) wait briefly and answer with the delta they
  caused, so the next step can use refs straight away.
- Frames: refs inside iframes work like any other; the tree shows the frame's content under its
  `Iframe` node. `act {"do":"switchFrame","frame":"e13"}` makes later actions run inside it;
  `{"do":"switchFrame","to":"top"}` steps back out.

## From a shell

`vexdom snapshot` prints the same tree and deltas; `--json` prints `{text, refs:[{ref, role, name}]}`.
Element commands take `@e5` (or a CSS selector) and print the delta they caused.
