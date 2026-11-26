# Auth vault

Save a login once from a shell, encrypted on this computer, and let an agent sign in with it by name.
The Bridge types the password into the page itself; no result, error or log line carries it.

```sh
printf '%s\n' "$PASSWORD" | vexdom auth save work --url https://app.example.com/login \
  --username ada@example.com --password-stdin
vexdom auth login work
```

---

## Commands

| CLI | MCP |
| --- | --- |
| `vexdom auth save <name> --url u --username u (--password-stdin \| --password p) [--username-selector s] [--password-selector s] [--submit-selector s]` | not offered |
| `vexdom auth login <name> [--url u] [--no-navigate] [--credential-provider p --item i] [--timeout ms]` | `act {"do":"authLogin","name":"work"}` |
| `vexdom auth list` | `act {"do":"authList"}` |
| `vexdom auth show <name>` | `act {"do":"authShow","name":"work"}` |
| `vexdom auth delete <name>` | `act {"do":"authDelete","name":"work"}` |

The Bridge answers them (`bridge.authSave`, `bridge.authLogin`, `bridge.authList`,
`bridge.authShow`, `bridge.authDelete`); only `authLogin` needs the browser. `authList` and
`authShow` are read-only. The CLI prints each result as JSON on one line; `--json` prints the
`{"ok":true,"data":…}` envelope.

---

## Saving a login

`vexdom auth save` needs a name (letters, digits, `_`, `-`), the login page's `--url`, the
`--username`, and the password:

- `--password-stdin` reads the first line of stdin. An empty line fails with "Password from stdin is
  empty".
- `--password p` works, but prints
  `⚠ Passwords on the command line may be visible in process listings and shell history. Use --password-stdin instead.`

`--username-selector`, `--password-selector` and `--submit-selector` are CSS selectors stored with
the login, for forms the defaults below do not find. Saving under an existing name replaces it.

The result is `{"saved":"work"}`. The first save on a machine with no key also creates one and adds
`"keyCreated":true` and
`"notice":"Auto-generated encryption key at /Users/ada/.vexdom/.vault-key -- back up this file or set VEXDOM_VAULT_KEY"`.

Saving is yours alone:

- `vexdom mcp` never offers `bridge.authSave` (`act {"do":"authSave"}` answers `UNKNOWN_ACTION`), so
  a password never sits in an MCP client's transcript.
- Side-panel runs do not get it either; a call answers `BLOCKED`: "Saving a login is for the user
  (vexdom auth save); an agent run cannot store passwords."
- Where the side panel shows an external `authSave` call, the password reads `********`.

---

## Storage

| Path | What |
| --- | --- |
| `~/.vexdom/vault/<name>.json` | One login, AES-256-GCM, mode `0600` in a `0700` folder |
| `~/.vexdom/.vault-key` | The key: 64 hex characters (256 bits), mode `0600`, created by the first save |

The key comes from `VEXDOM_VAULT_KEY` when set (64 hex characters; `openssl rand -hex 32` makes one),
else from `~/.vexdom/.vault-key`. With the variable set, no key file is created. `VEXDOM_HOME` moves
both paths.

Each file is an envelope `{"version":1,"encrypted":true,"iv","authTag","data"}` (base64, a fresh
12-byte IV per save) around the login's JSON: name, URL, username, password and the selectors. An
unencrypted file is not accepted.

Back up the key: without it the logins cannot be opened.

| Code | When |
| --- | --- |
| `VAULT_KEY_MISSING` | Neither `VEXDOM_VAULT_KEY` nor the key file exists, or the one in use is not 64 hex characters |
| `AUTH_PROFILE_NOT_FOUND` | No login by that name ("Auth profile 'work' not found") |
| `ACTION_FAILED` | The key does not open the file ("Decryption failed: …") |
| `INVALID_INPUT` | A bad name ("Must match /^[a-zA-Z0-9_-]+$/") or a missing field |

---

## Listing, showing, deleting

```json
{"profiles":[{"name":"work","username":"ada@example.com","url":"https://app.example.com/login"}]}
```

`auth list` lists every file; one the key cannot open is listed as
`{"name":"…","error":"Failed to decrypt"}`. `auth show <name>` answers
`{"profile":{"name","url","username","usernameSelector","passwordSelector","submitSelector"}}`.
Neither ever returns the password. `auth delete <name>` removes the file and answers
`{"deleted":"work"}`.

---

## Logging in

`auth login <name>` runs on one tab, the active tab when it starts, so a tab switched to midway
never receives the password.

1. Opens the login's URL (`page.navigate`). `--url` uses another page for this login only; it must
   be on the saved login's origin (`INVALID_TARGET` otherwise), so whoever asks for a login never
   picks where its password goes. The origin the tab lands on is pinned: each fill and the click
   first check the page is still there.
2. Waits for the username field: the given or saved selector, else, for up to 5 s,
   `input[type=email]`, `input[name=email]`, `input[autocomplete=username]`, `input[name*=user i]`
   and similar, then for the rest of the timeout `input[type=text]` or `input:not([type])`. A field
   counts once it is visible and enabled. Fills it with `page.fillInput`.
3. Waits for the password field (default `input[type=password]`) and fills it. The password is
   typed only into an element with `type=password`: a selector pointing anywhere else fails with
   `INVALID_TARGET` "'‹selector›' is not a password field; the password is typed only into one".
4. Waits for the submit button (default `button[type=submit]`, `input[type=submit]`,
   `button:not([type])`) and clicks it.
5. Waits up to 10 s for a navigation to start and load; with none, 2 s.

The result is `{"loggedIn":true,"name":"work"}`. It says the form was submitted, not that the site
accepted the password: check for a page only a signed-in user sees.

| Flag | Input | Effect |
| --- | --- | --- |
| `--no-navigate` | `noNavigate: true` | Log in on the page already open. Its origin must match the login's URL (or `--url`), checked before the credential is read and again before each fill and the click |
| `--url u` | `url` | A login page to use instead of the saved one, on the saved login's origin; with `--no-navigate`, only the origin to require |
| `--username-selector`, `--password-selector`, `--submit-selector` | `usernameSelector`, `passwordSelector`, `submitSelector` | Override the saved selectors for this login |
| `--timeout ms` | `timeoutMs` | How long each wait lasts, default 25000; the whole login can take a few times that |
| `--credential-provider p --item i` | `credentialProvider`, `item` | Take the login from a plugin instead of the vault (below) |

| Code | When |
| --- | --- |
| `TIMEOUT` | A field or the button did not appear, e.g. "Timed out waiting for password selector 'input[type=password]'" |
| `INVALID_TARGET` | `--url` on another origin than the login's; `--no-navigate` on a page that is not HTTP(S), or whose origin differs from the login's; the page left the login's origin before a fill or the click; the password selector is not a `type=password` field |
| `TAB_UNREACHABLE` | The page changed during a `--no-navigate` login, or did not say its URL once the login page opened |
| `EXTENSION_OFFLINE` | No browser connected |
| `PLUGIN_FAILED` | The credential plugin failed |

An error the page returns while the password is filled has the password replaced. A flow recording
running on the tab drops every request that carries it ([Saved flows](flows.md#what-is-never-saved)).

One-time codes are not handled: the agent asks you for the code and fills it.

---

## Logins from a credential plugin

```sh
vexdom auth login --credential-provider keychain --item "My App" --url https://app.example.com/login
```

`credentialProvider` names a configured [plugin](plugins.md) with the `credential.read` capability;
`item` is passed to it as `itemRef`, and `name` (which defaults to `item`) as `profileName`. The
plugin returns the username, the password and optionally the URL and selectors; `--url` wins over
the plugin's URL but must be on its origin (`INVALID_TARGET` otherwise). Nothing is written to the
vault. A failure is `PLUGIN_FAILED` with a generic message
("Plugin 'keychain' returned success=false", "Plugin 'keychain' timed out", "Credential plugin
'keychain' returned an incomplete credential"), never the plugin's own `error` text, because it may
have echoed the secret. The lookup may take 15 s.

---

## The playbook

The `auth` playbook (`crates/vexdom-core/assets/skills/auth/SKILL.md`) tells an agent to list saved
logins, log in by name, confirm it got in, and never ask you to paste a password. MCP clients get it
as the prompt `auth`, the resource `skill://auth/SKILL.md`, and through `guide {"topic":"auth"}`.

---

## See also

- [Plugins](plugins.md): credential plugins and their protocol
- [Configuration](../configuration.md#environment): `VEXDOM_VAULT_KEY`, `VEXDOM_HOME`
- [Approvals](../approvals.md): how secrets are sealed in results
- [MCP clients](../mcp-clients.md)
- [reference/errors.md](../../reference/errors.md)
