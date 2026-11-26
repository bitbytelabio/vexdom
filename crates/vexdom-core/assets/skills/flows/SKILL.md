---
name: flows
description: Record a task once while doing it in the browser, save it as a flow of direct requests with parameters, and rerun it later in one call instead of clicking through the page again.
---

# Saved flows

Driving the page is right the first time and slow the hundredth. A flow keeps the site's own API
requests behind a task (a search, a lookup, a list) and reruns them from the user's tab, with the
recorded clicks as a fallback. Experimental: a run can fail on a site that changed; then continue
in the UI.

## Run one that exists

`act {"do":"flowList"}` (or `origin:"shop.example"`) lists saved flows with their params and whether
the last run verified. When one matches the task, run it instead of driving the page:

    act {"do":"flowRun","name":"search-items","params":{"q":"boots"},
         "where":{"brand":"Acme"},"fields":["name","price"],"limit":5}

- `params` fills the flow's parameters; `where` keeps items whose field contains the value (case-insensitive);
  `fields` keeps only those fields (dotted paths work); `limit` caps the items; `timeoutMs` bounds
  the whole run.
- The run sends the requests **from the user's tab**, with its cookies (the tab must be on the
  flow's site), and falls back to replaying the recorded actions. Cookie values never leave the
  browser.
- The answer says `verified:true` when the response still has the recorded shape. Unverified or
  `FLOW_DRIFTED`: the site changed — do the task in the UI and consider recording again.

## Record and save

1. Open the page where the task starts and `act {"do":"flowRecord","op":"start"}`. It records the
   tab's requests (the debugger bar shows) and every action you run on that tab.
2. Do the task once with ordinary actions — ideally twice with different inputs, so the values that
   vary are clear.
3. `act {"do":"flowRecord","op":"stop"}`, then `act {"do":"flowSteps"}` lists what was recorded:
   step number, action, URL, how many requests, ok or failed.
4. `act {"do":"flowSave","name":"search-items","params":{"q":"shoes"},"description":"Search the catalog"}`
   keeps the first-party JSON requests, and turns each value named in `params` (the value you
   typed) into a parameter. `from`/`to` save a range of steps; `output` picks the request whose
   response is the result. The report lists the params it found on its own and the steps it could
   not turn into requests, with the reason.

`flowRecord {"op":"status"}` says whether a recording runs. `FLOW_NOT_CAPTURING`: start one first.
Cookies, `authorization` and token or CSRF headers are never saved; credential values in URLs are
saved as `<redacted>`.

## Writes

A flow that changes something (a POST that orders, posts or deletes) does not send the write on its
own: the run asks the user to approve it, and waits. `allowWrite:"all"` (or a list of request ids)
sends writes without asking — only when the user asked for exactly that.

## When a run stops

- `AUTH_EXPIRED`: the user is signed out; ask them to sign in, then rerun.
- `CREDENTIAL_NOT_SENDABLE`, `REQUEST_SIGNED`: this request cannot be replayed; use the page.
- `RATE_LIMITED`: wait before rerunning.
- `BLOCKED_BY_SITE`: a bot check answered; continue in the UI or let the user clear it.
- `OUTCOME_UNKNOWN`: a write was sent and no answer came; check the site, do not resend.
- `WRITE_APPLIED`: the write went through with a different answer; do not repeat it.
- `FLOW_NOT_FOUND`: check `flowList`.

## From a shell

`vexdom flow record start|stop|status`, `vexdom flow steps`,
`vexdom flow save <name> [--from n] [--to n] [--param q=shoes]... [--output id] [--description t]`,
`vexdom flow run <name> [--param q=boots]... [--where brand=Acme] [--fields name,price] [--limit 5] [--allow-write all|id,id] [--origin h]`,
`vexdom flow list [--origin h]`.
