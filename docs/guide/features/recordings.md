# Recordings

A recording captures a job you do yourself in one tab and saves it as named steps, so the agent can
repeat it later against the live page, or a [scheduled task](scheduling.md) can replay it.

[Site maps](site-maps.md) describe how a site is laid out; recordings describe a task you perform
on it.

---

## Recording

Press **Record** in the side panel's **Recordings** tab, do the job yourself (click through the
pages, fill the fields, submit the form), then press stop. Or say it:

```
record my browsing session
stop recording
```

Both are [instant commands](instant-commands.md). A recording only ever starts from your own click
or your own words; there is no action an agent or MCP client can call to start one.

Vexdom splits what you did into ordered steps, names them after what you accomplished, and keeps
them in a renameable list.

A recording **follows only the tab it started in**. Navigations inside that tab become steps, other
tabs are ignored, and closing the tab stops and saves it. It runs for at most **15 minutes**, warns
you at 13, and stops itself at the limit. Only http(s) pages can be recorded, and never a
[blocked site](blocked-sites.md).

---

## What you type is not saved by default

Every field becomes a placeholder, such as `{{email}}` or `{{invoice_number}}`, and you are asked
for the value when the recording is replayed.

Tick **Save what I type** to keep the literal values instead. Either way, **passwords, hidden
fields, one-time codes and anything shaped like a card number are always dropped.**

---

## Replaying

```
do it like last time
```

Replay is not blind playback. The agent (the `browse-navigation` skill) treats the steps as a plan:

- it finds the recording with `listRecordings` and opens it with `readRecording`;
- it re-checks each target against the live page before acting;
- it prefers the **visible text** it recorded over the CSS selector, because selectors are what a
  redesign breaks first;
- anything consequential still waits for [approval](../approvals.md), even though you did it
  yourself while recording;
- if a step no longer lands, the run **stops and tells you which one** instead of improvising a
  different route to the same result.

If two recordings could match what you asked for, you are asked which one. The agent never guesses
between them, because replaying the wrong workflow spends real clicks on your real account.

**Schedule** on a recording turns it into a [scheduled task](scheduling.md) that replays the steps
without an agent, and hands over to the agent only when a step no longer fits the page.

---

## Where they live

In the extension's own storage, not on disk. That is why `vexdom skills` does not list them, and why
they are read through the `listRecordings` and `readRecording` actions (an MCP client calls
`act {"do":"listRecordings"}`). A recording still being turned into steps answers
`RECORDING_NOT_READY`. Removing the extension removes them.

A recording leaves the browser once: the scrubbed trace goes to the Bridge, which runs a one-shot
Claude Code session to turn it into named steps. That session sends the trace to Claude Code's model
like any other run.

---

## See also

- [Site maps](site-maps.md): the other kind of memory
- [Flows](flows.md): replayable sequences of actions, run by the Bridge
- [Skills](skills.md): `browse-navigation` is the skill that replays these
- [internals/subsystems.md](../../internals/subsystems.md): capture, scrubbing and step synthesis
