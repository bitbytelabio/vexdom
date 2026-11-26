# The side-panel agent

The side panel runs each instruction through [Claude Code](https://claude.com/claude-code), the
agent CLI you are signed in to. It is the only agent the side panel supports. This page covers
installing it, picking a model, how a run is contained, and common problems.

It applies to the side panel only. Driving the browser from Claude Code in a terminal, or any other
MCP client, goes through `vexdom mcp`; see [MCP clients](mcp-clients.md).

---

## Install and sign in

```sh
npm i -g @anthropic-ai/claude-code
claude            # once, to sign in
vexdom agent      # checks it
```

```
● Claude Code  ready — 2.1.332 (Claude Code)
    models: Vexdom's built-in list of 4.
```

`vexdom agent fix claude` lets Vexdom fix what the agent still needs, and `vexdom doctor` reports the
same state under `agent.claude`. The side panel's status pill shows it too, so a missing CLI shows
up before you send an instruction.

## Picking a model

The status pill has a model select. *Default* runs whatever Claude Code would run on its own; the
shipped list is its aliases `fable`, `opus`, `sonnet` and `haiku`, which it resolves to the newest
model of each family. A picked model applies from the next instruction and keeps the open
conversation.

```sh
vexdom agent model claude opus     # pin a model
vexdom agent model claude          # back to the default
vexdom agent models claude         # the models the select offers, and where they came from
vexdom agent models claude --refresh
```

Unset, the model is `sonnet`.

---

## How a run works

For each instruction the Bridge starts `claude -p` with:

- the system prompt in a file of its own (`--append-system-prompt-file`), your message on stdin;
- one MCP server, `vexdom`, which is this same binary's `vexdom mcp` with `VEXDOM_AGENT_RUN` set to
  the run's id, so every call is tied to the run and its [approval gate](approvals.md). It offers the
  same three tools as for any MCP client (`snapshot`, `act`, `guide`); actions withheld from runs
  (`authSave`, and page code until you switch **Live tool** on) are not offered;
- `--strict-mcp-config`, `--tools` empty, `--allowedTools mcp__vexdom`, and an explicit
  `--disallowedTools` list of Claude Code's own tools (shell, file reads and edits, sub-agents, web
  tools and the rest), so the model can reach the browser and nothing else on your machine. A
  site-mapping run that is allowed to research also gets `WebSearch` and `WebFetch`;
- `--resume` for a follow-up, so the conversation continues; `--model` and `--effort` from your
  settings.

Claude Code keeps the system prompt a conversation began with, so when what the agent should know
changes (an element picked with A-Eye, a skill attached, notes for a site you moved to), Vexdom puts
the difference in the message itself. A message that changes nothing carries nothing extra.

The environment is sealed: cloud keys, registry tokens and database URLs inherited from your shell
are removed before the spawn. [internals/agent-runs.md](../internals/agent-runs.md) has the detail.

---

## Settings

In `~/.vexdom/config.json`:

```json
{
  "agent": "claude",
  "agents": {
    "claude": { "bin": "/opt/homebrew/bin/claude", "model": "opus", "effort": "high" }
  }
}
```

| Key | Default | Effect |
| --- | --- | --- |
| `agents.claude.bin` | `claude` | Absolute path to the binary. Set it when the Bridge's `PATH` differs from your shell's |
| `agents.claude.model` | `sonnet` | Passed as `--model` |
| `agents.claude.effort` | unset | Passed as `--effort`: `low`, `medium`, `high`, `xhigh` or `max` |

Changes apply to the next run; the Bridge needs no restart.

---

## Common problems

| Symptom | Fix |
| --- | --- |
| `AGENT_MISSING` | The Bridge cannot find `claude` on its `PATH`. Set `agents.claude.bin` to an absolute path (`command -v claude`) |
| `AGENT_FAILED: Not logged in · Please run /login` | Run `claude` and sign in, then send the message again |
| `AGENT_UNUSABLE` | The CLI did not run or is too old for the flags Vexdom passes. Update it |
| *could not start Vexdom's browser tools* | Often a Bridge still running from files an update replaced. `vexdom restart`, then send the message again |
| The model select says *built-in list*, with a reason | Vexdom could not read the CLI's own list. The shipped list works meanwhile |
| A model shows *not listed* | You pinned it and the CLI no longer lists it. It is still passed on |

---

## See also

- [Configuration](configuration.md): the rest of `config.json`
- [MCP clients](mcp-clients.md): the route in from a terminal
- [internals/agent-runs.md](../internals/agent-runs.md): how a run is spawned and streamed
