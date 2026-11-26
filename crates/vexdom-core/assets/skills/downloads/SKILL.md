---
name: downloads
description: Make a page download a file and keep it, find the files kept so far, and upload one to another page without its bytes ever passing through you.
---

# Downloads and uploads

Files move between pages through the Bridge, not through you: you get a path, an id and notes about
what arrived, never the bytes.

## Download

`act {"do":"captureDownload","target":"e6"}` clicks the link or button that starts the download and
keeps the file in the user's `~/vexdom/download/` folder. The result reports the path, the size, the
type and notes on what the file holds, plus a `downloadId`.

- `url` instead of `target` fetches a direct http(s) link with the browser's own cookies — the
  user's logged-in session, not an anonymous request. Prefer `target` when a button exists: many
  exports have no fetchable URL.
- With neither, it waits for the next download this tab starts — start the waiting call, then do
  whatever triggers it (a menu, a confirmation dialog) in the next call.
- `timeoutMs` (default 60 s, up to 10 min) covers slow exports.

Report where the file went and what the notes say it contains; do not claim more than the notes.

## Find kept files

`act {"do":"listDownloads"}` lists captured files, newest first, with their notes and paths;
`nameContains` narrows it. `act {"do":"listFiles"}` lists the files the user stored in Vexdom
themselves, with summaries.

## Upload

`act {"do":"attachFile","target":"e4","downloadId":"…"}` attaches a captured download to an
`<input type="file">`; `fileId` attaches a stored file; `paths:["/Users/…/cv.pdf"]` attaches local
files by absolute path. Download here, upload there: the file never passes through you.

If the upload control is a custom drop zone with the real `<input type="file">` hidden, target the
input itself (a snapshot with `interactive:false`, or `{"selector":"input[type=file]"}`).
Then submit the form the way the page expects and check the page shows the file.

## From a shell

`vexdom download @e6 [path]`, `vexdom wait --download [path]`, `vexdom upload @e4 ./cv.pdf`,
`vexdom downloads` (list), `vexdom downloads clear` (delete them all).
