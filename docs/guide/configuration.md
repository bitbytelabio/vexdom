# Configuration

The keys Vexdom reads from `~/.vexdom/config.json`, the project file `./vexdom.json`,
`~/.vexdom/profile.json` and the environment, with their defaults and limits. Every key is
optional; Vexdom runs with no config file at all.

The Bridge re-reads `config.json` on every use, so an edit applies to the next thing you ask without
a restart. It also watches the file, so the extension's settings page and the Mac app show a hand
edit as soon as you save it. Writers (the settings page, the Mac app, `vexdom pairing`,
`vexdom agent model`) change one key and keep everything else in its place.

`vexdom doctor` checks that both `config.json` and `./vexdom.json` parse. Where each file lives:
[internals/state.md](../internals/state.md).

---

## A complete example

```json
{
  "agent": "claude",
  "agents": {
    "claude": { "bin": "/opt/homebrew/bin/claude", "model": "opus", "effort": "high" }
  },
  "requireApproval": ["page.submitForm", "page.evaluate"],
  "maxConcurrentRuns": 3,
  "theme": "midnight",
  "pairing": { "mode": "auto" },
  "guardrails": {
    "rules": { "raw-html-read": "allow" },
    "unattended": "deny",
    "urlPayloadBytes": 512,
    "hosts": [],
    "fence": true
  },
  "screenshotDir": "~/vexdom/screenshot",
  "downloadDir": "~/vexdom/download",
  "downloadTtlDays": 14,
  "skillsDir": "~/vexdom/skills",
  "siteMap": {
    "research": true,
    "allowClicks": false,
    "maxPages": 15,
    "maxScreenshots": 10,
    "timeoutMs": 600000
  },
  "cli": { "json": false, "maxOutput": 20000, "timeoutMs": 5000 }
}
```

---

## The side-panel agent

| Key | Default | Notes |
| --- | --- | --- |
| `agent` | `claude` | The CLI the side panel runs on. Claude Code is the only one |
| `agents.claude.bin` | `claude` | Absolute path to the binary. Set it when the Bridge's `PATH` differs from your shell's, the usual cause of `AGENT_MISSING` |
| `agents.claude.model` | `sonnet` | Passed as `--model`. The side panel's model select and `vexdom agent model claude <model>` write it. A value starting with a dash or holding a space is ignored |
| `agents.claude.effort` | unset | Passed as `--effort`: `low`, `medium`, `high`, `xhigh` or `max`; anything else is dropped |

The older top-level `claudeBin`, `model` and `effort` keys are still read as the Claude settings.
See [The side-panel agent](agents.md).

## Runs

| Key | Default | Max | Notes |
| --- | --- | --- | --- |
| `maxConcurrentRuns` | 3 | 8 | How many tab sessions may have a run going at once. Past it a new run gets `RUN_LIMIT` |

## Approvals and guardrails

The settings page (**Guardrails**) and the Mac app write these keys one row at a time. Turning a row
back off deletes its line rather than writing the default.

| Key | Default | Notes |
| --- | --- | --- |
| `requireApproval` | `["page.submitForm"]` | Actions that must be approved every time. Listing `page.submitForm` also catches `pressEnter: true` and pressing Enter, because those submit forms too. `[]` gates no form |
| `guardrails.rules` | none | Override any rule's effect by id: `allow`, `confirm` or `deny` |
| `guardrails.unattended` | `deny` | `allow` turns every `confirm` for a CLI or MCP caller into `allow`, so it no longer waits for you. Any other value leaves it waiting |
| `guardrails.urlPayloadBytes` | 512 | Query string plus fragment size above which a navigation counts as carrying a payload |
| `guardrails.hosts` | none | Hosts added to every side-panel run's scope. `["*"]` turns host confinement off |
| `guardrails.fence` | `true` | Whether page text is wrapped and marked as untrusted data |

[Approvals](approvals.md) lists every rule id, including the four the settings page shows locked.

## Pairing

| Key | Default | Notes |
| --- | --- | --- |
| `pairing.mode` | `auto` | `auto` pairs a browser by itself, `ask` once you allow it, `code` only with a typed code. Any other value pairs only by code. `vexdom pairing <mode>` writes it. See [Pair](pair.md#2-choose-how-browsers-pair) |

## Paths

| Key | Default | Notes |
| --- | --- | --- |
| `screenshotDir` | `~/vexdom/screenshot` | Where kept screenshots are written, mode `0600` |
| `downloadDir` | `~/vexdom/download` | Where files captured with `captureDownload` are written, mode `0600` |
| `downloadTtlDays` | 14 | How long a captured download is kept before the Bridge sweeps it. Fractions work: `0.5` is twelve hours |
| `skillsDir` | `~/vexdom/skills` | Where uploaded skills and generated site maps live |
| `extensionDir` | `~/vexdom/extension/chrome-mv3` | Where `vexdom setup` writes the unpacked extension. `setup --dir <dir>` writes it here for you. Chrome derives the extension ID from the path, so after a change load the new folder again |

HAR files, PDFs, traces and profiles that an action is not given a `path` for go to
`~/vexdom/captures/`, which is not configurable.

## Site mapping

| Key | Default | Ceiling | Notes |
| --- | --- | --- | --- |
| `siteMap.research` | `true` | | Lets a mapping run use web search for public background on the domain |
| `siteMap.allowClicks` | `false` | | Lets a mapping run reach routes that only exist behind an interaction |
| `siteMap.maxPages` | 15 | 40 | |
| `siteMap.maxScreenshots` | 10 | 24 | |
| `siteMap.timeoutMs` | 600 000 (10 min) | 1 800 000 (30 min) | |

A value above the ceiling is clamped; one that is not a number ≥ 1 falls back to the default. See
[Site maps](features/site-maps.md).

## Appearance

| Key | Default | Notes |
| --- | --- | --- |
| `theme` | absent (Neon) | `neon`, `ember`, `midnight`, `phosphor` or `daylight`: the look of the side panel and the settings page in every paired browser |

Pick it under **Appearance** on the extension's settings page, or under **Browser theme** in the Mac
app. A browser that has never been paired keeps its choice and hands it to the Bridge when it first
connects. The settings page's **Extension** section also holds per-browser settings (action cues,
the right-click menu, keyboard shortcuts, hands-free) that are kept in that browser, not in
`config.json`.

## Plugins

`plugins` lists the credential and command plugins `vexdom plugin add` registered; see
[Plugins](features/plugins.md). Plugins are read from `config.json` (and `VEXDOM_PLUGINS`) only,
never from a project's `vexdom.json`.

---

## CLI preferences

Defaults for the page commands (`vexdom snapshot`, `click`, `screenshot`…) and for `vexdom mcp`:

| Key | Type | What it sets |
| --- | --- | --- |
| `json` | boolean | Print every page command's result as `{"ok":true,"data":…}` / `{"ok":false,"error":{"code","message"}}`, as `--json` does |
| `maxOutput` | number | The output limit, as `--max-output` does; `0` is no limit. The MCP server's default is 20,000 characters |
| `annotate` | boolean | Draw ref labels on screenshots, as `--annotate` does |
| `screenshotFormat` | `png` or `jpeg` | As `--screenshot-format` |
| `screenshotQuality` | 0–100 | As `--screenshot-quality` |
| `timeoutMs` | number | The default `timeoutMs` for element commands |

They come from two places, and the later wins:

1. `"cli": { … }` in `~/.vexdom/config.json`, yours;
2. `./vexdom.json` in the directory you run `vexdom` from (never a parent directory).

Environment (`VEXDOM_MAX_OUTPUT`) overrides both, and a flag overrides everything.

**A project file is untrusted**, since it comes with whatever repository you are in. Only the six
keys above (and `$schema`) are read from it. Anything else is ignored with one warning on stderr:

```
⚠ ignoring 2 unsupported settings from ./vexdom.json (project config is untrusted). Put them in ~/.vexdom/config.json under "cli".
```

A key with the wrong type skips the whole file: `invalid config file ./vexdom.json: expected a JSON
object with correctly typed preferences`.

---

## Profile

The **Profile** section of the extension's settings page writes `~/.vexdom/profile.json`, which keeps
personal details out of `config.json`. It is re-read before every run, and every field is optional:

```json
{
  "fields": {
    "givenName": "Ada",
    "familyName": "Lovelace",
    "email": "ada@example.com",
    "street": "12 St James’s Square\nFlat 3",
    "country": "United Kingdom"
  },
  "details": [{ "label": "Frequent flyer", "value": "BA 123456" }],
  "instructions": "Always choose the cheapest shipping."
}
```

| Key | Limit | Notes |
| --- | --- | --- |
| `fields` | 300 characters each | `givenName`, `familyName`, `email`, `phone`, `street`, `city`, `region`, `postalCode`, `country`, `company`, `jobTitle` |
| `details` | 20 entries; label 60, value 300 characters | Anything the fields do not cover |
| `instructions` | 4,000 characters | Kept on every task |

An entry that breaks a limit, or a key the page does not know, is left out rather than cut short.
[Profile](features/profile.md) covers what the agent is told.

---

## Environment

| Variable | Effect |
| --- | --- |
| `VEXDOM_HOME` | Moves the state directory from `~/.vexdom`. A Bridge whose state lives outside your home folder does not register the native host, so a scratch copy never takes your browsers over |
| `VEXDOM_REGISTER_HOST=1` | Register the native host anyway |
| `VEXDOM_PORTS` | Comma-separated ports to try instead of 8785, 8786, 8787 |
| `VEXDOM_LOCKFILE` | Where clients find the running Bridge, instead of `~/.vexdom/daemon.json` |
| `VEXDOM_MAX_OUTPUT` | The output limit for page commands and `vexdom mcp` |
| `VEXDOM_DEBUG` | Also echo the Bridge's log to stderr |
| `VEXDOM_VAULT_KEY` | The [auth vault](features/auth-vault.md)'s key, 64 hex characters, instead of `~/.vexdom/.vault-key` |
| `VEXDOM_PLUGINS` | A JSON array of [plugins](features/plugins.md); when it parses, it replaces `config.json`'s list |
| `VEXDOM_READ_PRIVATE` | Set (any value) in the environment of `vexdom mcp`: `snapshot {"as":"markdown","url":…}` may fetch loopback, private and link-local addresses instead of answering `READ_REFUSED` |

`~/vexdom` (the visible half: the unpacked extension, skills, screenshots, downloads, captures) is
not relocatable.

---

## Things that are not configurable

| | |
| --- | --- |
| Open tab sessions | 8 |
| Concurrent background monitors | 3 |
| Recording duration | 15 minutes, warning at 13 |
| Full-page screenshot | 48 tiles, 16 384 px canvas side |
| System prompt | 64 KB |
| Waiting for an approval (CLI, MCP) | 120 seconds |
| Loopback ports | 8785, 8786, 8787 (unless `VEXDOM_PORTS`) |

---

## See also

- [Approvals](approvals.md): the gate, rule by rule
- [internals/state.md](../internals/state.md): every file Vexdom writes
- [reference/cli.md](../reference/cli.md): commands that read and write this config
