# Skills

Skills are markdown playbooks that tell an agent how to handle a kind of job. Vexdom ships 31 of
them inside the binary. The side panel routes each instruction to one; MCP clients get them as
prompts, `skill://` resources and `guide` topics. You can add, override or pin your own.

```sh
vexdom skills          # every skill the router can see, and where it came from
vexdom skills --json   # the same, plus the folders and Claude Code's own skills
```

---

## The bundled skills

They live in `crates/vexdom-core/assets/skills/<name>/SKILL.md`.

| Kind | Skills |
| --- | --- |
| Core playbook for MCP clients | `vexdom` (snapshot, act on refs, read the delta; reference files `references/batch.md`, `references/read.md`, `references/snapshot-refs.md`) |
| Action groups | `navigation`, `input`, `page-storage`, `code`, `downloads`, `drag`, `network`, `emulation`, `dialogs`, `capture`, `devtools`, `webmcp` |
| Tasks | `flows`, `auth`, `page-state`, `captcha`, `page-theming`, `page-diagnostics`, `monitor-progress`, `scheduled-jobs`, `browser-control`, `browse-navigation`, `page-research`, `page-scripting`, `dogfood`, `derive-client`, `webmcp-gen`, `slack` |
| Side panel only | `a-eye`, `site-mapper` |

`a-eye` and `site-mapper` drive side-panel run modes and are not served over MCP. The other 29 are.

---

## Skills over MCP

`vexdom mcp` serves the 29 playbooks four ways:

| | |
| --- | --- |
| **Prompts** | One per playbook, named after it (`network`); the body without its front matter |
| **Resources** | `skill://<name>/SKILL.md`, and each reference file as `skill://<name>/<path>` |
| **Skills extension** | The same files, for clients that support it |
| **`guide`** | `guide` with no topic lists every playbook with its description; `guide {"topic":"<name>"}` returns one, `guide {"topic":"vexdom/references/batch.md"}` a reference file |

Where a group and a playbook share a name (`network`), `guide {"topic":"network"}` returns the
group's guide: its playbook and each action's input.

The side-panel agent talks to the same three-tool server, so it opens a playbook mid-run the same
way, with `guide`.

---

## Routing in the side panel

Every side-panel instruction is routed to one **base** skill. A base is any skill with category
`general` (the default) except `page-scripting`. The base is picked by counting the trigger words of
each base found in what you typed; `browser-control` is the default when none match. These bundled
bases have triggers:

| Skill | Handles |
| --- | --- |
| `browser-control` *(default)* | Drive the open tab: click, type, submit, navigate, verify |
| `page-research` | Read and summarise without changing anything |
| `page-theming` | [Retheme a page](theming.md) |
| `page-diagnostics` | Find out [why a page misbehaved](diagnostics.md) |
| `browse-navigation` | [Replay a recorded session](recordings.md) |
| `monitor-progress` | [Watch a long-running task](monitoring.md) |
| `scheduled-jobs` | [Do something on a clock](scheduling.md) |
| `site-mapper` | [Walk a site](site-maps.md) and write up its layout |
| `captcha` | [Get past a "verify you are human" block](captcha.md) |
| `a-eye` | Work on [the element you pointed at](a-eye.md) |

Prefix an instruction with `@name` to pin any skill:

```
@page-research what does this page say about refunds?
```

A pinned `general` skill becomes the base; a pinned site skill rides on top of the routed base.

`page-scripting` is never the base: it rides along only while the composer's **Live tool** switch is
on (see [Page actions](page-actions.md#live-page-code)).

Typing `/` at the start of the composer opens a picker over every skill, with skills for the current
site first.

### Site notes are overlays

A skill with `category: site-exploration` and a `domains:` list stacks **on top of** the base
whenever the active tab's host matches, longest match first. [Site maps](site-maps.md) generate
these; you can write one by hand.

---

## Writing your own

A skill is markdown with front matter:

```markdown
---
name: acme-admin
description: Our internal admin tool.
category: site-exploration
domains: [admin.acme.com]
---

Search is `#q` and submits on Enter, not on the button.
Results lazy load. Click "Load more" until it disappears before counting anything.
```

A `general` skill takes `triggers: [phrase, …]` to be routed to. Names are lowercase letters, digits
and hyphens, at most 48 characters.

Both `<name>.md` and `<name>/SKILL.md` are recognised (the file wins if both exist). Three layers,
later ones shadowing earlier ones by name:

| Layer | Tagged | Where |
| --- | --- | --- |
| bundled | `bundled` | Inside the binary |
| yours | `user` | `~/.vexdom/skills/` (`$VEXDOM_HOME/skills`) |
| uploaded | `uploaded` | `~/vexdom/skills/` (or `skillsDir`): panel uploads and generated site maps |

All three are re-read on every run, so an edit applies to the next instruction.

Your copy of a bundled skill is used by side-panel routing and by a group's guide
(`guide {"topic":"captcha"}`). MCP prompts, `skill://` resources and playbook topics always serve the
bundled text.

---

## Claude Code's own skills

The `/` picker also lists the skills Claude Code keeps in `~/.claude/skills/` (files up to 48 KB, at
most 100). Picking one attaches it to that message: the Bridge reads the file when the run starts and
appends it to the system prompt, marked as such. Only the title reaches the browser; the panel holds
an opaque id the Bridge issued, so the file's path and contents stay on your computer.

---

## What does not go through routing

[Instant commands](instant-commands.md) never reach an agent. Anything starting with `@` always goes
to the agent, which is why a pin always works.

---

## See also

- [internals/agent-runs.md](../../internals/agent-runs.md): how the prompt is assembled
- [MCP clients](../mcp-clients.md): connecting a client to `vexdom mcp`
- [Site maps](site-maps.md): generated overlays
