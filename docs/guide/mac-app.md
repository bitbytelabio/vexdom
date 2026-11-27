# The Mac app

`Vexdom.app` puts the Bridge behind a power switch. It starts and stops the Bridge, pairs browsers,
answers approvals, picks the side panel's model and holds a few settings. For all of this it runs
the `vexdom` binary inside its own bundle, so anything it does can also be done from a terminal. The
app has five tabs: **Overview**, **Browsers**, **Agent**, **Settings** and **About** (⌘1 to ⌘5).

It needs macOS 14 or newer, on Apple silicon (`aarch64-apple-darwin`) or Intel (`x86_64-apple-darwin`).

---

## Getting it

**With the install script** (recommended):

```sh
curl -fsSL https://github.com/bitbytelabio/vexdom/releases/latest/download/install.sh | sh
```

On macOS the script installs the Bridge in `~/.local/share/vexdom` (see [Install](install.md)) and
then downloads `Vexdom-<version>-<target>.zip`. It checks the zip against its `.sha256`, unpacks it
with `ditto` and moves `Vexdom.app` into `/Applications`, or into `~/Applications` when it cannot
write to `/Applications`. An existing `Vexdom.app` is kept until the new one is in place. The script
then clears the quarantine flag on the new copy. `VEXDOM_NO_APP=1` skips the app;
`VEXDOM_VERSION=0.9.2` installs a specific release.

**From the release page.** Each [release](https://github.com/bitbytelabio/vexdom/releases) has the
app twice for each target: `Vexdom-<version>-<target>.zip`, and `Vexdom-<version>-<target>.dmg`, a
disk image that you open and drag **Vexdom** onto **Applications**. Each file has a `.sha256`
next to it.

The app is signed ad hoc. It has no Developer ID and is not notarized, so Gatekeeper blocks a copy
that a browser downloaded. Clear the quarantine flag once:

```sh
xattr -dr com.apple.quarantine /Applications/Vexdom.app
```

The bundle holds everything the app runs:

```
Vexdom.app/Contents/MacOS/vexdom-desktop                  the window
Vexdom.app/Contents/MacOS/vexdom                          the Bridge, the CLI and the MCP server
Vexdom.app/Contents/Resources/extension/chrome-mv3/       the extension build, for setup to copy
```

---

## First run: the checks

Until two required checks pass, the app shows a **Set up Vexdom** screen of check rows in place of
the tabs. Once they pass, the screen goes away by itself.

| Check | Passes when | Button when it fails |
| --- | --- | --- |
| **The vexdom command** | `vexdom` is inside the app bundle, or on your login shell's `PATH` | None: "There is no vexdom beside this app or on your PATH. Reinstall Vexdom." |
| **Browsers can wake the Bridge** | `vexdom doctor --json --offline` reports `native_host.registered` as `pass` | **Set up**, which runs `vexdom setup --no-pair --no-open --no-wait --json` |
| **A browser** (optional) | `vexdom browsers --json` lists an installed browser | **Get Chrome**, which opens google.com/chrome |
| **Claude Code** (optional) | The running Bridge reports Claude Code as installed | **Install Claude Code**, which runs `npm i -g @anthropic-ai/claude-code`. It is greyed out, with "npm not found", when `npm` is not on your `PATH` |

**Set up everything** runs the fixes for every failed required check, in order, and runs the checks
again when they finish. **Check again** reruns the checks without fixing anything. The app only
asks about Claude Code while the Bridge is running, because asking would start one. With the Bridge
off, that row reads "Start Vexdom to look for Claude Code."

**About → Run the checks again** brings the screen back. Opened that way, it stays until you press
**Continue**.

---

## Overview

**The switch.** The round orange button starts the Bridge (`vexdom start`) or stops it
(`vexdom stop`). While a start or stop is in progress it reads **Starting** or **Stopping** and
ignores clicks. Next to it, a lamp and one line give the state:

| Lamp | Line |
| --- | --- |
| Unlit | "Looking for the Bridge" (before the first answer), or "Vexdom is off" |
| Amber | "Waiting for your browser", or "‹browser› wants to connect" |
| Green | "‹browser› is connected", or "‹n› browsers are connected" |

While the Bridge runs, the face also shows its **Address** (`127.0.0.1:<port>`), the **Agent**
("Claude Code ‹version› · ‹model›", or "Claude Code is not ready"), the browsers that are
**Paired**, and a **Restart** button (`vexdom restart`). The app asks the Bridge for its status
every 2 seconds.

**Start the Bridge when the app opens.** If Settings → General → **Start Vexdom when the app opens**
is on (the default), the app runs `vexdom start` once at launch when no Bridge is running.
Otherwise the Bridge stays off until you press the switch, or until a browser, the CLI or an MCP
client starts it.

**A browser asking to connect.** In [`ask` pairing mode](pair.md#2-choose-how-browsers-pair), each
new browser shows up as a face titled **Allow ‹browser›?**, with a name field already filled in
with the browser's name (40 characters at most), **Deny** and **Allow**. In the field, Enter allows
and Escape denies. When more than one browser is waiting, they come one at a time and the face
says how many are left. Each new request also posts a macOS notification ("‹browser› wants to
connect to Vexdom"). While the app is connected, it is the approver the Bridge names:
`vexdom pairing` prints "The Vexdom app asks you about each new browser."

**Actions waiting for your approval.** When a guardrail holds an action from the CLI or an MCP
client, it appears under **Waiting for your approval** as **Allow ‹action› on ‹site›?**, with the
action's summary and three buttons:

| Button | Does |
| --- | --- |
| **Deny** | Refuses it; the caller gets `DECLINED` |
| **Always on ‹site›** | Allows it, and stops asking for that action on that site. Shown only when the action has a site |
| **Allow** | Allows it once |

Each new request also posts a notification, and an amber dot on the Overview tab shows that
something is waiting while you are on another tab. Answering a request that has stopped waiting
reports "That action stopped waiting for an answer." Side-panel runs ask in the side panel, not
here. See [Approvals](approvals.md#from-the-cli-or-an-mcp-client).

**Notices.** When a monitor, scheduled task or reminder finishes while its browser is out of sight,
the Bridge gives the notice to the app. The app posts a notification and adds the notice to the
**Notices** list. **Open** on a row asks the Bridge to bring that browser's tab back. Each notice
is shown once.

---

## Browsers

**Pairing** (needs the Bridge running):

- **Auto / Ask / Code** sets [`pairing.mode`](pair.md#2-choose-how-browsers-pair). The line next to
  the buttons explains the selected mode.
- **Get a pairing code** shows a code as `XXXX-YYYY` with a `mm:ss` countdown: "Type it into the
  extension. It works for ‹mm:ss›." The code disappears when it expires. See
  [Pair with a code](pair.md#3-pair-with-a-code).
- **Unpair all** appears once a browser is paired. It asks "Unpair every browser? Each one has to
  pair again." before it revokes every session.

**Browsers on this Mac** has one row for each browser that `vexdom browsers --json` finds
installed or paired. Each row shows the browser's state: "Connected", "Paired, not open",
"Extension not paired", or "Start Vexdom to see its state". A browser that is not connected gets a
button:

| Browser | Button | Does |
| --- | --- | --- |
| Chrome, Edge, Brave, Arc, Vivaldi, Opera, Chromium | **Load it unpacked** | Runs `vexdom setup --browser <id> --unpacked --no-pair --no-wait --json`, which writes the extension to `~/vexdom/extension/chrome-mv3`. Then it copies that path to the clipboard and opens the browser on its extensions page |
| Firefox | **Add extension** | Runs `vexdom setup --browser firefox --json --no-pair`, which opens the add-on's page in Firefox |

After **Load it unpacked**, the strip at the bottom of the window shows the remaining steps for 45
seconds: turn on Developer mode, click **Load unpacked**, press ⇧⌘G in the folder picker, paste,
and press Return. The folder picker hides dot-folders, which is why the extension goes in
`~/vexdom` and not `~/.vexdom`.

**Paired** lists every paired browser, with its name, when it was last seen and an **Unpair**
button. A **Reload needed** badge appears on a browser that runs an unpacked extension older than
the build in the folder (for example after an update). It says which version the browser runs and
which page to reload it on: press ↻ on the extension's card there.

---

## Agent

The side panel runs Claude Code. This tab shows whether it is ready, with its version, or what is
wrong:

| State | Button |
| --- | --- |
| Not installed (`AGENT_MISSING`) | **Install Claude Code** (`npm i -g @anthropic-ai/claude-code`) |
| Installed but cannot run (`AGENT_UNUSABLE`, …) | **Fix** (`vexdom agent fix claude`), with the Bridge's own hint above it |

**Model** offers **Default** plus every model Claude Code reports. Picking one runs
`vexdom agent model claude <model>`, and **Default** runs `vexdom agent model claude`. With the
Bridge off, the tab shows **Start Vexdom** and nothing else. See [The side-panel agent](agents.md).

---

## Settings

Settings has four pages, chosen from the list on the left.

### General

| Section | What it holds |
| --- | --- |
| **This app** | **Appearance**: System, Light or Dark. **Start Vexdom when the app opens**. **Open at login**, which registers the app as a login item; it is shown only for a bundled `Vexdom.app`. If macOS turned the login item off, the row says to allow Vexdom in System Settings → General → Login Items |
| **Browser theme** | The look of the side panel in every paired browser: Neon, Ember, Midnight, Phosphor or Daylight (`theme` in [config.json](configuration.md#appearance)) |
| **Guardrails** | Each rule with **Allow**, **Ask** or **Block** ("Locked" for the rules you cannot change); **Fence page text**; **When nobody can answer**: **Refuse** or **Allow** (`guardrails.unattended`). See [Approvals](approvals.md#changing-the-policy) |
| **Command line** | Where the app's `vexdom` is. **Link vexdom** puts a symlink to the bundle's `vexdom` in the first of `~/.local/bin`, `/opt/homebrew/bin` or `/usr/local/bin` that is on your login `PATH`. It never touches a `vexdom` file it did not create, and **Unlink** removes only its own link. The **Claude Code** row has the MCP registration line, `claude mcp add vexdom -- <path> mcp`, with **Copy** ([MCP clients](mcp-clients.md#register)) |
| **Uninstall** | See [Uninstall](#uninstall) |

The app sends Browser theme and Guardrails changes to the Bridge, which writes them to
`~/.vexdom/config.json`, the same keys the extension's settings page writes
([Configuration](configuration.md#approvals-and-guardrails)). With the Bridge off, the app writes
`config.json` itself.

### Skills

Every skill from `vexdom skills --json`, with its description and where it comes from. The field
filters by name and description (Escape clears it); **All / Bundled / User / Uploaded** filters by
source. See [Skills](features/skills.md).

### Activity

- **Standing approvals**: one row per site, listing the actions you allowed with **Always on
  ‹site›**. **Forget** runs `vexdom approvals clear <host>`.
- **Downloads**: the files agents captured (`vexdom downloads --json`), with size, site and date.
  **Show** reveals a file in Finder; **Clear all** runs `vexdom downloads clear`.

### Logs

The last 400 lines of the Bridge's log, `~/.vexdom/daemon.log`, reread every second while the page
is open. The view stays at the end of the log unless you scroll up.

---

## About

- **Versions**: the app, the Bridge (while it runs), and the extension in each connected browser,
  each with **Copy**.
- **Updates**: see below.
- **Help**: **Report a bug** opens a new GitHub issue with those versions and your macOS version
  already filled in; nothing is sent until you submit the issue. **Docs**, **Release notes**, and
  **Run the checks again**.

### Updates

The app checks GitHub's latest release when it opens and then every 6 hours. **Check for updates**
checks right away. The card shows where the update stands:

| State | The card says | Button |
| --- | --- | --- |
| Not checked yet | "Vexdom looks for a new version when it opens and every six hours." | **Check for updates** |
| Checking | "Looking for a new version…" | |
| Up to date | "Vexdom ‹version› is the latest version." | **Check for updates** |
| Newer release | "Vexdom ‹new› is out; this is ‹current›." | **Update to v‹new›** |
| Downloading, verifying | "Downloading Vexdom ‹new›…", "Checking Vexdom ‹new›'s signature and bundle…" | |
| Relaunching | "Vexdom ‹new› opens as soon as this one quits…" | |
| Failed | The error | **Check for updates** |

**Update to v‹new›** works like this:

1. It downloads `Vexdom-<version>-<target>.zip` with the release's `SHA256SUMS` and
   `SHA256SUMS.minisig`.
2. It checks the minisign signature against the release key built into the app, then checks the
   zip's SHA-256 against the signed list.
3. It unpacks the zip next to the running app and checks that the bundle is `com.vexdom.desktop`
   at the version it was offered.
4. It quits. The new app replaces the old one and opens.
5. On its first launch, the new app runs `vexdom setup --no-pair --restart`, which restarts the
   Bridge and rewrites the unpacked extension folder. Your browsers stay paired; an unpacked
   extension shows **Reload needed** until you reload it.

If anything fails, the card shows the error (`SIGNATURE_INVALID`, `CHECKSUM_MISMATCH`,
`UPDATE_ARCHIVE_INVALID`, …; see [Errors](../reference/errors.md#vexdom-update)) and the installed
app is not changed. A copy that is not running from inside `Vexdom.app` answers `UPDATE_UNMANAGED`.
`vexdom update` run with the `vexdom` inside the bundle (for example through **Link vexdom**)
updates the app the same way; see [Maintenance](maintenance.md#updating).

---

## The menu bar and the window

The menu-bar item, a switch-and-lamp icon, has a menu with:

- the state line: the same line as Overview, or "‹browser› wants to connect", or "‹n› actions wait
  for your approval";
- **Start Vexdom** or **Stop Vexdom**;
- **Restart Vexdom** (only while the Bridge runs);
- **Open Vexdom**;
- **Quit Vexdom**.

Closing the window only hides the app; it keeps running in the menu bar, and **Open Vexdom** brings
it back. **Quit Vexdom** (or ⌘Q) quits the app but leaves the Bridge running: the Bridge is a
separate process, so the side panel and MCP clients keep working. While the app is connected, the
Bridge does not exit when idle. Once the app is gone, the Bridge exits after
30 minutes with no clients.

Notifications come from Vexdom. If they never show up, allow them for Vexdom in System Settings →
Notifications.

Light or dark follows macOS unless **Appearance** says otherwise. The app keeps its own settings in
`~/.vexdom/desktop.json`: `appearance`, `startBridgeOnLaunch`, and `finishUpdateOnLaunch` (set
while an update is being installed).

---

## Uninstall

**Settings → General → Uninstall…** asks first, with **Keep my skills** ticked by default. Then it
removes the app's `vexdom` links and runs `vexdom uninstall --yes`, adding `--keep-skills` when the
box is ticked. That stops the Bridge and deletes `~/.vexdom` and `~/vexdom`: pairings, settings,
approvals, logs and the unpacked extension. The window goes back to the first-run checks.
`Vexdom.app` stays until you move it to the Bin.

Before you uninstall, remove the extension from each browser (right-click its toolbar icon →
**Remove**). See [Maintenance](maintenance.md#uninstalling).

---

## See also

- [Install](install.md): the script and the Bridge without the app
- [Pair](pair.md): pairing modes, codes and what pairing protects
- [Approvals](approvals.md): what asks, and how standing approvals work
- [MCP clients](mcp-clients.md): registering `vexdom mcp`
- [Configuration](configuration.md): the `config.json` keys the app writes
- [Maintenance](maintenance.md): doctor, update, uninstall
- [Troubleshooting](troubleshooting.md)
- [Internals: the Mac app](../internals/mac-app.md)
