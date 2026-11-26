# User guide

How to install, pair, use, configure and troubleshoot Vexdom on your own computer and browser.

## Setting up

No account, no API key, no cloud service.

1. **[Install](install.md)**: one script installs Vexdom Bridge (and, on macOS, the [Mac app](mac-app.md)); `vexdom setup` adds the extension
2. **[Pair](pair.md)**: the extension connects by itself, or with a single-use code
3. **[First run](first-run.md)**: a tour of the side panel, your first instruction, and the same from a terminal

## Using it

| | |
| --- | --- |
| [Features](features/) | One page per capability, side panel, CLI and MCP |
| [MCP clients](mcp-clients.md) | `claude mcp add vexdom -- vexdom mcp`: three tools (`snapshot`, `act`, `guide`) and the playbooks |
| [The side-panel agent](agents.md) | Claude Code: installing it, models, how a run is contained |
| [The Mac app](mac-app.md) | The switch, pairing, approvals and settings in a window |

## Configuring

| | |
| --- | --- |
| [Configuration](configuration.md) | `~/.vexdom/config.json`, the project file `./vexdom.json`, the environment |
| [Approvals](approvals.md) | Which actions wait for your approval, and how you answer from the panel, the app, a terminal or an MCP client |

## When something is wrong

| | |
| --- | --- |
| [Limits](limits.md) | Where Vexdom stops, including the Chromium-only debugger features |
| [Troubleshooting](troubleshooting.md) | Symptom, cause, fix |
| [Maintenance](maintenance.md) | `vexdom doctor`, signed updates, uninstalling |

---

How it works underneath: [Internals](../internals/).
