# Instant commands

Vexdom checks every instruction you type or say in the side panel against a local grammar before it
reaches the agent. A confident single-step command such as "go back" runs in the browser in
**milliseconds** instead of taking several seconds through Claude Code; everything else goes to the
agent with your text unchanged.

Commands handled locally carry a **⚡ local** tag on the timeline.

---

## What runs locally

| Runs locally | Goes to the agent |
| --- | --- |
| back, forward, reload | "is there a login button?" |
| open github.com, open localhost:3000, go to gmail | "open the settings menu" |
| open github.com in a new tab | "close this tab", "switch to my gmail tab" |
| google something, search the web for something | "search for wireless headphones" |
| search this site for something | "find the cheapest one" |
| scroll up, down, top, bottom, page down, scroll to pricing | "scroll down and tell me what it says" |
| press enter, hit escape, press arrow down | "click sign in and then fill in my email" |
| click Sign in, tap Continue | "click Buy now", "click it" |
| record my browsing session, stop recording | "record a video of this page" |
| stop monitoring | "stop watching and tell me what happened" |

Each local command is one page action: `navigate`, `openTab`, `searchSite`, `scrollTo`, `pressKey`,
`clickElement`, `stopMonitor`, or starting and stopping a [recording](recordings.md).

Common site names are known, so "open gmail", "open hacker news" and "open stack overflow" resolve
without a URL.

---

## What always goes to the agent

The grammar leans toward escalating, because the two possible mistakes do not cost the same:

- Escalating something it could have handled costs a round trip.
- Acting on something it misread spends a wrong click on your real page.

So these always escalate, however confident the match looks:

| | |
| --- | --- |
| Questions | "is there a login button?", and anything starting *what*, *how*, *find*, *show*, *check*, *fill* and similar |
| Multi-step phrasing | "and then", "then", "after that", "followed by" |
| Hedges | "if", "unless", "maybe", "try to", "as soon as" |
| Anything starting with `@` | An explicit [skill pin](skills.md) |
| Consequential-sounding targets | *buy*, *pay*, *checkout*, *delete*, *remove*, *send*, *submit*, *transfer*, *subscribe*, *confirm* and similar |

A match that scores below the threshold also escalates, and so does a local command that runs and
**fails**, instead of reporting the failure.

---

## Logs and routing checks

**Local commands do not appear in `vexdom logs`.** They never reach Vexdom Bridge, so the ⚡ tag
on the timeline is their only record. This is expected.

The grammar lives in `crates/vexdom-pure/src/intent.rs` and runs in the extension through its wasm
build. To see how one instruction would be routed, from a checkout of the repository:

```sh
cd packages/extension
bun run ext-core:build        # once, to build the wasm the check loads
bun run check:intent "take me to the checkout page"
```

It prints the routing (`act` with the action and input, or `escalate` with the reason) and the score
it needs to act.

---

## See also

- [Conversations](conversations.md): the timeline these appear on
- [Skills](skills.md): what happens to everything that escalates
- [internals/agent-runs.md](../../internals/agent-runs.md): the intent funnel and scoring
