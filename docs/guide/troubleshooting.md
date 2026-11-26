# Troubleshooting

Causes and fixes for common problems, by the symptom or message you see. For what an error code
means, see [reference/errors.md](../reference/errors.md).

---

## Start here

```sh
vexdom doctor      # every check, each warning with its fix; --fix repairs what it can
vexdom status      # the Bridge, each paired browser, the agent
vexdom logs        # run starts, routed skills, every action and its outcome
```

The Bridge's log is `~/.vexdom/daemon.log`; `VEXDOM_DEBUG=1` also echoes it to stderr.

---

## Setup and connection

| Symptom | Cause | Fix |
| --- | --- | --- |
| `vexdom: command not found` | `~/.local/share/vexdom/bin` is not on `PATH` | Add it, as `install.sh` printed |
| "No Vexdom daemon is running" | The Bridge is not running on 8785–8787 | `vexdom start`, then `vexdom status` and `vexdom logs` |
| `vexdom setup` keeps waiting for the browser | The extension is not loaded yet, pairing is `code`, or it waits for you to allow it | Open the side panel: it says which. Enter the code `setup` printed, or allow the browser. Ctrl-C stops the wait without undoing anything |
| The panel says **Vexdom Bridge isn't on this computer yet**, but it is | This browser has no native host registered (installed after setup, or unknown to the Bridge) | `vexdom setup`, or `vexdom doctor --fix`. Or pair with a code |
| The panel says **Allow this browser in the Vexdom app** | `pairing` is `ask` | Allow it in the app, or `vexdom approve`. `vexdom requests` lists who waits |
| The panel says **This computer unpaired this browser** | It was revoked | Press **Reconnect**; in `ask` mode also allow it |
| The panel says **Automatic pairing is off on this computer** | `pairing` is `code` | Pair with a code, or `vexdom pairing auto` |
| The panel says **Vexdom Bridge needs an update** | The Bridge is older than the extension expects | `vexdom update` |
| A protocol version mismatch in the **Connection** sheet | Extension and Bridge speak different link versions | `vexdom update`, then press ↻ on the unpacked card |
| "That pairing code is wrong or expired" | Codes work once and last 10 minutes | `vexdom pair` for a new one |
| `vexdom status` says two copies answer in one browser | Two Vexdom extensions are loaded | Remove one on the browser's extensions page |
| Updated, but the browser still runs the old extension | A browser never reloads an unpacked extension | Press ↻ on its card, or **Update** in the side panel |
| The panel stays offline after `vexdom stop` | `stop` also holds the browser's wake-up | `vexdom start`; `vexdom status` shows `held` until then |
| "Load unpacked" cannot see `~/vexdom` | A Flatpak or Snap browser is sandboxed from your home folder | `vexdom setup --unpacked --dir <a folder it can read>`, or grant access (`flatpak override --user --filesystem=~/vexdom …`) |
| `vexdom update` says `UPDATE_UNMANAGED` | This `vexdom` was built or `cargo install`ed, not installed from a release | Reinstall with `install.sh`, or rebuild |
| `vexdom update` says `SIGNATURE_INVALID` | The downloaded `SHA256SUMS` does not match its signature | Nothing was replaced. Try again later; report it if it persists |
| `EXTENSION_OFFLINE` | The browser is closed, or not paired | Open it; `vexdom sessions` shows pairing |

## The side-panel agent

| Symptom | Cause | Fix |
| --- | --- | --- |
| `AGENT_MISSING` | `claude` is not on the *Bridge's* `PATH` | Set `agents.claude.bin` to an absolute path |
| `AGENT_FAILED: Not logged in` | Claude Code is signed out | Run `claude` and sign in |
| *could not start Vexdom's browser tools* | A Bridge still running from files an update replaced | `vexdom restart`, then send the message again |
| `RUN_IN_PROGRESS` | A tab runs one instruction at a time | Cancel the running one, or use another tab |
| `RUN_LIMIT` | `maxConcurrentRuns` runs are going | Wait, or raise it (max 8) |
| `TAB_IN_USE` | That tab belongs to another conversation | Switch to it from the Sessions strip |

## Pages and tabs

| Symptom | Cause | Fix |
| --- | --- | --- |
| `TAB_UNREACHABLE` on a normal site | The extension needs reloading | Press ↻ on its card |
| `TAB_UNREACHABLE` on `chrome://`, the new-tab page | Those pages cannot host a content script | `vexdom open <url>` (or `navigate`) still works there |
| `UNKNOWN_REF` | The ref is not in the tab's latest snapshot | `vexdom snapshot` (MCP: `snapshot`), then use the new ref |
| `STALE_REF` | The page reloaded since the snapshot | Take a new snapshot |
| `NO_SNAPSHOT` | A ref was used before any snapshot of that tab | Take one first |
| `TARGET_NOT_FOUND` | The page changed, or the locator matches nothing (`--exact` compares whole text) | Take a new snapshot; loosen the locator |
| `INVALID_TARGET: … is disabled` / `… is hidden` / `… covers (x, y)` | The click pre-checks refused | Wait for the element, close the overlay, or `--force` (MCP: `"force": true`) |
| `INVALID_INPUT: date inputs take YYYY-MM-DD …` | A date, time, colour, range or number input got a value of the wrong shape | Use the format the message shows |
| `EVAL_FAILED` from `vexdom eval` in Chrome, or nothing runs | **Allow User Scripts** is off | `chrome://extensions` → Vexdom → **Details** → turn it on |
| `UNSUPPORTED` | A debugger feature in Firefox, or a feature this browser lacks | Use a Chromium browser; see [Limits](limits.md#the-debugger-features-are-chromium-only) |
| `DEBUGGER_UNAVAILABLE` | DevTools is open on that tab (Chrome allows one debugger per tab) | Close DevTools |
| `DEBUGGER_DETACHED` | The debugger left the tab (you closed the bar, the tab closed) | Call again to re-attach; routes, HAR and emulation on that tab start over |
| `vexdom console` prints *Capture started on this tab* | Capture starts on first use and sees nothing from before | Reproduce the problem, then run it again |
| `FFMPEG_MISSING` | `record` needs ffmpeg | `brew install ffmpeg` |
| `AXE_UNAVAILABLE` | The first `a11y` could not fetch axe-core from the npm registry | Check the network and run it again; later runs use the cache |
| A captcha keeps setting new challenges | The vendor distrusts the browser | Solve one round yourself. See [Captchas](features/captcha.md) |
| New tabs open Vexdom Home | That is [Vexdom Home](features/new-tab.md) | Switch it off under **Settings → Extension → New tab** |

## Approvals

| Symptom | Cause | Fix |
| --- | --- | --- |
| A CLI command or MCP call hangs, then `APPROVAL_TIMEOUT` | The action needs approval and nobody answered within 120 s | Answer in the Vexdom app, `vexdom approve c-…`, or the client's prompt. See [Approvals](approvals.md) |
| `DECLINED` | Someone denied it | Do not retry; it was a decision |
| `PAGE_CHANGED` after you approved | The tab moved to another site while you were asked, so the action was not run | Go back to the page and let the agent ask again |
| `vexdom approve` says it needs you at a terminal | stdin is not a terminal (an agent or a script ran it) | Approve in the Vexdom app, the MCP client's prompt, or your own terminal |
| `BLOCKED` | A rule denies it (`raw-html-read`, `network-body-read`, page code from outside the panel…) | Read the message; change the rule in `config.json` only if you mean it |
| `SITE_BLOCKED` | The site is on your [Blocked sites](features/blocked-sites.md) list | Intended. Remove it from the list in the extension's settings if you want it back |

## MCP clients

| Symptom | Cause | Fix |
| --- | --- | --- |
| No vexdom tools in a session | The server was registered mid-session, or the client cannot find `vexdom` | Restart the session; give the absolute path to `vexdom` |
| `UNKNOWN_ACTION` | `do` named no action this browser offers | `guide {}` lists them; Chromium-only actions are absent in Firefox |
| `READ_REFUSED` from `snapshot {"as":"markdown","url":…}` | The URL is, or resolves to, `localhost`, a private or a link-local address; agents read public pages only | Read it yourself with `vexdom read <url>`, open it in a tab and `snapshot {"as":"markdown"}`, or start `vexdom mcp` with `VEXDOM_READ_PRIVATE=1` |
| A result ends with a note that it was cut | Over the 20,000-character limit | Narrow it (`filter`, `selector`, `maxChars`), or raise `--max-output` |
| `tools:` in `vexdom status` says *the extension's own list* | Extension and Bridge are different versions | Nothing to fix while one waits for its update; `vexdom update` and ↻ bring them together |

## Behaviour that looks wrong but is not

| Symptom | Why |
| --- | --- |
| An action ran but nothing appears in `logs` | It matched the local command grammar and never reached the Bridge (it carries a ⚡) |
| `awaitMonitor` returns `settled: false` | The poll window ended while the watch continues. Call again |
| A theme change vanished | Themes do not survive a reload. Reapply it |
| A site map was written but nothing changed | Maps are staged for review. **Skills** → **Activate** |
| Chrome shows "Vexdom is debugging this browser" | A debugger feature holds the tab. It goes when the feature releases it (`set reset`, `network unroute`, `har stop`…) |
| A second `snapshot` prints only a few lines | It is the delta from the last one. `--full` (MCP: `full: true`) prints the whole tree |

---

## Still stuck

- **Report a bug** on the extension's About page, or the About tab of the Mac app, opens GitHub's
  issue form with your versions filled in
- [Limits](limits.md): it may be a known limit
- [internals/](../internals/): how each part works
