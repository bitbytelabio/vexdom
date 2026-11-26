# Screenshots

Capture the viewport, the full scrolling page or one element. Captures an agent takes to see the page
go straight back to it; a file is written only when asked for.

| Caller | Form |
| --- | --- |
| MCP and the side panel | `snapshot {"as":"image"}`, with `target`, `fullPage`, `annotate`, `ifChanged`, `save`, `format`, `quality`, `tabId` |
| Any `act` caller | `act {"do":"screenshot", …}` (also `maxLongSide`, `filename`) |
| CLI | `vexdom screenshot [-o FILE] [--screenshot-dir DIR] [--screenshot-format png\|jpeg] [--screenshot-quality N] [--full] [-s <t>] [--if-changed [--threshold F]] [--annotate]` |

---

## Three kinds of capture

| | What you get | Cost |
| --- | --- | --- |
| **Viewport** (default) | What is on screen | One grab, well under a second |
| **Full page** (`fullPage`, `--full`) | The whole scroll view, stitched from viewport tiles | About a second per screenful |
| **One element** (`target`, `-s`) | That element's box | One grab |

Full-page capture scrolls in viewport steps and waits out the browser's capture rate limit between
each. It is capped at **48 tiles and a 16,384 px canvas side**; past that the bottom is cut off and
the result says `truncated: true`.

---

## Annotated and unchanged captures

`annotate: true` takes a snapshot first and draws each element's ref (`e5`) on it, so a picture and
the tree line up.

`ifChanged: true` compares the capture with the last one of that tab and answers
"Screenshot unchanged since the last one." instead of an image when they are identical. With a
`threshold` (0 to 1) a capture that differs by no more than that share of pixels also counts as
unchanged.

`vexdom diff screenshot --baseline FILE [-o OUT] [--threshold F]` compares against a saved image and
writes a diff picture. An agent's `diff` (MCP `act {"do":"diff",…}`) reads a baseline only from
`~/vexdom/captures`, and only a snapshot text or a PNG: the folder also holds HARs and traces,
which carry cookies.

---

## Where files go

| Caller | File |
| --- | --- |
| MCP / `act` | None by default. `save: true` writes `~/vexdom/screenshot/screenshot-<timestamp>.<ext>` (`filename` to name it) at mode `0600` and reports `savedTo` |
| CLI | Always a file: `-o FILE`, else `--screenshot-dir DIR`, else your temp folder as `vexdom-screenshot-<stamp>.<ext>`; prints the result with `savedTo` |

`screenshotDir` in [config](../configuration.md) moves the `save: true` folder. `./vexdom.json` can
set CLI defaults for format, quality and annotate.

---

## Format and size

JPEG at quality 80 by default; PNG for lossless output or transparency. The CLI picks PNG when `-o`
ends in `.png`. A viewport capture taken to look at the page comes back at the page's CSS-pixel size,
so a position in the picture is a usable click point; full-page, element and saved captures are
downscaled so the longest side is at most 1600 px. `maxLongSide` overrides either.

---

## Background tabs

While the tab is in front it is captured the ordinary way. Once it is behind, Chrome renders it
through its debugger for that one call, so the "Vexdom is debugging this browser" bar flashes up. If
DevTools is open on that tab the capture fails with `DEBUGGER_UNAVAILABLE`. Firefox captures a
background tab directly.

---

## See also

- [Capture](capture.md): PDFs and video
- [Theming](theming.md): measuring contrast beats looking at a picture
- [internals/subsystems.md](../../internals/subsystems.md): how tiles are stitched and who writes the file
