# Vexdom documentation

Vexdom lets an agent work in your real, logged-in browser. One Rust binary, `vexdom`, is Vexdom
Bridge (the background process the browser extension connects to), the command line and an MCP
server; the extension acts on your tabs and hosts a side panel that runs Claude Code. You drive the
browser from the side panel, from the `vexdom` CLI (`vexdom snapshot`, `vexdom click @e5`…), or from
any MCP client through three tools: `snapshot`, `act` and `guide`.

The docs are in three parts: the user guide, the internals and the reference.

## Using it

**[The user guide →](guide/)**

| | |
| --- | --- |
| [Install](guide/install.md) | `install.sh`, then `vexdom setup` adds the extension to your browser |
| [Pair](guide/pair.md) | How a browser connects to the Bridge, by itself or with a code |
| [First run](guide/first-run.md) | The side panel, a first instruction, and the same from a terminal |
| [Features](guide/features/) | One page per capability |
| [MCP clients](guide/mcp-clients.md) | `claude mcp add vexdom -- vexdom mcp`, the three tools and the playbooks |
| [The side-panel agent](guide/agents.md) | Claude Code: models, containment, problems |
| [The Mac app](guide/mac-app.md) | Vexdom.app |
| [Configuration](guide/configuration.md) | `~/.vexdom/config.json`, `./vexdom.json`, environment variables |
| [Approvals](guide/approvals.md) | What waits for your approval, and how you answer it |
| [Limits](guide/limits.md) | Where Vexdom does not fit; read it before you rely on it |
| [Troubleshooting](guide/troubleshooting.md) | Symptom → cause → fix |
| [Maintenance](guide/maintenance.md) | `vexdom doctor`, `vexdom update`, uninstalling |

## Building on it

How the parts work, for contributors and integrators.

**[Internals →](internals/)**

| | |
| --- | --- |
| [Overview](internals/overview.md) | The processes, and why a browser extension needs a daemon |
| [Transport](internals/transport.md) | Ports, the origin gate, pairing, Cap'n Proto over WebSocket |
| [The action registry](internals/registry.md) | One definition per action, the manifests, Bridge actions |
| [Request path](internals/request-path.md) | An `act` call or a CLI command, end to end |
| [Inside the extension](internals/extension.md) | Background and content scripts, frames, the debugger hub |
| [Agent runs](internals/agent-runs.md) | Side-panel runs: intent, Claude Code, prompt assembly |
| [Guardrails](internals/guardrails.md) | The policy, confirmations, sealing, fencing |
| [Subsystems](internals/subsystems.md) | Monitors, recordings, site maps, files, artifacts, video, WebMCP |
| [State on disk](internals/state.md) | What is written where, and at what mode |
| [Contributing](internals/contributing.md) | Building, the gates, adding an action or a skill |
| [The macOS app](internals/mac-app.md) | Vexdom.app |

## Looking something up

**[Reference →](reference/)**

| | |
| --- | --- |
| [Tools](reference/tools.md) | The three MCP tools, and every action `act` runs |
| [CLI](reference/cli.md) | Every `vexdom` command and flag |
| [Errors](reference/errors.md) | Error codes, what caused them, what to do |

---

To get started: [Install](guide/install.md) → [Pair](guide/pair.md) → [First run](guide/first-run.md).

The [project README](../README.md) is the short version.
