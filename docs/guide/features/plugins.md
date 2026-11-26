# Plugins

A plugin is a program Vexdom starts on demand: it gets one JSON request on stdin and answers with
one JSON response on stdout (protocol `vexdom.plugin.v1`). Plugins supply logins to
[auth vault](auth-vault.md) sign-ins and answer their own request types through
`vexdom plugin run`. Vexdom keeps the browser automation, the policy and the redaction; a plugin
never drives the browser.

```sh
vexdom plugin add ./keychain-plugin.py
vexdom plugin list
vexdom auth login --credential-provider keychain --item "My App" --url https://app.example.com/login
```

---

## Capabilities

| Capability | Used by | Request `type` |
| --- | --- | --- |
| `credential.read` | `vexdom auth login --credential-provider <plugin> --item <i>`, `act {"do":"authLogin","credentialProvider":"…","item":"…"}` | `credential.resolve` |
| `command.run` | `vexdom plugin run <plugin> <type>` for any type the plugin does not list as a capability | the type you give |

A plugin is only sent a request under a capability it declares; otherwise the call fails with
"Plugin '‹name›' does not declare required capability '‹capability›'". `vexdom plugin run` with a
type the plugin lists among its capabilities sends it under that capability instead of
`command.run`.

There is no MCP action for plugins other than `authLogin`'s `credentialProvider`.

---

## Configuration

Plugins are read from two places only:

- `~/.vexdom/config.json`, key `plugins`: an array of entries. `vexdom plugin add` writes it, keeping
  every other key. See [Configuration](../configuration.md#plugins).
- `VEXDOM_PLUGINS`: a JSON array of entries. When it parses, it **replaces** config.json's list;
  when it does not, it is ignored with a warning.

A project's `./vexdom.json` never adds a plugin; `vexdom plugin add --project` fails with "plugins
are kept only in ~/.vexdom/config.json; a project's vexdom.json cannot add them".

```json
{
  "plugins": [
    {
      "name": "keychain",
      "command": "/Users/ada/bin/keychain-plugin.py",
      "args": [],
      "capabilities": ["credential.read"],
      "source": "path:/Users/ada/bin/keychain-plugin.py"
    }
  ]
}
```

| Field | Meaning |
| --- | --- |
| `name` | What `--credential-provider` and `plugin run` call it. When two entries share a name, the later one is used |
| `command`, `args` | The program and its arguments, started directly (no shell) |
| `capabilities` | What it may be asked |
| `source` | Where `plugin add` found it: `path:…`, `npm:…` or `github:…` |

---

## Commands

| Command | Effect |
| --- | --- |
| `vexdom plugin` or `vexdom plugin list` | `‹name›  ‹capabilities›` per plugin, or "No plugins configured" |
| `vexdom plugin add <ref> [--name N] [--capability C]... [--no-manifest]` | Register a plugin (`install` is the same) |
| `vexdom plugin show <name>` | `Name:`, `Command:`, `Args:`, `Capabilities:` |
| `vexdom plugin run <name> <type> [--payload <json>]` | Send one request and print the response |

`--json` prints `{"plugins":[…]}`, `{"plugin":{…}}`, the response, or
`{"success":true,"plugin":{…},"configPath":"…"}` for `add`. Errors go to stderr, or as
`{"success":false,"error":"…"}` with `--json`. Warnings about an unreadable config or
`VEXDOM_PLUGINS` go to stderr as `⚠ …`.

### `plugin add`

The reference decides how the plugin is started:

| Reference | Command |
| --- | --- |
| A path starting `./`, `../`, `/` or `~/` (must be an existing file) | The file itself, by its absolute path |
| `owner/repo` | `npx -y github:owner/repo` |
| Anything else, scoped (`@acme/vexdom-plugin-vault@1.2.0`) or not | `npx -y <ref>` |

Unless `--no-manifest`, `add` first runs the plugin with a `plugin.manifest` request (60 s limit). The
name is `--name`, else the manifest's `name`, else derived from the reference with a
`vexdom-plugin-` or `plugin-` prefix or a `-plugin` suffix removed. A name cannot be empty or
contain whitespace or `:`. The capabilities are those given with `--capability` (repeatable, or
comma-separated; `--capabilities` is the same flag), else the manifest's. With none, `add` fails:
"Plugin did not declare capabilities; rerun with --capability <name> or use a plugin that supports
plugin.manifest". When the manifest fails and no `--capability` was given:
`PLUGIN_FAILED: … Rerun with --capability <name> to add the plugin without manifest discovery.`

An entry with the same name is replaced. The output:

```text
Added plugin 'keychain'
Config: /Users/ada/.vexdom/config.json
Source: path:/Users/ada/bin/keychain-plugin.py
Command: /Users/ada/bin/keychain-plugin.py
Capabilities: credential.read
```

### `plugin run`

Sends `--payload` (default `{}`) as the request and prints the plugin's whole response. It refuses
the core entry points `credential.read`, `credential.resolve` and `plugin.manifest`: "plugin run
cannot invoke core plugin entrypoint '…'; use the dedicated vexdom command path". A failure is
printed as `PLUGIN_FAILED: Plugin '‹name›' failed: ‹the plugin's error›`. The limit is 60 s.

---

## The protocol

Vexdom starts `command` with `args`, writes one JSON object to stdin, closes stdin, and reads stdout
to the end. stderr is discarded, so nothing a plugin prints there can leak.

The environment is cleared first: a plugin sees only `PATH`, `HOME`, `USER`, `LOGNAME`, `LANG`,
`LC_ALL`, `TMPDIR` and `SHELL` (those that are set), never the vault key or API keys from the
Bridge's environment. Its working directory is `~/.vexdom` (`VEXDOM_HOME` when set), so a
relative path the plugin opens is resolved from there.

The request:

```json
{
  "protocol": "vexdom.plugin.v1",
  "type": "credential.resolve",
  "capability": "credential.read",
  "request": { "profileName": "My App", "itemRef": "My App", "url": "https://app.example.com/login" }
}
```

| `type` | `capability` | `request` |
| --- | --- | --- |
| `plugin.manifest` | `plugin.manifest` | `{}` |
| `credential.resolve` | `credential.read` | `profileName`, `itemRef` (or `null`), `url` (or `null`) |
| what `plugin run` names | that type if declared, else `command.run` | the `--payload` |

The response must be one JSON object with `"protocol":"vexdom.plugin.v1"` and `"success":true`, and
the process must exit 0:

```json
{ "protocol": "vexdom.plugin.v1", "success": true,
  "manifest": { "name": "keychain", "capabilities": ["credential.read"], "description": "Logins from the macOS Keychain" } }
```

```json
{ "protocol": "vexdom.plugin.v1", "success": true,
  "credential": { "username": "ada@example.com", "password": "…", "url": "https://app.example.com/login",
                  "usernameSelector": null, "passwordSelector": null, "submitSelector": null } }
```

- A manifest may also come under `data` instead of `manifest`.
- A credential needs a non-empty `username` and `password`; `url` and the three selectors are
  optional. A login with no URL from either the plugin or `--url` fails with "Credential has no
  URL". `otp` is accepted and not used.
- To fail, answer `"success":false` with an `"error"` text. `plugin run` shows that text; a
  credential lookup never does.

| Failure | Message |
| --- | --- |
| Not started | `Failed to start plugin '‹name›': …` |
| Over the limit (15 s for a credential, 60 s otherwise) | `Plugin '‹name›' timed out`; the process is killed |
| Exit status not 0 | `Plugin '‹name›' exited unsuccessfully` |
| stdout is not JSON | `Plugin '‹name›' returned invalid JSON` |
| Wrong `protocol` | `Plugin '‹name›' used unsupported protocol` |
| `success` not `true` | `Plugin '‹name›' returned success=false` |

Through `authLogin` all of these are `PLUGIN_FAILED`.

---

## A minimal credential plugin

A macOS plugin that reads a login from a Keychain generic password whose service is the item name:

```python
#!/usr/bin/env python3
import json, re, subprocess, sys

PROTOCOL = "vexdom.plugin.v1"
call = json.load(sys.stdin)

def answer(**fields):
    json.dump({"protocol": PROTOCOL, **fields}, sys.stdout)

if call.get("type") == "plugin.manifest":
    answer(success=True, manifest={"name": "keychain", "capabilities": ["credential.read"],
                                   "description": "Logins from the macOS Keychain"})
elif call.get("type") == "credential.resolve":
    req = call["request"]
    service = req.get("itemRef") or req["profileName"]
    attrs = subprocess.run(["security", "find-generic-password", "-s", service],
                           capture_output=True, text=True)
    secret = subprocess.run(["security", "find-generic-password", "-s", service, "-w"],
                            capture_output=True, text=True)
    account = re.search(r'"acct"<blob>="([^"]*)"', attrs.stdout)
    if attrs.returncode or secret.returncode or not account:
        answer(success=False, error=f"no Keychain item {service}")
    else:
        answer(success=True, credential={"username": account.group(1),
                                         "password": secret.stdout.rstrip("\n"),
                                         "url": req.get("url")})
else:
    answer(success=False, error="unsupported request type")
```

```sh
chmod +x keychain-plugin.py
security add-generic-password -s "My App" -a ada@example.com -w   # prompts for the password
vexdom plugin add ./keychain-plugin.py
vexdom auth login --credential-provider keychain --item "My App" --url https://app.example.com/login
```

macOS may ask once whether `security` may read the item. On Linux, `secret-tool lookup` plays the
same part.

---

## See also

- [Auth vault](auth-vault.md): `auth login` and `--credential-provider`
- [Configuration](../configuration.md#plugins): the `plugins` key and `VEXDOM_PLUGINS`
- [MCP clients](../mcp-clients.md)
- [reference/cli.md](../../reference/cli.md)
