# Files

Reading a file you attach in the side panel, uploading a file to a page, and keeping a file a page
downloads. Both directions to and from a page ask for your approval by default.

---

## Files you hand the side panel

Attach a file in the side panel: press **Attach a file** on the composer, or drop files anywhere on
the panel. The file belongs to **the conversation in that tab**.

Each file gets its own **file analyst**: a separate one-shot Claude Code session with no browser and
nothing to open but that one file. It writes a report and ends; its copy of the file is deleted.

| Kind | Up to |
| --- | --- |
| Text (CSV, JSON, Markdown, logs, code, anything in UTF-8) | 5 MB |
| PDF | 10 MB |
| Images (PNG, JPEG, GIF, WebP) | 5 MB |

The bytes decide the kind, not the name. Anything else (a program, audio, video, or a ZIP archive,
which Word, Excel and PowerPoint files also are) is turned away before the analyst starts.

| Verdict | The chip shows | |
| --- | --- | --- |
| **analyzed** | its summary | The report holds a summary, the file's outline, specifics and notes |
| **rejected** | the reason, in amber | Not a file Vexdom reads, or one that would not open. Retrying changes nothing |
| **failed** | the reason, in red, and **Retry** | The analyst timed out or could not start |

The report reaches the agent **once**, inside the next message you send in that conversation. The
agent never sees the file itself. It gets the report, plus two actions:

| | |
| --- | --- |
| `listFiles` | The stored files, with each one's summary |
| `attachFile` with `fileId` | Put one into an `<input type="file">` on the page |

---

## Uploading a local file

`attachFile` also takes `paths`: absolute paths of files on the computer running the Bridge. The
Bridge reads them and hands the bytes to the page.

| CLI | MCP |
| --- | --- |
| `vexdom upload <t> <file…>` | `act {"do":"attachFile","target":"e7","paths":["/abs/report.pdf"]}` |

The CLI resolves relative paths against your working directory and fails with `FILE_NOT_FOUND` for
a missing one. At most 20 files, 25 MB together (`FILE_TOO_LARGE`); nothing from the Bridge's own
folder (`~/.vexdom`) is ever attached.

---

## Files it takes off a page

`captureDownload` makes the page download a file and keeps it:

- with a **`target`**, it clicks (an "Export CSV" button) and waits for the download the click
  produces;
- with a **`url`**, it fetches the file **in the browser's own session**, with your cookies;
- with neither, it waits for the next download the tab starts.

| CLI | MCP |
| --- | --- |
| `vexdom download <t> [path]` | `act {"do":"captureDownload","target":"e9"}` |
| `vexdom wait --download [path] [--timeout MS]` | `act {"do":"captureDownload","timeoutMs":60000}` |
| `vexdom downloads` | `act {"do":"listDownloads"}` |

The bytes go to **`~/vexdom/download/`** at mode `0600`. The answer has `downloadId`, `name`,
`mime`, `size`, `notes` and `savedTo`; the CLI's `[path]` copies the file there as well. The agent
gets notes, never the bytes:

```
expenses-2026-08.csv — text/csv, 8.1 KB — 42 rows × 6 columns
```

`timeoutMs` defaults to 60 s.

---

## Download here, upload there

`attachFile` takes a `downloadId` wherever it takes a `fileId`, so a captured file can go straight
into another page, page → disk → page, never through the agent.

---

## What it will not keep

Each refusal **deletes the file the browser already wrote**.

| Code | |
| --- | --- |
| `DOWNLOAD_REFUSED` | Executables: `.exe`, `.dmg`, `.msi`, `.sh`, `.ps1`, `.apk`, `.jar` and the rest |
| `DOWNLOAD_TOO_LARGE` | Over 100 MB, measured on disk |
| `DOWNLOAD_OFF_SCOPE` | From a host the run was never pointed at |

Captures are swept after **14 days**.

```sh
vexdom downloads          # what has been captured, and where
vexdom downloads clear    # delete all of it
```

`downloadDir` and `downloadTtlDays` in [config](../configuration.md) move the folder and change the
expiry.

---

## Both directions are gated

| Rule | Action |
| --- | --- |
| `file-upload` | `attachFile`: putting a file into a page hands it to whoever runs that site |
| `file-download` | `captureDownload`: letting a page write a file to your disk |

Both are `confirm` by default. A CLI or MCP caller waits up to 120 s for your answer: in the desktop
app, with `vexdom approve c-… [--always]` / `vexdom deny c-…`, or through the MCP client's
elicitation prompt. No answer is `APPROVAL_TIMEOUT`; a refusal is `DECLINED`. See
[Approvals](../approvals.md).

---

## The playbook

The `downloads` playbook (`crates/vexdom-core/assets/skills/downloads/SKILL.md`) is served over MCP
as the prompt `downloads`, the resource `skill://downloads/SKILL.md`, and through
`guide {"topic":"downloads"}`.

---

## See also

- [Approvals](../approvals.md): the `file-upload` and `file-download` rules
- [Screenshots](screenshots.md) and [Capture](capture.md): the other files Vexdom writes
