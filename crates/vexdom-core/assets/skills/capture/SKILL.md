---
name: capture
description: Save the page as a PDF, or record a video of the tab (with an optional contact sheet of the moments it changed) as evidence or a demo.
---

# PDF and video

Both run only in a Chromium browser. Files go to the absolute `path` you give, else into
`~/vexdom/captures/`; the result reports where. Never write into `~/.vexdom`.

## PDF

`act {"do":"pdf","path":"/Users/…/invoice.pdf"}` prints the page as the browser would.
`landscape:true`; `printBackground` (default true) keeps background colours and images;
`preferCSSPageSize:true` uses the page's own `@page` size. To see what the print stylesheet does
first, use the `emulation` playbook (`media: {"type":"print"}`) and look with
`snapshot {"as":"image"}`.

## Video

1. `act {"do":"recordStart","path":"/Users/…/repro.webm"}` — `.webm`, `.mp4` or `.gif`; `fps` 1–60
   (default 30); `cursor:true` draws the pointer; `contactSheet:true` also writes a PNG grid of
   the frames where the page changed (`contactSheetThreshold` 0–1 tunes what counts as a change).
2. Do the steps at a human pace — a pause before and after each action, a longer one on the
   result — so the video can be watched at 1×. `typeText` reads better on video than `fillInput`.
3. `act {"do":"recordStop"}` → `{path, frames, durationMs, contactSheetPath?}`.

The Bridge encodes with ffmpeg: `FFMPEG_MISSING` means it is not installed (`brew install ffmpeg`);
tell the user. One recording at a time (`RECORDING_ACTIVE`); `NO_RECORDING` on stop means none was
running. Frames come only while the tab paints; a still page makes a short video. Stop the
recording even when a step fails.

The contact sheet is the quick way to check what the video shows without watching it.

Use a .gif path for a short shareable clip of what you did; keep it under ~30 s. It is made when
you stop (at most 12 fps, 960 px wide, a progress bar along the bottom), and every action you take
while recording is ringed and captioned in it.

## From a shell

`vexdom pdf [path] [--landscape] [--no-background] [--css-page-size]`,
`vexdom record start <path> [--fps n] [--cursor] [--contact-sheet] [--contact-sheet-threshold x]`,
`vexdom record stop`, `vexdom record restart <path>` (stop, then start again).
