# Internals

How Vexdom is built: the processes it runs, how they connect, and the path a request takes from
an MCP client, the CLI or the side panel to the page.

Vexdom is one Rust binary, `vexdom`, plus a browser extension. The binary is Vexdom Bridge (the
daemon, `vexdom daemon`), the CLI, the MCP server (`vexdom mcp`) and the native-messaging host
(`vexdom native-host`). The extension (`packages/extension/`) talks to the Bridge in Cap'n Proto
over a loopback WebSocket. Side-panel runs are carried out by Claude Code, which the Bridge
starts. `Vexdom.app` (`desktop/`) is an optional macOS front end that drives the Bridge over the
same control socket the CLI uses.

Read in order, these pages follow a request through all of it:

| | |
| --- | --- |
| **1.** [Overview](overview.md) | The processes, the crates that build them, and why a browser extension needs a daemon |
| **2.** [Transport](transport.md) | Ports, `/health`, the `Origin` gate, the pairing handshake, Cap'n Proto over WebSocket, the protocol version, the control token |
| **3.** [The action registry](registry.md) | One TypeScript registry, the manifest the Bridge embeds, action names, `chromiumOnly`, reserved actions, drift |
| **4.** [Request path](request-path.md) | An MCP `act` call and a CLI verb, end to end: guardrails, confirmations, refs, deltas, sealing, fencing |
| **5.** [Inside the extension](extension.md) | Background vs content script, frames, the debugger hub, the panel, tab scoping |
| **6.** [Agent runs](agent-runs.md) | The side panel's path: the intent funnel, Claude Code, prompt assembly, skills |
| **7.** [Guardrails](guardrails.md) | The policy, run scope, approvals, result fencing |
| **8.** [Subsystems](subsystems.md) | Monitors, recordings, site maps, files, screenshots |
| **9.** [State on disk](state.md) | Every file Vexdom writes, and why it is where it is |
| **10.** [Contributing](contributing.md) | Build topology, the checks, and adding a capability |
| **11.** [The macOS app](mac-app.md) | What `Vexdom.app` does natively and what stays in the Bridge |

Error codes are listed in [reference/errors.md](../reference/errors.md) and CLI commands in
[reference/cli.md](../reference/cli.md). For one action's exact input, ask the Bridge:
`guide {"topic":"<action>"}` over MCP, or `vexdom tools` for the whole manifest.

The design decisions behind this layout are in [decisions.md](decisions.md) (ADR-022 to
ADR-032).
