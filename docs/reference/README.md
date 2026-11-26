# Reference

Lookup tables. For tasks, see the [user guide](../guide/README.md); for how Vexdom works, see
[internals](../internals/overview.md).

| Page | Holds |
| --- | --- |
| [Tools](tools.md) | The three MCP tools (`snapshot`, `act`, `guide`) with their input schemas, the `vexdom://page/*` and `skill://` resources, the output limit, and every page and Bridge action |
| [CLI](cli.md) | Every `vexdom` command and flag, the page-command flags, `--json`, `./vexdom.json`, and exit codes |
| [Errors](errors.md) | Every error code, where it comes from, and what to do next |

The machine-readable action list ships in the binary:

```sh
vexdom tools                 # page actions with their input schemas, as JSON
vexdom mcp call guide        # what the connected browser offers, by group
```
