# Background monitoring

A monitor watches a long job in a tab, such as an upload, a build or a deploy, and notifies you when
it finishes. The browser does the watching, so no agent spends tokens asking whether the job is done.

```
watch this upload and tell me when it's done
```

---

## How a monitor runs

Vexdom finds the progress signal, pins the tab, and watches it **in the background while you
work elsewhere**. It tracks the percentage, extrapolates an ETA from the sample history, notices when
progress has stalled, and notifies you on completion.

The watch runs in the browser, in the extension. It needs no further actions, and it keeps running
if the agent finishes, the MCP client disconnects, or Vexdom Bridge goes away. Only http(s) pages can
be watched; a browser page answers `UNSUPPORTED`.

---

## What it can watch for

| Condition | Completes when |
| --- | --- |
| `element-appears` | An element shows up |
| `element-vanishes` | An element goes away, usually a spinner |
| `text-matches` | Page text matches a regular expression |
| `title-matches` | The tab title matches one |
| `progress-reaches` | A progress bar hits a threshold (100 by default) |

`findProgress` picks the signal. It scans for progress bars with their current percentage,
percentage readouts in text, spinners and busy regions, each with a selector for `startMonitor`. If it
finds nothing, the page shows nothing measurable, and you are asked what completion looks like
instead of being given a watch on nothing.

| Action | Does |
| --- | --- |
| `findProgress` | Lists the progress signals on the page |
| `startMonitor` | Starts a watch on one condition |
| `monitorStatus` | Reports running and recently finished monitors |
| `awaitMonitor` | Waits until a monitor completes (see [From an MCP client](#from-an-mcp-client)) |
| `stopMonitor` | Ends a watch |

---

## Limits

| | |
| --- | --- |
| Concurrent monitors | 3 |
| Default duration | 30 minutes |
| Maximum duration | 4 hours |
| While it runs | The tab is pinned (and unpinned afterwards unless it was pinned before); you can work anywhere else |
| On completion | A card on the page you are on, plus the run's own report. While the browser is behind other windows or minimized, the [Mac app](../mac-app.md) can show it as a macOS notification instead (see [Where a finished notice shows](#where-a-finished-notice-shows)) |

Sampling is debounced and rate-limited (at most one sample every three seconds), with a five-second
backstop tick so a page that stops changing still gets checked.

---

## Where a finished notice shows

| The browser | The Vexdom app | Where it shows |
| --- | --- | --- |
| In front | Any | A card on the page you are on |
| Behind other windows, or minimized | Running, with notifications allowed | A macOS notification from the app, and a notice on its Overview tab whose **Open** button brings the tab back |
| Behind, or minimized | Not running, or notifications off | A card on the page, or the browser's notification when no page can show one |

The same goes for a scheduled task finishing and for a reminder. Allow notifications for Vexdom in
System Settings › Notifications.

---

## Stopping one

```
stop monitoring
```

This is an [instant command](instant-commands.md): it ends the watch without starting the agent. The
tab is unpinned and no notification is shown, since you asked for the stop.

---

## From an MCP client

An MCP client starts one with `act {"do":"startMonitor", …}` and then waits with
`act {"do":"awaitMonitor", …}`, which long-polls until the monitor completes: two minutes by
default, ten at most. A reply with `settled: false` means the poll window ended while the watch
continues: **call again**. That is normal, not an error. If the call fails with `EXTENSION_OFFLINE`,
the monitor is still running in the browser: reconnect and call again.

---

## See also

- [Scheduling](scheduling.md): when the page shows nothing to watch and the job must be re-done
- [Skills](skills.md): the `monitor-progress` skill routes these requests
- [internals/subsystems.md](../../internals/subsystems.md): how sampling works
- `guide {"topic":"startMonitor"}` over MCP, or `vexdom tools`, for every input
