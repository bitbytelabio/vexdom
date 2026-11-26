# Capture

Save the page as a PDF, or record a video of the tab, optionally with a contact sheet of the moments
the page changed. Chromium only. For still images see [Screenshots](screenshots.md).

```sh
vexdom pdf ~/Desktop/invoice.pdf
vexdom record start ~/Desktop/demo.mp4 --cursor --contact-sheet
… use the page …
vexdom record stop
```

---

## PDF

| CLI | MCP |
| --- | --- |
| `vexdom pdf [path] [--landscape] [--no-background] [--css-page-size]` | `act {"do":"pdf","path":"/abs/page.pdf","landscape":false,"printBackground":true,"preferCSSPageSize":false}` |

Chrome prints the page (`Page.printToPDF`) and the Bridge writes the file: to the absolute `path`,
or to `~/vexdom/captures/page-<unix_ms>.pdf`. The CLI resolves a relative path against your working
directory. Background colours and images are printed unless `--no-background`. The answer is
`{path, bytes}`.

The file is written with mode `0600`. A `path` must be absolute and outside the Bridge's own folder
(`~/.vexdom`), else `INVALID_INPUT`. Over 48 MiB fails with `ARTIFACT_TOO_LARGE`.

---

## Video

| CLI | MCP |
| --- | --- |
| `vexdom record start <path> [--fps n] [--cursor] [--contact-sheet] [--contact-sheet-threshold x]` | `act {"do":"recordStart","path":"/abs/demo.webm","fps":30,"cursor":false,"contactSheet":false,"contactSheetThreshold":0.05}` |
| `vexdom record stop` | `act {"do":"recordStop"}` |
| `vexdom record restart <path> …` | `recordStop` then `recordStart` |

The extension streams the tab's screencast as JPEG frames to the Bridge, which paces them to a
constant frame rate and pipes them into **ffmpeg**:

- `path` must be absolute and end in `.webm` (VP8) or `.mp4` (H.264).
- `fps` defaults to 30, clamped to 1–60.
- `cursor` draws the mouse pointer and its clicks into the video.

ffmpeg has to be on the Bridge's `PATH`. Without it `recordStart` fails with `FFMPEG_MISSING`
("install it with brew install ffmpeg") before the tab is touched.

`recordStop` waits for ffmpeg to finish the file and answers `{path, frames, durationMs}`. One
recording runs at a time per Bridge (`RECORDING_ACTIVE` for a second; `NO_RECORDING` when there is
nothing to stop).

### Contact sheet

`contactSheet` (or any `--contact-sheet-threshold`) also writes a PNG grid next to the video, named
`<stem>.contact-sheet.png`: the frames where at least `contactSheetThreshold` of the pixels changed
(default 0.05), with timestamps and the changed regions outlined, at most 100 frames. `recordStop`
then also reports `contactSheetPath` and `contactSheetFrames`.

---

## The debugger and errors

PDF takes the tab's debugger for one call. A recording holds the `record` lease until it stops, so
Chrome shows "Vexdom is debugging this browser" while it runs. If the debugger is taken away, the
screencast ends and `recordStop` writes what arrived.

| Code | When |
| --- | --- |
| `UNSUPPORTED` | Firefox |
| `DEBUGGER_UNAVAILABLE` | DevTools is open on the tab, or Chrome refused to attach |
| `FFMPEG_MISSING` | No `ffmpeg` on the Bridge's `PATH`, or it did not start |
| `RECORDING_ACTIVE` / `NO_RECORDING` | A recording is already running / none is |
| `ACTION_FAILED` | ffmpeg exited with an error or did not finish within 10 s |

---

## The playbook

The `capture` playbook (`crates/vexdom-core/assets/skills/capture/SKILL.md`) is served as the MCP
prompt `capture`, the resource `skill://capture/SKILL.md`, and through `guide {"topic":"capture"}`.
The `dogfood` playbook uses recordings as evidence in its bug reports.

---

## See also

- [Screenshots](screenshots.md): single images, annotated refs, pixel diffs
- [DevTools](devtools.md): traces and profiles, written the same way as a PDF
- [Recordings](recordings.md): a different thing — your own browsing, saved for the agent to replay
