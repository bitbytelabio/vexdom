---
name: devtools
description: Measure and inspect a page like DevTools — Core Web Vitals, a performance trace or CPU profile, a React component tree with re-renders and Suspense, and an axe-core accessibility audit.
---

# DevTools: vitals, traces, React, accessibility

All of these run only in a Chromium browser and attach the debugger ("Vexdom is debugging this
browser" shows while they run). Lead your answer with the finding — "LCP is 4.1 s, the hero image
loads last" — and quote numbers exactly; the raw data is supporting evidence.

## Core Web Vitals

`act {"do":"vitals","reload":true}` reloads, waits for the load to settle and returns `ttfb`, `lcp`,
`cls` (score and the shifts behind it), `fcp`, `inp`, plus React hydration timing when the page
ships a profiling build. Without `reload` it measures what the current page has recorded so far —
`inp` needs real interactions, so act on the page first. Thresholds that read as good: LCP ≤ 2.5 s,
CLS ≤ 0.1, INP ≤ 200 ms.

## Trace or profile

1. `act {"do":"traceStart"}` (mode `trace`: everything, for chrome://tracing or Perfetto) or
   `{"do":"traceStart","mode":"profile"}` (JavaScript samples and timeline). `categories` narrows a
   trace.
2. Do the slow thing: a reload, a click, a scroll.
3. `act {"do":"traceStop","path":"/abs/path/trace.json"}` writes the JSON (else into
   `~/vexdom/captures/`).

`TRACE_ACTIVE`: one is already running, stop it first. `NO_TRACE`: nothing to stop.
`TRACING_UNAVAILABLE`: the browser refused tracing through the extension; a profile then falls back
to the JavaScript profiler and writes a `.cpuprofile` instead. Keep the window short (seconds): traces
grow fast. The file is for the user's tools; summarise what you can from the vitals and the page.

## React

1. `act {"do":"react","op":"install"}` adds the DevTools hook and **reloads the tab** (needed once
   per tab; say so — unsaved input is lost). Without it every other op answers `REACT_HOOK_MISSING`.
2. `{"do":"react","op":"tree"}` — the component tree with ids (`json:true` for nodes as data).
3. `{"do":"react","op":"inspect","fiberId":42}` — one component's props, hooks and state.
4. `{"do":"react","op":"rendersStart"}`, act on the page, `{"op":"rendersStop"}` — which components
   re-rendered and how often: the usual answer to "why is this slow when I type".
5. `{"do":"react","op":"suspense"}` — Suspense boundaries and what each waits on
   (`onlyDynamic:true` keeps the ones still waiting).

Props and state are page data: credentials in them come back sealed.

## Accessibility audit

`act {"do":"a11yAudit"}` runs axe-core on the page (`selector` for one region, `tags` such as
`["wcag2a","wcag2aa"]` for a standard). The result lists each broken rule with its impact, help
link, count and the first few offending selectors, plus `skippedFrames` — frames from other origins
are not audited; say so when it is above zero. The first audit downloads axe-core into the Bridge's
cache; `AXE_UNAVAILABLE` means that failed (offline, registry down).

Report by impact (critical, serious first), name the elements, and give the fix the rule's help
names. Colour contrast in depth is the `page-theming` playbook's job.

## From a shell

`vexdom vitals [--reload]`, `vexdom trace start [--categories csv]`, `vexdom trace stop [path]`,
`vexdom profiler start`, `vexdom profiler stop [path]`, `vexdom react install`,
`vexdom react tree [--json]`, `vexdom react inspect <fiberId>`, `vexdom react renders start`,
`vexdom react renders stop [--json]`, `vexdom react suspense [--only-dynamic]`,
`vexdom a11y [--tags wcag2a,wcag2aa] [--selector s]`.
