# CLI

Every command and flag of `vexdom`: the Bridge, its setup and maintenance, the page commands, flows,
the auth vault, plugins and the MCP server. Copied from `usage()` and the parsers in
[`app/src/cli/`](../../app/src/cli/).

```
vexdom <command> [args] [flags]
```

With no command, `vexdom` prints usage; when `VEXDOM_AGENT_RUN` is set it serves MCP instead.
`vexdom help`, `--help` and `-h` print usage; `--version` and `-v` print the version.

Commands that talk to the Bridge start it when it is not running. `status`, `browsers`, `doctor`, `tools`,
`skills`, `approvals`, `tasks`, `downloads`, `logs`, `token`, `plugin`, `state list|show|rename|clear|clean`
and `uninstall` do not.

---

## Exit codes

| Code | When |
| --- | --- |
| `0` | The command did what it was asked |
| `1` | Anything else: a usage error, a refused or failed action, no Bridge. The reason goes to stderr (`CODE: message` for an action), or to stdout as the `--json` envelope |

Particular cases: `doctor` exits `1` when any check fails; `batch` exits `1` when any step failed;
`stop` exits `1` when a Bridge would not exit; `uninstall` exits `1` without removing anything when
stdin is not a terminal and `--yes` was not given.

---

## Install and maintenance

| Command | Does |
| --- | --- |
| `vexdom setup` | Start the Bridge, register the native host with each browser found, then add the extension to a browser and pair the two |
| `vexdom update` | Install the latest signed release over this copy (the tarball or `Vexdom.app`) and restart the Bridge |
| `vexdom doctor` | Check this install, the Bridge, the native host, browsers, the agent, config, permissions, ffmpeg and the network |
| `vexdom browsers` | The browsers on this computer, where each gets the extension, and which are connected. `--json` |
| `vexdom uninstall` | Stop the Bridge and remove everything Vexdom wrote |

### `setup`

| Flag | Does |
| --- | --- |
| `--browser <id>` | `chrome`, `edge`, `brave`, `arc`, `vivaldi`, `opera`, `chromium` or `firefox`: skip the question |
| `--unpacked` | Write the extension folder to load unpacked (`~/vexdom/extension/chrome-mv3`), in any browser |
| `--dir <path>` | Write the unpacked folder there instead. Remembered, so `update` refreshes the same copy |
| `--force` | Rewrite every file of the unpacked folder |
| `--no-open` | Do not open the Firefox add-on link in the browser; print it |
| `--no-wait` | Print the pairing code and return, instead of waiting for the browser to connect |
| `--no-pair` | Mint no pairing code |
| `--restart` | Stop the running Bridge first, so the new binary's Bridge starts |
| `--json` | The result as JSON (every browser's row, the code, the folder). Never asks, never waits |

Chromium browsers load the extension unpacked from the folder `setup` writes; Firefox installs the
signed add-on from the release. Without a terminal and without `--browser`, `setup` asks nothing: it
prints each browser's steps and a code. See [Install](../guide/install.md).

### `update`

| Flag | Does |
| --- | --- |
| `--check` | Only say whether a newer release is out |
| `--no-restart` | Install, but do not restart the Bridge |
| `--local` | Re-run setup with this binary (`setup --no-pair --restart`); downloads nothing |
| `--json` | `{from, to, updated: false}`, `{from, to, available: true}` with `--check`, or `{from, to, layout, restarted}` |

A copy not installed from a release fails with `UPDATE_UNMANAGED`. The download is checked against
the signed `SHA256SUMS` (`SIGNATURE_INVALID`, `CHECKSUM_MISMATCH`). See
[Maintenance](../guide/maintenance.md#updating).

### `doctor`

| Flag | Does |
| --- | --- |
| `--fix` | Repair what it safely can, then report |
| `--offline` | Skip the network checks |
| `--json` | Checks, summary and repairs as JSON |

See [Maintenance](../guide/maintenance.md#doctor).

### `uninstall`

| Flag | Does |
| --- | --- |
| `--dry-run` | Print the plan and stop |
| `--yes` / `-y` | Skip the question. Required when stdin is not a terminal |
| `--keep-skills` | Leave `skills/` behind |

See [Maintenance](../guide/maintenance.md#uninstalling).

---

## Pairing and approvals

| Command | Does |
| --- | --- |
| `vexdom pair` | Issue a one-time pairing code to type into the extension |
| `vexdom pairing [auto\|ask\|code]` | Show or set how browsers pair: by themselves (`auto`), once you allow each (`ask`), or only with a code (`code`) |
| `vexdom requests` | Browsers waiting for you to allow them, and actions waiting for your approval (`c-…` ids). `--json` prints `{requests, confirmations}` |
| `vexdom approve [id]` | Allow a waiting browser (the only one, when there is one), or let a revoked or denied one back in. `--name <name>` names it |
| `vexdom approve c-… [--always]` | Allow a waiting action; `--always` allows it on that site from now on. Needs stdin to be a terminal, which stops an agent that simply runs it, though not a process that gives itself one (`script`); see [Approvals](../guide/approvals.md#from-the-cli-or-an-mcp-client) |
| `vexdom approve --watch` | Ask about each browser and action as it comes, until Ctrl-C. Needs a terminal. Answer `y`, `n`, or `a` (always on this site) |
| `vexdom deny <id>` | Turn a waiting browser down, or refuse a waiting action (`c-…`) |
| `vexdom sessions` | Paired browsers, how each paired, and the ones kept out |
| `vexdom rename <id> <name>` | Name a paired browser. An empty name clears it |
| `vexdom revoke [id]` | Unpair one browser by the id `sessions` prints (or by origin), or all of them |

A page command waiting on an approval prints `Waiting for approval c-…: approve it in Vexdom, or
run vexdom approve c-… (vexdom deny c-… to refuse).` on stderr. The wait is 120 seconds; then the
action fails with `APPROVAL_TIMEOUT`, or `DECLINED` when refused. See
[Approvals](../guide/approvals.md#from-the-cli-or-an-mcp-client) and [Pair](../guide/pair.md).

---

## Page commands

Each page command becomes one or a few [actions](tools.md#actions). It prints the result as one
line of JSON, then the snapshot delta the action caused, if any.

### Shared flags

| Flag | Does |
| --- | --- |
| `--tab <id>` | Act on that tab instead of the active one |
| `--max-output <n>` | Cut printed text to `n` characters. No limit by default |
| `--json` | Print `{"ok":true,"data":…}` or `{"ok":false,"error":{"code","message"}}` on stdout, and exit `1` on an error |

`./vexdom.json` in the working directory (never a parent), over `"cli"` in
`~/.vexdom/config.json`, sets defaults for `json`, `maxOutput`, `annotate`, `screenshotFormat`,
`screenshotQuality` and `timeoutMs`; other keys in the project file are ignored with a warning. See
[Configuration](../guide/configuration.md#cli-preferences).

### Targets

`<t>` is a ref from the last snapshot (`@e5`, `e5` or `ref=e5`) or a CSS selector. `find` names an
element by locator instead.

### Looking

| Command | Flags |
| --- | --- |
| `vexdom snapshot` | `--all` every element (default: interactive only, `-i`), `-c` compact, `-d <n>` depth, `-s <sel>` subtree, `-u` link targets, `--full` the whole tree rather than the delta, `--json` `{text, refs}` |
| `vexdom read` | The tab as markdown. `--outline` headings only, `--filter <text>` matching sections, `--target <t>` one element, `--offset <n>` from char `n` (a cut answer names the next one) |
| `vexdom read <url>` | A page over HTTP, no tab. `--offset <n>`, `--raw`, `--require-md`, `--llms index\|full`, `--timeout <ms>`, `--outline`, `--filter <text>`. Unlike the MCP `snapshot` tool, it may read loopback and local-network addresses; each URL and redirect hop is still checked like a navigation (blocked sites, URL payload) |
| `vexdom screenshot` | `-o <file>`, `--screenshot-dir <dir>`, `--screenshot-format png\|jpeg`, `--screenshot-quality <0-100>`, `--full`, `-s <t>`, `--if-changed [--threshold <0-1>]`, `--annotate`. Without `-o` or a directory it writes to the temp folder; prints the result with `savedTo` |
| `vexdom get text\|html\|value\|attr\|count\|box\|styles <t> [name]` | One fact about an element; `attr` needs the attribute name |
| `vexdom get title\|url` | The page's title or address |
| `vexdom is visible\|enabled\|checked <t>` | `true` or `false` |

### Acting on elements

| Command | Flags |
| --- | --- |
| `vexdom click <t>` | `--button left\|right\|middle`, `--modifiers ctrl,shift` (also `ctrl+shift`; `ctrl`, `shift`, `alt`, `meta`), `--new-tab` (opens the link's `href` in a new tab; no `--button`/`--modifiers`), `--force` (skip actionability checks) |
| `vexdom dblclick <t>` | `--modifiers …` |
| `vexdom fill <t> <text>` | Replace the field's value |
| `vexdom type <t> <text>` | `--clear` empty it first, `--delay <ms>` between keys, `--trusted` |
| `vexdom select <t> <value…>` | `--by value\|label` (default `value`); several values for a `<select multiple>` |
| `vexdom check\|uncheck <t>` | `--force` |
| `vexdom hover <t>` | `--force` |
| `vexdom focus\|highlight\|scrollintoview <t>` | |
| `vexdom submit [<t>]` | |
| `vexdom drag <from> <to>` | |
| `vexdom upload <t> <file…>` | Files must exist (`FILE_NOT_FOUND`) |
| `vexdom download <t> [path]` | Click it and capture the download; with a path, copy the file there |
| `vexdom find role\|text\|label\|placeholder\|alt\|title\|testid <value> <command> [arg]` | `--name <n>`, `--exact`. `<command>` is an element command, `get`, `is` or `press` |
| `vexdom find first\|last <sel> <command> [arg]` | |
| `vexdom find nth <n> <sel> <command> [arg]` | |

### Keyboard and mouse

| Command | Flags |
| --- | --- |
| `vexdom press\|key [<t>] <key>` | `Enter`, `Control+a`, `Shift++`. `--trusted` |
| `vexdom keydown\|keyup <key>` | `--trusted` |
| `vexdom keyboard type <text>` | Into the focused element. `--trusted` |
| `vexdom keyboard inserttext <text>` | Without key events |
| `vexdom scroll up\|down\|left\|right\|top\|bottom [px]` | `--selector <sel>` scrolls that container |
| `vexdom mouse move <x> <y>` | `--human`, `--seed <n>`, `--steps <n>`, `--duration <ms>` |
| `vexdom mouse down\|up [left\|middle\|right]` | |
| `vexdom mouse wheel <dy> [dx]` | |

### Waiting and moving

| Command | Flags |
| --- | --- |
| `vexdom wait <ms>` | Sleep, at most 60,000 ms |
| `vexdom wait <t>` | `--state attached\|visible\|hidden\|detached`, `--timeout <ms>` |
| `vexdom wait --text <s> \| --url <glob> \| --load <state> \| --fn <js>` | Any combination; `--timeout <ms>` |
| `vexdom wait --download [path]` | Capture the next download; `--timeout <ms>` |
| `vexdom open\|goto\|navigate <url>` | `--init-script <file>` adds an init script first |
| `vexdom back\|forward\|reload` | |
| `vexdom pushstate <url>` | Change a single-page app's route without a reload |
| `vexdom tab [<id>\|<text>]` | No argument: list the open tabs. Otherwise switch by id (`12` or `t12`) or by text matching a title or address |
| `vexdom tab new <url>` | `--label <name>` |
| `vexdom tab close [<id>\|<text>]` | |
| `vexdom tab keep live\|throttled\|frozen [tab]` | |
| `vexdom window new [url]` | |
| `vexdom frame <t>\|main` | Enter an iframe, or go back to the top document (`main` or `top`) |

### Page state and code

| Command | Flags |
| --- | --- |
| `vexdom eval <js>` | Also `eval -b <base64>` (`--base64`) and `eval --stdin` |
| `vexdom storage local\|session [key]` | `… set <k> <v>`, `… remove <k>`, `… clear` |
| `vexdom cookies [get [name]]` | `--url <u>` |
| `vexdom cookies set <name> <value>` | `--url`, `--domain`, `--path`, `--secure`, `--http-only`, `--same-site`, `--expires <epoch s>` |
| `vexdom cookies set --curl <file>` | Every cookie in a curl cookie file; `--domain`, `--url` |
| `vexdom cookies clear [name]` | `--url <u>` |
| `vexdom clipboard read\|copy\|paste` | |
| `vexdom clipboard write <text>` | |
| `vexdom initscript add <file\|->` | `-` reads stdin |
| `vexdom initscript remove <id>` \| `initscript list` | |
| `vexdom state save\|load\|show <name\|path>` | Cookies and web storage, kept in `~/.vexdom/states`. `--tab <id>` |
| `vexdom state list` \| `state rename <old> <new>` \| `state clear [name\|--all]` \| `state clean --older-than <days>` | No browser needed |
| `vexdom diff snapshot` | `--baseline <file>`, `-s <sel>`, `-c`, `-d <n>` |
| `vexdom diff screenshot --baseline <file>` | `-o <out>`, `--threshold <f>`, `-s <t>`, `--full` |
| `vexdom diff url <u1> <u2>` | `--screenshot`, `--full` |

### WebMCP

| Command | Flags |
| --- | --- |
| `vexdom webmcp list [tool]` | `--frame <id>`, `--json` (with schemas) |
| `vexdom webmcp invoke <tool>` | `--params <json\|@file>`, `--frame <id>`, `--detach` |
| `vexdom webmcp result <id>` | `--wait <ms>` |
| `vexdom webmcp cancel <id>` | |

### Debugger features (Chromium only)

| Command | Flags |
| --- | --- |
| `vexdom console` | `--level <l>`, `--contains <s>`, `--limit <n>`, `--clear`. Starts a capture on the tab when none runs |
| `vexdom errors` | Console errors only. `--clear` |
| `vexdom network requests` | `--filter <s>`, `--type <csv>`, `--method <m>`, `--status <s>`, `--clear` |
| `vexdom network request <id>` | One request with headers and bodies |
| `vexdom network route <pattern>` | `--abort`, or `--body <json\|@file>` with `--status <n>`, `--content-type <t>`, `--allow-origin <o>`; `--resource-type <csv>` |
| `vexdom network unroute [pattern]` | |
| `vexdom network har start` | `--content all\|text\|none` (default `text`) |
| `vexdom network har stop [path]` | |
| `vexdom dialog [status]` \| `dialog accept [text]` \| `dialog dismiss` | |
| `vexdom dialog auto on\|off` | `--alert`, `--beforeunload` (neither: both) |
| `vexdom set viewport <w> <h>` | `--scale <n>`, `--mobile` |
| `vexdom set device <name>` | |
| `vexdom set geo <lat> <lon>` | `--accuracy <m>` |
| `vexdom set offline [on\|off]` \| `set headers <json\|@file>` \| `set credentials <user> <pass>` | |
| `vexdom set media [screen\|print]` | `--color-scheme <s>`, `--reduced-motion <s>` |
| `vexdom set reset` | |
| `vexdom pdf [path]` | `--landscape`, `--no-background`, `--css-page-size` |
| `vexdom record start <path>` | `--fps <n>`, `--cursor`, `--contact-sheet`, `--contact-sheet-threshold <x>` |
| `vexdom record stop` \| `record restart <path> …` | |
| `vexdom trace\|profiler start` | `--categories <csv>` |
| `vexdom trace\|profiler stop [path]` | |
| `vexdom vitals` | `--reload` |
| `vexdom react install` \| `react tree` \| `react inspect <id>` \| `react renders start\|stop` \| `react suspense` | `--json`; `suspense --only-dynamic` |
| `vexdom a11y` | `--tags <csv>`, `--selector <sel>` |

On Firefox these answer `UNSUPPORTED`. Relative paths are resolved against the working directory.

### `batch`

```sh
vexdom batch [--bail] [--json] [--omit-command] ["<command>" …]
```

Steps come from the arguments, from stdin as a JSON array of string arrays, or from stdin one per
line: `click @e3` or `{"action": "page.clickElement", "input": {…}}`. Lines starting with `#` are
skipped. Without `--json` it prints `N. <action> ok: <result>` or `N. <action> failed: CODE:
message` per step; with `--json`, one array of `{command|index, success, result|error}`.
`--bail` stops at the first failure. The exit code is `1` when any step failed.

---

## Flows, logins and plugins

| Command | Does |
| --- | --- |
| `vexdom flow record start\|stop\|status` | Record a task in this tab. `--origin <host>` |
| `vexdom flow steps` | What the recording captured |
| `vexdom flow save <name>` | Keep its requests as a saved flow. `--from <n>`, `--to <n>`, `--param k=v` (repeatable), `--output <id>`, `--description <t>`, `--timeout <ms>` |
| `vexdom flow run <name>` | Rerun it as direct requests from the page. `--param k=v`…, `--origin <host>`, `--allow-write all\|id,id`, `--where f=v`…, `--fields a,b`, `--limit <n>`, `--timeout <ms>` |
| `vexdom flow list` | `--origin <host>` |
| `vexdom auth save <name> --url <u> --username <u> --password-stdin` | Save a login in `~/.vexdom/vault`. `--password <p>` instead of stdin; `--username-selector`, `--password-selector`, `--submit-selector` |
| `vexdom auth login <name>` | `--url <u>`, `--no-navigate`, `--credential-provider <plugin> --item <i>`, `--timeout <ms>` |
| `vexdom auth list` \| `auth show <name>` \| `auth delete <name>` | |
| `vexdom plugin [list]` | The plugins in `~/.vexdom/config.json` (or `VEXDOM_PLUGINS`), with their capabilities |
| `vexdom plugin add <npm package\|owner/repo\|./file>` | Also `install`. `--name <n>`, `--capability <c>` (repeatable, or comma-separated; `--capabilities`), `--no-manifest`. `--project` is refused: plugins live only in `~/.vexdom/config.json` |
| `vexdom plugin show <name>` | |
| `vexdom plugin run <name> <type>` | `--payload <json>`. Refuses the core entry points `credential.read`, `credential.resolve` and `plugin.manifest` |

`flow`, `auth` and `plugin` take `--json`. See [Flows](../guide/features/flows.md),
[Auth vault](../guide/features/auth-vault.md) and [Plugins](../guide/features/plugins.md).

---

## Agent, skills and stored data

| Command | Does |
| --- | --- |
| `vexdom agent` | Which agent runs the side panel, and whether it is ready. `--json` |
| `vexdom agent claude` | Switch to Claude Code, the only agent |
| `vexdom agent fix claude` | Let Vexdom fix what the agent still needs (`agent setup` is the older spelling) |
| `vexdom agent model claude [model]` | Pin the model; omit it for the CLI's default |
| `vexdom agent models claude` | The models it offers. `--refresh` asks its CLI again; `--json` |
| `vexdom skills` | The skills the agent can route to, and where they came from. `--json` |
| `vexdom approvals` | The "always on this site" approvals you granted. `--json` |
| `vexdom approvals clear [host]` | Forget them, all or one site's |
| `vexdom tasks` | Scheduled tasks, when each runs next, how it last went. `--json` |
| `vexdom tasks pause\|resume [id]` | One task, or all of them |
| `vexdom tasks delete <id>` | |
| `vexdom downloads` | Files captured from pages, and where they were saved. `--json` |
| `vexdom downloads clear` | Delete all of them |
| `vexdom tools` | The bundled page-action manifest as JSON: each action's short `name`, full `action`, description and input schema. No browser needed |

See [Agents](../guide/agents.md), [Skills](../guide/features/skills.md),
[Scheduling](../guide/features/scheduling.md), [Files](../guide/features/files.md).

---

## The Bridge

| Command | Does |
| --- | --- |
| `vexdom status` | The Bridge, which browsers can start it, the agent, each paired browser (version, source, connected, an unpacked copy that needs ↻), whether the tool lists match, and the pairing mode |
| `vexdom start` | Bring the Bridge up in the background, if it is not already |
| `vexdom stop` | Stop the Bridge |
| `vexdom restart` | Stop it and bring up a fresh one |
| `vexdom logs` | Print the Bridge's log |
| `vexdom token` | Print the control token, for MCP clients, not the browser |

A running Bridge keeps the build it started with until `restart`.

---

## MCP

| Command | Does |
| --- | --- |
| `vexdom mcp` | Serve MCP over stdio: what a client runs. `--max-output <n>` sets the output limit (`0`: none) |
| `vexdom mcp call <tool> [json]` | Call one tool the way a client would, e.g. `vexdom mcp call act '{"do":"navigate","url":"https://example.com"}'`. Prints text parts; an image as its type and size. Exits `1` on an error result |

```sh
claude mcp add vexdom -- vexdom mcp
```

See [Tools](tools.md) and [MCP clients](../guide/mcp-clients.md).

### Started by other programs

| Command | Run by |
| --- | --- |
| `vexdom daemon [--no-idle-exit]` | The Bridge process itself, started by `start` and on demand |
| `vexdom native-host` | The browser, through the native messaging manifest `setup` registers |

---

## See also

- [Tools](tools.md): the MCP tools and every action
- [Errors](errors.md): what each code means
- [Configuration](../guide/configuration.md): `config.json` and `vexdom.json`
- [Troubleshooting](../guide/troubleshooting.md)
- [internals/contributing.md](../internals/contributing.md): building from source
