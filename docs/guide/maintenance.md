# Checking, updating and uninstalling

`vexdom doctor` checks an install and repairs what it safely can, `vexdom update` installs the
latest signed release over the copy you run, and `vexdom uninstall` removes everything Vexdom wrote.

---

## Doctor

```sh
vexdom doctor            # every check, with a fix for each warning
vexdom doctor --fix      # also repair what it safely can
vexdom doctor --offline  # skip the checks that reach GitHub
vexdom doctor --json     # {"success", "summary": {pass, warn, fail}, "checks": [{id, category, status, message, fix?}], …}
```

| Check | Warns or fails when | `--fix` |
| --- | --- | --- |
| `env.version`, `env.state_dir`, `env.user_dir`, `env.disk_free` | `~/.vexdom` is not writable; less than 500 MB free | |
| `bridge.running` | The Bridge is not running, or a lockfile names a process that is gone | Removes the stale lockfile |
| `bridge.version` | The running Bridge is another version than this `vexdom` | Restarts the Bridge |
| `native_host.registered` | The native-messaging host is missing or out of date | Registers it |
| `extension.connected`, `extension.version`, `extension.manifest` | No browser is connected; an unpacked copy is behind the folder on disk; the extension's action manifest differs from the Bridge's | Refreshes the unpacked folder, then says where to press ↻ |
| `agent.claude` | Claude Code is missing or not ready | |
| `config.user`, `config.project` | `~/.vexdom/config.json` or `./vexdom.json` does not parse, or the project file has keys it may not set | |
| `security.state_perms`, `security.vault_key` | `~/.vexdom` is not `0700`, or `auth.json`, `config.json` or the vault key is not `0600` | `chmod` |
| `recording.ffmpeg` | `ffmpeg` is not on `PATH` (only `record` needs it; `brew install ffmpeg`) | |
| `net.github`, `update.available` | GitHub is unreachable; a newer release is out | |

The text form groups checks by category, prints `fix:` under each warning, and ends with
`Summary: N pass, N warn, N fail`. The exit code is non-zero when a check fails.

---

## Updating

```sh
vexdom update --check     # is a newer release out?
vexdom update             # install it and restart the Bridge
```

`update` asks GitHub for the latest release, downloads `SHA256SUMS` and `SHA256SUMS.minisig`, checks
the minisign (ed25519) signature against the public key compiled into `vexdom`, then checks the
asset's SHA-256 against the signed list. A bad signature stops with `SIGNATURE_INVALID`; nothing
is replaced. What it installs depends on how this copy was installed:

| This `vexdom` is | `update` installs |
| --- | --- |
| `<dir>/bin/vexdom` from `install.sh` | `vexdom-<version>-<target>.tar.gz`, replacing `bin/` and `extension/` |
| inside `Vexdom.app` | `Vexdom-<version>-<target>.zip`, staged beside the app and swapped in (the running app is asked to quit, and reopens on the new version) |
| anything else (`cargo install`, a checkout) | Nothing: `UPDATE_UNMANAGED: <path> was not installed from a release; reinstall with install.sh or rebuild it` |

Then it runs `setup --no-pair --restart` with the new binary, which refreshes the unpacked extension
folder and restarts the Bridge; your browsers stay paired. `--no-restart` skips that step, `--json`
prints `{from, to, layout, restarted}`, and `--local` only re-runs setup with the binary you have.

**The extension.** An unpacked copy never reloads itself: after an update, press ↻ on its card at
`chrome://extensions` (or **Update** in the side panel); `vexdom status` says which browser needs
it. The Firefox add-on checks the release's `updates.json` about once a day; **Check for Updates**
in `about:addons` checks now, and the side panel offers **Update** once it has one.

When an update adds a permission to the extension, Chrome disables it until you accept the new
permission on its card. Until then the browser is unpaired and page actions answer
`EXTENSION_OFFLINE`. Nothing is lost: accepting reconnects the pairing you had.

**The Bridge runs on demand.** The first command, MCP client or browser that needs it starts it
(so does `vexdom start`), and it runs until `vexdom stop` or 30 idle minutes. A rebuild from source
does not touch the process already running: `vexdom restart` swaps it.

---

## Uninstalling

```sh
vexdom uninstall
```

**Remove the extension from each browser first** (right-click its toolbar icon → **Remove**, or
`about:addons` in Firefox). Recordings and held secrets live in extension storage, not on disk, and
removing an unpacked copy after its folder is gone leaves the browser holding a broken one.

The command prints what it will remove and asks first:

| | |
| --- | --- |
| Bridge | Whatever answers on the Bridge's ports, not what the lockfile claims. Sessions are revoked through it first, so a connected browser is told it is unpaired |
| state | `~/.vexdom`: pairing keys, config, approvals, saved states, logs |
| files | `~/vexdom`: the unpacked extension, skills, site maps, screenshots, downloads, captures |

| Flag | Does |
| --- | --- |
| `--dry-run` | Print the plan and stop |
| `--yes` / `-y` | Skip the question. Required when stdin is not a terminal |
| `--keep-skills` | Leave `skills/` behind. Nothing else has a copy of your site maps |

It names, but does not remove, the `vexdom` binary itself, `Vexdom.app`, the entry in your MCP
client (`claude mcp remove vexdom`), and any folder you moved elsewhere with `screenshotDir`,
`downloadDir` or `skillsDir`. Deleting `~/.vexdom` by hand leaves a running Bridge holding its port;
`vexdom stop` probes the ports rather than trusting the lockfile, so it still finds it. It stops
only the Bridges serving its own `VEXDOM_HOME` (each reports a hash of its state folder on
`/health`), so a test run with its own home leaves yours alone; `vexdom uninstall` stops every one.

---

## Development

From `packages/extension/`:

```sh
bun run dev                 # build, launch a throwaway Chrome profile, hot reload
bun run dev:firefox
bun run build               # production build (ext-core wasm first)
bun run bridge:build        # target/release/vexdom
bun run tools:emit          # the action manifest the Bridge embeds, after a registry change
bun run check               # ext-core, Rust gates, tsc, tests with coverage floors
```

What to run before a pull request: [internals/contributing.md](../internals/contributing.md).
