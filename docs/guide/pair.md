# Pair your browser

Pairing connects the extension in one browser to Vexdom Bridge, once per browser. With the Bridge
installed, the extension pairs by itself: no code to copy. This page covers how that works, how to
make it ask you first or turn it off, pairing with a code, what pairing protects, and managing
several browsers.

It assumes you have [installed the Bridge and added the extension](install.md).

---

## 1. It connects by itself

The extension finds the Bridge through the browser's native-messaging host, which `vexdom setup`
registers (and every Bridge start refreshes), and the two connect. The extension tries when it is
installed, when the browser starts, when you open the side panel, and every minute or so while it is
not paired, more slowly when there is nothing to find. If the Bridge is down, the host starts it.

The Bridge then issues a long-lived session key that survives browser and Bridge restarts and lasts
until you revoke it. Updating either side does not need a new pairing.

## 2. Choose how browsers pair

```sh
vexdom pairing          # show how browsers pair now
vexdom pairing auto     # by themselves (the default)
vexdom pairing ask      # once you allow each one
vexdom pairing code     # only with a code
```

The setting is `pairing.mode` in `~/.vexdom/config.json`; the [Mac app](mac-app.md) writes the
same key. The extension shows the setting but cannot change it, because the browser is what the
setting guards.

**Ask** shows each new browser to you before it connects:

- **In the Vexdom app**, a notification and a row on Overview ask *Allow ‹browser›?*, with a name
  field ("Work Chrome"), **Deny** and **Allow**.
- **From a terminal**, `vexdom requests` lists the browsers waiting, `vexdom approve <id>` lets one
  in, `vexdom approve --watch` asks about each one as it comes, and `vexdom deny <id>` turns one
  down. `vexdom setup` also asks while it waits.

An allowed browser pairs by itself from then on. A denied one stays out until someone presses
**Reconnect** in that browser and you allow it.

**Code** pairs nothing by itself. It is also the switch to reach for if you ever want automatic
pairing off at once.

## 3. Pair with a code

```sh
vexdom pair
```

This starts the Bridge if it is not already running and prints a code:

```

  Pairing code:  K7QM-3XPT

  Open the Vexdom side panel, paste it, and press Connect.
  Expires in 10 minutes and works once.

```

Click Vexdom in the toolbar, then the status pill (or **Set up the link** in the empty chat), press
**Pair with a code instead**, enter the code and press **Connect**. Up to three codes are live at
once, so a code the app shows keeps working while a terminal prints another.

A code always works, whatever `pairing` is set to: use it for a browser the Bridge cannot register
with (section 6) or a managed browser whose policy blocks native messaging.

## 4. Verify

```sh
vexdom status
```

```
bridge:    running on 127.0.0.1:8785 (pid 41207, v0.9.2)
wake-up:   Chrome, Firefox can start the Bridge
agent:     Claude Code — 2.1.332 (Claude Code)
browsers:
  ● Chrome           v0.9.2    unpacked          connected
  ○ Firefox          v0.9.2    Firefox add-on    not connected
tools:     in sync
pairing:   automatic: a browser with Vexdom connects by itself
```

| Line | What it means |
| --- | --- |
| `bridge` | The Bridge is up, on one of ports 8785–8787 |
| `wake-up` | Which browsers can start the Bridge, and pair, when it is down. `held since "vexdom stop"` means it stays down until `vexdom start` |
| `agent` | The CLI the side panel runs on, and whether it is ready |
| `browsers` | One row per paired browser: its extension version, where the extension came from, and whether it is connected now. An unpacked copy a build behind the folder on disk adds `↻ at chrome://extensions loads v…` |
| `tools` | `in sync` means the extension and the Bridge carry the same action manifest. When they differ, the Bridge uses the extension's own list |
| `pairing` | How a new browser pairs (section 2) |

`vexdom sessions` also says how each browser paired: by itself, allowed, or with a code.
`vexdom doctor` checks the same things and names the fix for each.

---

## What pairing protects, and what it does not

Any web page can open a WebSocket to loopback, so the Bridge classifies every connection by its
handshake `Origin`, which browsers set themselves and page JavaScript cannot forge. A web page is
refused outright. An extension origin must then prove it holds a pairing code, a one-time grant or a
session key, and the Bridge proves itself back, so another local process cannot squat the port and
pose as your Bridge. No secret crosses the socket.

Automatic pairing gets its grant through the native-messaging host, which the browser launches only
for the extensions its manifest names: in Chromium browsers the unpacked folder's extension ID and
the IDs of browsers already paired, in Firefox the signed add-on's ID. Web pages cannot reach it. A
grant works once, for two minutes, for that one browser profile and the extension origin the browser
vouched for, and a grant handed out is taken back the moment the setting changes to `code` or the
browser is revoked.

What that leaves: someone at your keyboard who loads a copy of the extension that claims Vexdom's
ID (Developer mode in Chromium, a temporary add-on in Firefox) is paired by itself in `auto` mode.
In `ask` mode you see it and can deny it.

What pairing does **not** do is authenticate local programs. Anything running as your user can read
`~/.vexdom/daemon.json` and drive an already-paired browser. Vexdom treats your user account as the
trust boundary. See [Limits](limits.md#pairing-controls-which-browser-not-which-process) and, for
the mechanism, [internals/transport.md](../internals/transport.md).

---

## Managing pairings

```sh
vexdom sessions                    # which browsers are paired, and which are kept out
vexdom rename <id> "Work Chrome"   # name one
vexdom revoke                      # unpair every browser
vexdom revoke <id>                 # unpair one, by the id "sessions" prints
vexdom approve <id>                # let a revoked or denied browser back in
```

Revoking a browser also keeps it from pairing again by itself, so it does not come back a minute
later. It returns when you press **Reconnect** in that browser (in `auto` mode), or pair it with a
code. In `ask` mode, allow it as well. **Disconnect** in the extension does the same from the
browser's side: it stays unpaired until you press **Connect** there.

Several browsers can be paired **and connected at once**: Chrome beside Firefox, or two Chrome
profiles. Each keeps its own key, and the side panel in each runs on its own. A browser is known by
an install id it mints on first run, never by its extension origin, and a newer connection
supersedes an older one only when both come from the same browser profile.

The CLI and MCP clients reach the browser you were last in. They stay with that browser through a
burst of calls, because tab ids only mean something in the browser that issued them, and follow you
again once it has been quiet for two minutes. `guide` (no topic) names the connected browser for an
MCP client; `vexdom status` does for you.

---

## 5. When it does not connect by itself

The side panel's **Connection** sheet and the welcome page say why. The usual reasons:

| It says | What to do |
| --- | --- |
| Vexdom Bridge isn't on this computer yet | Install it ([Install](install.md)). Already installed? Run `vexdom setup`, which registers this browser |
| Vexdom Bridge needs an update | `vexdom update` |
| Allow this browser in the Vexdom app | `pairing` is `ask`: allow it in the app, or run `vexdom approve` |
| This computer unpaired this browser | It was revoked. Press **Reconnect** |
| Automatic pairing is off on this computer | `pairing` is `code`: pair with a code, or run `vexdom pairing auto` |
| Your organization's browser policy keeps Vexdom Bridge from starting | Pair with a code |
| Vexdom Bridge is turned off | `vexdom start`, or the switch in the app |

## 6. Browsers it registers with

`setup` and every Bridge start register the native host where each installed browser reads it, and
always in Chrome's and Mozilla's locations, since Brave, Opera and other forks read Chrome's.

On macOS: Chrome and its Beta, Dev and Canary channels, Chrome for Testing, Chromium, Edge and its
channels, Brave (and Beta, Nightly), Vivaldi, Arc, Opera, Opera GX, Comet, Thorium, Firefox and its
channels, Zen, LibreWolf, Floorp and Waterfox. Linux has the equivalent list. A browser installed
after setup is registered the next time the Bridge starts. A Snap or Flatpak browser on Linux cannot
run the host: pair it with a code.

`VEXDOM_HOME` pointing outside your home folder skips registration, so a scratch Bridge never takes
your browsers over; `VEXDOM_REGISTER_HOST=1` registers it anyway.

---

## Next

**[First run →](first-run.md)**: a tour of the side panel and a first instruction to try.

Driving the same browser from Claude Code in a terminal is optional and takes one more command:
[MCP clients](mcp-clients.md).
