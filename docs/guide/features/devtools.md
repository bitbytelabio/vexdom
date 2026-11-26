# DevTools

Measure and inspect a page the way Chrome DevTools does: a performance trace or CPU profile, Core
Web Vitals, a React app's component tree, re-renders and Suspense boundaries, and an axe-core
accessibility audit. Chromium only.

```sh
vexdom vitals --reload
vexdom trace start
… use the page …
vexdom trace stop
vexdom a11y --tags wcag2a,wcag2aa
```

---

## Traces and profiles

| CLI | MCP |
| --- | --- |
| `vexdom trace start` | `act {"do":"traceStart","mode":"trace"}` |
| `vexdom profiler start [--categories csv]` | `act {"do":"traceStart","mode":"profile","categories":[…]}` |
| `vexdom trace stop [path]` / `vexdom profiler stop [path]` | `act {"do":"traceStop","path":"/abs/file.json"}` |

`trace` records everything, for chrome://tracing or Perfetto. `profile` records JavaScript samples
and the timeline with a default set of categories, or the `categories` you give.

`traceStop` writes JSON through the Bridge: to the absolute `path`, or to
`~/vexdom/captures/trace-<unix_ms>.json` (`profile-…json` for a profile). The answer has `mode`,
`eventCount`, `path` and `bytes`; `eventsDropped: true` means the 5,000,000-event cap was hit.

If Chrome refuses tracing in profile mode, Vexdom falls back to the V8 CPU profiler: the start
answers `profiler: "cpu"` and the stop writes a `.cpuprofile`. When neither works, or a `trace`
is refused, the start fails with `TRACING_UNAVAILABLE`.

| Code | When |
| --- | --- |
| `TRACE_ACTIVE` | A trace or profile is already running on this tab |
| `NO_TRACE` | `traceStop` with nothing running |
| `TRACING_UNAVAILABLE` | The debugger refused to trace (and to profile) |
| `DEBUGGER_DETACHED` | The trace was lost with the debugger before the stop |
| `ARTIFACT_TOO_LARGE` | The file would exceed 48 MiB |

---

## Core Web Vitals

| CLI | MCP |
| --- | --- |
| `vexdom vitals [--reload]` | `act {"do":"vitals","reload":true}` |

Reports TTFB, LCP, CLS (score and entries), FCP and INP, plus React hydration timing (phases and the
slowest hydrated components) when the page runs a React profiling build. `reload: true` reloads first
and waits for the load to settle, so the whole load is measured. Without it you get what the page
has reported so far.

---

## React

| CLI | `act {"do":"react", …}` |
| --- | --- |
| `vexdom react install` | `"op":"install"` |
| `vexdom react tree [--json]` | `"op":"tree"` |
| `vexdom react inspect <id>` | `"op":"inspect","fiberId":42` |
| `vexdom react renders start` / `renders stop [--json]` | `"op":"rendersStart"` / `"op":"rendersStop"` |
| `vexdom react suspense [--only-dynamic] [--json]` | `"op":"suspense","onlyDynamic":true` |

`install` adds React DevTools' global hook to every load of the tab and reloads it; it is needed once
per tab, and every other op fails with `REACT_HOOK_MISSING` until it has run. The hook stays
installed while the tab's `react` lease is held, so the debugger bar stays up.

The Bridge turns the raw answers into text reports (a component tree, a re-render report, a Suspense
report); `json: true` returns the data instead.

---

## Accessibility audit

| CLI | MCP |
| --- | --- |
| `vexdom a11y [--tags csv] [--selector s]` | `act {"do":"a11yAudit","tags":["wcag2a","wcag2aa"],"selector":"main"}` |

Runs [axe-core](https://github.com/dequelabs/axe-core) in the page and reports each broken rule: id,
impact, help text and link, how many elements, and up to five of them. It also counts passes,
incomplete and inapplicable rules, and `skippedFrames` (frames from other origins are not audited).

axe-core is not bundled. On first use the Bridge fetches axe-core 4.12.1 from the npm registry,
checks the tarball against the registry's sha512 integrity, and keeps `axe.min.js` in
`~/.vexdom/cache/axe-core-4.12.1.min.js`. Later audits use the cached copy. If the fetch or the check
fails, the audit fails with `AXE_UNAVAILABLE`.

---

## The debugger

`vitals`, `a11yAudit` and the React reads take the tab's debugger for one call. A trace or profile
holds the `trace` lease until it stops, and `react install` holds the `react` lease. Chrome shows
"Vexdom is debugging this browser" while any lease is held.

| Code | When |
| --- | --- |
| `UNSUPPORTED` | Firefox |
| `DEBUGGER_UNAVAILABLE` | DevTools is open on the tab, or Chrome refused to attach |

---

## The playbook

The `devtools` playbook (`crates/vexdom-core/assets/skills/devtools/SKILL.md`) is served as the MCP
prompt `devtools`, the resource `skill://devtools/SKILL.md`, and through
`guide {"topic":"devtools"}`.

---

## See also

- [Diagnostics](diagnostics.md): console errors and failed requests
- [Theming](theming.md): the WCAG contrast audit, which needs no debugger
- [Capture](capture.md): PDFs and video, written the same way as traces
