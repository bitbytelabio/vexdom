# First run

A tour of the side panel, three instructions to try, the same thing from a terminal, and what to
check if nothing answers. It assumes you have [installed](install.md) and [paired](pair.md).

---

## Open the side panel

Click the Vexdom toolbar icon, or open it from the browser's side panel menu. Clicking the icon
again closes it. In Firefox the icon opens and closes the Vexdom sidebar.

## The panel, top to bottom

| | |
| --- | --- |
| **Status pill** | Connection state. Click it for the pairing form and the agent's state and model. |
| **Sessions strip** | One row per tab that has a conversation: its title, a pulsing dot while it works, how many messages it holds. See [Conversations](features/conversations.md). |
| **Chat · History · Skills · Recordings · Schedules** | The five sections. Chat is where you talk; the others are covered in [Features](features/). |
| **Timeline** | Every action as it happens, with what it targeted and what came back. |
| **Composer** | Type, or press the mic to dictate. |

## Your first instruction

Open an ordinary website (not a `chrome://` page or the new-tab page, which cannot host a content
script) and type:

```
what's on this page?
```

The Bridge starts Claude Code with Vexdom's three MCP tools; the agent takes a snapshot of the tab
and answers. That round trip shows every piece working: panel → Vexdom Bridge → Claude Code →
browser.

Then something that acts:

```
scroll to the bottom
```

That one carries a ⚡ and returns at once: it matched the local command grammar and ran in the
browser with no agent involved. See [Instant commands](features/instant-commands.md).

Then something with several steps:

```
find the search box, type "wireless headphones" and show me what comes back
```

Follow-ups continue the same conversation, so **"now click the second one"** works.

## The same page from a terminal

The CLI drives the same browser without an agent. A snapshot lists the tab's controls, each with a
ref; element commands take a ref (`@e3`) or a CSS selector and print the result, then what changed:

```sh
vexdom snapshot
vexdom fill @e3 "wireless headphones"
vexdom press Enter
vexdom read --outline
```

```
snapshot r1
- textbox "Search" [ref=e3]
- button "Go" [ref=e4]
…
```

After a page command, `snapshot r2 (delta from r1): …` shows only the lines that changed, and
`snapshot r2: unchanged` when nothing did. A ref belongs to the latest snapshot of its tab: an unknown one is refused with
`UNKNOWN_REF`, one from a page that has since reloaded with `STALE_REF`. Take a new snapshot then.
Every verb is in `vexdom help`.

## The first time something is consequential

Submitting a form pauses the run and asks you, with **Allow**, **Deny** and **Always on ‹host›**.
The last grants that one action on that one site and stops asking. Denying is final: the agent is
told to report it and stop, not to find another route to the same effect.

From the CLI or an MCP client the same action waits for your answer too: in the Vexdom app, with
`vexdom approve c-…`, or in the MCP client's own prompt. [Approvals](approvals.md) covers both.

## If it does not answer

Check, in this order:

1. `vexdom status`: is your browser `connected`?
2. `vexdom agent`: is Claude Code installed and ready?
3. `vexdom doctor`: every check at once, each with its fix.
4. `vexdom logs`: run starts, routed skills, every action.

[Troubleshooting](troubleshooting.md) maps symptoms to fixes.

---

## Where to go next

- **[Features](features/)**: one page per capability
- **[MCP clients](mcp-clients.md)**: drive the same browser from Claude Code in a terminal
- **[Configuration](configuration.md)**: `~/.vexdom/config.json` and `./vexdom.json`
- **[Limits](limits.md)**: where Vexdom stops, worth reading early
