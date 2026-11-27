# The macOS app

`Vexdom.app` is a GPUI window over the Bridge, built from `desktop/`. It implements nothing the
Bridge or the CLI already does. It reads state over the control socket and answers pairing and
approval requests there; everything that changes disk, starts processes or installs something goes
through the `vexdom` binary in its own bundle. The guide page is
[guide/mac-app.md](../guide/mac-app.md). The visual rules are in
[`desktop/DESIGN.md`](../../desktop/DESIGN.md).

---

## The crate

`desktop/` is a Cargo workspace of its own (an empty `[workspace]` table in
[`desktop/Cargo.toml`](../../desktop/Cargo.toml)), so GPUI never enters the root `Cargo.lock` or the
Linux CI (ADR-026). It builds one binary, `vexdom-desktop`, and uses these crates from the main
workspace:

| Crate | Used for |
| --- | --- |
| `vexdom-client` | `Bridge::existing`, the Cap'n Proto `Control` client over the control socket, `watch`, `confirmations`/`confirm`, `block_on` |
| `vexdom-core` | `paths` (state dir, `daemon.log`, `write_private`), `preferences` (read and apply without a Bridge), `config::THEMES` |
| `vexdom-schema` | `Topic`, the watchable events |
| `vexdom-update` | Finding, verifying and staging a new `Vexdom.app` (ADR-031) |

From outside the workspace: `gpui` 0.2.2, `tray-icon` (the menu-bar item), `notify-rust`
(notifications), `objc2-app-kit`/`objc2-foundation` (Dock icon, unhide, bundle id),
`objc2-service-management` (`SMAppService`), and `tokio` (current-thread runtime and child
processes).

### Modules

The modules that decide what to show do not depend on GPUI, so their tests run without a window.

| Module | GPUI | Holds |
| --- | --- | --- |
| [`main.rs`](../../desktop/src/main.rs) | yes | Loads `desktop.json`, starts the bridge thread, opens the Settings window hidden (760 × 600, min 680 × 520, transparent title bar), binds ⌘Q and ⌘1–⌘7, hides the window instead of closing it, brings it back on reopen (`on_reopen`, through the `MainWindow` global) |
| [`bridge.rs`](../../desktop/src/bridge.rs) | no | The bridge thread: the connection loop, `Command` in, `Update` out, every CLI call, the update flow |
| [`model.rs`](../../desktop/src/model.rs) | no | Turns `Control` replies and `vexdom … --json` output into rows: headline and lamp, facts, pair requests, notices (`NoticeBook`), sessions, browsers, `reload_needed`, agent card, confirmations, preferences and guardrail rows, skills, approvals, downloads, `UpdateState`, the bug-report URL, `mcp_command` |
| [`preflight.rs`](../../desktop/src/preflight.rs) | no | The four first-run checks: `evaluate(Findings)` decides pass/fail and the fix |
| [`settings.rs`](../../desktop/src/settings.rs) | no | `~/.vexdom/desktop.json` |
| [`log_tail.rs`](../../desktop/src/log_tail.rs) | no | Incremental tail of `daemon.log`: the last 400 lines; the first read covers at most the last 256 KiB; a shrunk file is reread from its start |
| [`shell.rs`](../../desktop/src/shell.rs) | no | The login shell's `PATH`, `which`, and **Link vexdom** (`link_dirs`, `link`, `unlink`, `linked_in`) |
| [`login_item.rs`](../../desktop/src/login_item.rs) | no | **Open at login** through `SMAppService.mainAppService`; `None` when unbundled |
| [`tray.rs`](../../desktop/src/tray.rs) | no | The menu-bar item: `Pick` (each item's id), `MenuState`, the layout as data (`entries`) and its `muda` items; the Dock policy (`set_dock_visible`), the notification sender, the Dock icon for `cargo run` |
| [`ui.rs`](../../desktop/src/ui.rs) | yes | `AppView`: the seven tabs, requests and approvals pinned above them, the first-run screen, the status strip, and the `MenuState` it hands the menu-bar item |
| [`theme.rs`](../../desktop/src/theme.rs) | yes | Light and dark palettes, the bundled Archivo faces (`desktop/fonts/`), buttons and faces |
| [`text_input.rs`](../../desktop/src/text_input.rs) | yes | A one-line field on the platform input handler (IME composition works), with Enter and Escape events; ported from GPUI's `examples/input.rs` |

---

## Threads and messages

Cap'n Proto clients are `!Send`, so the link to the Bridge has its own thread, `vexdom-bridge`, which
runs a current-thread tokio runtime inside a `LocalSet` (`vexdom_client::block_on`). The window and
that thread exchange messages over two unbounded channels:

- `Command` (window → thread): `Start`, `Decide`, `Confirm`, `SetPreference`, `InstallUpdate`, ….
- `Update` (thread → window): `Status`, `Sessions`, `PairRequests`, `Confirmations`, `Notices`,
  `Preflight`, `UpdateState`, `Said`/`Error` for the status strip, `Quit`, ….

The window receives `ui::Inbound`, which is either a bridge `Update` or a menu-bar `Pick`. Both
arrive on the same channel, so a menu click goes through the same code as the matching button.
After each update or pick, and after a setting changes or a pending control settles, the window
builds a `MenuState` and hands it to the menu-bar item, which rebuilds its menu only when it
differs from the last one. A click on a check item flips its mark by itself, so a pick also makes
the next state rebuild the menu (`Tray::forget`).

A control that has sent a command (`Start`, `Stop`, `Restart`, a pairing `Decide`, a `Confirm`) is
dimmed and ignores clicks until the matching update arrives, or for at most 20 seconds (60 for
`Restart`).

---

## Talking to the Bridge

### The connection loop

`run()` in `bridge.rs` loops on `Bridge::existing(None)`. That reads the lockfile and probes the
ports but never starts a Bridge (see
[Transport](transport.md#the-control-token-and-the-lockfile)).

**No Bridge.** The thread sends `Disconnected`. The first time, it also lists browsers and runs the
first-run checks. Then it waits 2 seconds, or for a command, whichever comes first. If
`startBridgeOnLaunch` is set and a `vexdom` was found, the first pass runs `vexdom start` instead,
once. Commands that need a connection (`Decide`, `Revoke`, `Confirm`, `PairCode`, `SetPairing`,
`OpenNotice`) answer "Vexdom is off."

**A Bridge.** `connected()` subscribes to four topics, then fetches status, sessions, pair
requests, notices, confirmations, the agent and the browser list. After that it polls `status` and
`sessions` every 2 seconds and refetches whatever a topic event names:

| Topic | Watched with | On an event |
| --- | --- | --- |
| `PairRequestsChanged` | approver `"app"` | `pairRequests`, `sessions` |
| `NoticesChanged` | `allowed = true` | `takeNotices` |
| `SettingsChanged` | | `agent`, `preferences` |
| `ConfirmationsChanged` | | `confirmations` |

`ManifestChanged` is not watched. The match over `Topic` is exhaustive, so adding a topic to the
schema breaks this build until the app handles it.

When a status call fails, the thread drops the connection and returns to the loop. `Stop`,
`Restart` and `Uninstall` run their CLI command and drop it too.

Watching `PairRequestsChanged` with approver `"app"` is what makes the Bridge report
`approver: "app"` (`approver_now()` in `crates/vexdom-daemon/src/lib.rs`). While the app is
connected, `vexdom pairing` says the app asks about each new browser. The open control connection
also counts as a client, so the Bridge does not exit when idle while the app is up (see the
[process table](overview.md#the-processes)).

### Control calls

| Call | For |
| --- | --- |
| `status` | Headline, address, pairing mode, Bridge version |
| `sessions` | Paired browsers, extension versions, source |
| `pairRequests`, `decide` | Pairing requests in ask mode, with the name from the field as `label` |
| `pair` | A pairing code and its `expiresAt` |
| `pairing` (set) | Auto / Ask / Code |
| `revoke` | One session, or every session when `session` is empty |
| `confirmations` and `confirm` (via `vexdom-client`) | External actions held by a guardrail; `remember` is **Always on ‹site›** ([Guardrails](guardrails.md#external-confirmations-adr-032)) |
| `takeNotices`, `openNotice` | Notices from monitors, schedules and reminders |
| `agent` (empty `set`, `grant`, `models`) | Reads Claude Code's state and model list |
| `preferences`, `setPreference` | Browser theme and guardrails, `{kind: "theme", theme}` or `{kind: "guardrail", setting, value}`; a rule's own default is written as `null` |

With the Bridge off, `Preferences` and `SetPreference` call `vexdom_core::preferences::preferences_now`
and `apply_preference` directly. In that case `config.json` is the only copy, and the next Bridge
reads it when it starts.

### CLI subprocesses

Everything else runs the `vexdom` binary. `vexdom_path()` picks the one next to the app's own
executable (`Contents/MacOS/vexdom`), or else the first `vexdom` on the login `PATH`.

| Action | Command |
| --- | --- |
| Switch, Restart | `vexdom start`, `vexdom stop`, `vexdom restart` |
| First-run **Set up** | `vexdom setup --no-pair --no-open --no-wait --json` |
| First-run checks | `vexdom doctor --json --offline` (its `native_host.registered` check), `vexdom browsers --json` |
| **Add extension** | `vexdom setup --browser <id> --json --no-pair` |
| **Load it unpacked** | `vexdom setup --browser <id> --unpacked --no-pair --no-wait --json`, then `/usr/bin/open -a "<App>" <extensionsPage>` |
| Agent | `vexdom agent model claude [model]`, `vexdom agent fix claude` |
| **Install Claude Code** | `npm i -g @anthropic-ai/claude-code` (`npm` found on the login `PATH`) |
| Skills, Activity | `vexdom skills --json`, `vexdom approvals --json`, `vexdom approvals clear <host>`, `vexdom downloads --json`, `vexdom downloads clear` |
| Uninstall | `vexdom uninstall --yes [--keep-skills]`, after removing the app's own links |
| Finishing an update | `vexdom setup --no-pair --restart --no-open --no-wait --json` |

`cli_json` parses stdout whatever the exit code, because `vexdom doctor` exits 1 on a failed check
and still prints its report. A failed command shows the last non-empty line it wrote in the status
strip. The app's `--json` parsing is in `model.rs`, so if a command's output shape changes, update
it there.

### The login `PATH`

An app opened from the Finder gets launchd's `PATH`, which has neither Homebrew nor `~/.local/bin`.
The Bridge that `vexdom start` launches would inherit that `PATH` and report Claude Code as
`AGENT_MISSING`. So `shell::login_path()` asks `$SHELL -l -c` (`/bin/zsh` when `SHELL` is unset)
once, on the bridge thread. The shell prints its `PATH` between `__VEXDOM_PATH__` markers, so
whatever its startup files print is skipped. If the shell does not answer within 2 seconds, the
app falls back to `/opt/homebrew/bin`, `/usr/local/bin`, `/usr/bin`, `/bin` and `~/.local/bin`. Every
child (`shell::command`) gets that `PATH` and a null stdin.

**Link vexdom** symlinks the bundle's `vexdom` into the first of `~/.local/bin`, `/opt/homebrew/bin`
and `/usr/local/bin` that is on the login `PATH` and accepts the link. A link that is already there
is replaced. A regular file named `vexdom` stops the operation ("… is another vexdom"). **Unlink**
and Uninstall remove only symlinks that point at this bundle's `vexdom`.

---

## Notifications and the menu bar

The app posts notifications through `notify-rust` on GPUI's background executor. Posting on the
main thread would run macOS's nested run loop for up to two seconds inside a GPUI update. It
notifies for:

- each new pair request: "‹browser› wants to connect to Vexdom";
- each new confirmation: the question, with the summary as body;
- each new notice. `NoticeBook` drops ids it has already shown.

An `announced` set keeps a request or confirmation from being announced twice. Before the first
notification, `tray::set_notification_sender` sets the sender to the bundle identifier. Unbundled,
it uses `com.apple.Finder`, because left unset `notify-rust` runs an AppleScript lookup that
opens a "Where is use_default?" chooser. Clicking a notification does nothing; a notice
picked from the menu's **Notices** sends `openNotice`.

The menu-bar item is a `tray-icon` template image (`brand/MenuBarTemplate@2x.png`, so macOS tints
it) and the app's main surface. Its menu (`tray::entries`) holds the state line and facts, a
submenu per pair request and per confirmation, Start/Stop and Restart, **Model**, **Pairing** and
**Notices** submenus, **Appearance** and the app's switches as check items, **Settings…**
(`NSApplication.unhide`, activate, raise the window), the update item, **About Vexdom** and
**Quit Vexdom**. Each item's `MenuId` is its `Pick` encoded as `verb` or `verb:rest`
(`allow:<request>`, `confirm-always:<id>`, `model:` for the default model, …); `Pick::parse` reads
it back, so a click needs no lookup table.

The window opens hidden (`show: false`) and is shown only on the first launch (no `desktop.json`
yet), while a required first-run check fails, or when there is no menu-bar item.
`on_window_should_close` hides the app and returns `false`, so the window is never destroyed.

**Dock.** GPUI makes the app a regular one (`NSApplicationActivationPolicyRegular`) as it finishes
launching; right after, `AppView::launch` calls `tray::set_dock_visible(showInDock)`, which switches
to `Accessory` (menu bar only) unless `showInDock` is set or the menu-bar item failed. Toggling the
setting switches the policy at once, and re-activates the window if it was in front. When the app
shows in the Dock unbundled, it sets its Dock icon from `brand/AppIcon-1024.png`. Both images come
from `brand/build.py`.

---

## `desktop.json`

`~/.vexdom/desktop.json`, or `$VEXDOM_HOME/desktop.json` when `VEXDOM_HOME` is set (see
[State on disk](state.md)):

| Key | Default | Meaning |
| --- | --- | --- |
| `appearance` | `"system"` | `system`, `light` or `dark` |
| `startBridgeOnLaunch` | `true` | Run `vexdom start` at launch when no Bridge answers |
| `showInDock` | `false` | Show the app in the Dock and the app switcher; off keeps it in the menu bar only |
| `finishUpdateOnLaunch` | `false` | Set just before an update swaps the bundle; the next launch runs the setup that finishes the update, then clears it |

A missing or wrongly typed key keeps its default. `settings::edit` rereads the file, changes one key
and writes the whole file to a `0600` temporary file that it renames over the original. The window
and an update in progress each write only their own key, and a crash never leaves a half-written
file.

---

## The bundle

[`scripts/bundle-mac.sh`](../../scripts/bundle-mac.sh) `<vexdom-bin> <desktop-bin> <extension-dir>
<version> <out-dir>` lays out:

```
Vexdom.app/Contents/Info.plist                  CFBundleExecutable vexdom-desktop, CFBundleIdentifier com.vexdom.desktop,
                                                CFBundleShortVersionString <version>, LSMinimumSystemVersion 14.0
Vexdom.app/Contents/MacOS/vexdom-desktop
Vexdom.app/Contents/MacOS/vexdom
Vexdom.app/Contents/Resources/AppIcon.icns      macOS before 26
Vexdom.app/Contents/Resources/Assets.car        macOS 26 and later
Vexdom.app/Contents/Resources/extension/chrome-mv3/
```

The `vexdom` next to `vexdom-desktop` is the binary `bridge.rs` runs. `packaged_extension()` in
[`app/src/cli/install.rs`](../../app/src/cli/install.rs) finds the extension at
`../Resources/extension/chrome-mv3` relative to the binary (source `"app"`), and `vexdom setup`
copies it from there to `~/vexdom/extension/chrome-mv3`.

The bundle is signed ad hoc (`codesign --force --deep --sign -`, then
`codesign --verify --deep --strict`). There is no Developer ID and no notarization, so a downloaded
copy needs `xattr -dr com.apple.quarantine`. `install.sh` and the updater both run that command.

[`release.yml`](../../.github/workflows/release.yml) runs on `macos-14` for `aarch64-apple-darwin`
and `x86_64-apple-darwin`. For each target it:

1. builds `vexdom` and `cargo build --release --locked --manifest-path desktop/Cargo.toml --target <target>`;
2. bundles them with the staged extension;
3. archives the app as `Vexdom-<version>-<target>.zip` (`ditto -c -k --keepParent`);
4. makes `Vexdom-<version>-<target>.dmg` (`hdiutil create … -format UDZO`, from a folder holding the
   app and an `Applications` symlink);
5. writes a `.sha256` next to each file.

The publish job then lists every asset in `SHA256SUMS` and signs that file with minisign as
`SHA256SUMS.minisig`, checked against `crates/vexdom-update/assets/minisign.pub`.
[`scripts/install.sh`](../../scripts/install.sh) `install_app` checks the zip's `.sha256`, unpacks
it into a temporary folder in `/Applications` (or `~/Applications`), keeps the old bundle until the
move succeeds, and clears the quarantine flag.

---

## The update flow

All of it is in `bridge.rs` and [`crates/vexdom-update`](../../crates/vexdom-update/src/lib.rs):

1. **Check**, at launch and every 6 hours (`UPDATE_EVERY`), or on **Check for updates**:
   `vexdom_update::latest()` (GitHub's `releases/latest` API), then `newer(release, VERSION)`, where
   `VERSION` is the crate version. The release is kept for the install step. An `updating` flag
   stops a check from overwriting the card during an install.
2. **Where.** `detect_layout(current_exe)` must return `Layout::App { bundle }` (the executable is
   in `<x>.app/Contents/MacOS/`). Anything else answers `UPDATE_UNMANAGED`.
3. **Download and verify.** `fetch_verified` downloads `SHA256SUMS`, `SHA256SUMS.minisig` and
   `Vexdom-<version>-<target>.zip` (`app_asset`, `target_triple()`) into
   `$TMPDIR/vexdom-update-<version>`. It checks the signature against the embedded `PUBLIC_KEY`
   (`SIGNATURE_INVALID`), then the zip's SHA-256 against its line in the signed list
   (`CHECKSUM_MISMATCH`).
4. **Stage.** `stage_app` runs `ditto -x -k` into `.Vexdom-<version>.unpack` next to the running
   bundle. `check_bundle` reads `Info.plist` with `PlistBuddy` and refuses anything other than
   `com.vexdom.desktop` at the offered version (`UPDATE_ARCHIVE_INVALID`). The bundle is renamed to
   `.Vexdom-<version>.app` and its quarantine flag is cleared.
5. **Swap.** The thread sets `finishUpdateOnLaunch`, then calls `swap_after_exit(pid, staged,
   bundle)`, which starts a detached `/bin/sh` running `relaunch_script()`, and asks the window to
   quit. The script waits up to 30 seconds (150 × 0.2 s) for the process to exit; if it does not,
   the script deletes the staged copy and exits. Otherwise it moves the old bundle to
   `<bundle>.previous`, moves the new one in, moves the old one back if that move fails, and runs
   `open -n`. If the script cannot be started, the flag is cleared again.
6. **Finish.** The new app sees `finishUpdateOnLaunch` and runs
   `vexdom setup --no-pair --restart --no-open --no-wait --json` with its own `vexdom`, which
   restarts the Bridge and rewrites the unpacked extension folder. Then it clears the flag.

`vexdom update` uses the same crate and, run from inside a bundle, the same zip and swap
([Maintenance](../guide/maintenance.md#updating)).

---

## Build and test

```sh
cd desktop
cargo test --locked                                  # model, preflight, settings, log_tail, shell, text_input, tray
cargo clippy --locked --all-targets -- -D warnings
cargo fmt --check
cargo run                                            # unbundled
```

CI runs those three checks in the `macos` job of `.github/workflows/ci.yml`, after `brew install
capnp`. An unbundled `cargo run` has no bundle identifier, so:

- **Open at login** is hidden (in the window and the menu);
- notifications come from Finder;
- the Dock icon, when **Show in Dock** is on, is set by code;
- the updater answers `UPDATE_UNMANAGED`;
- `vexdom` is taken from the login `PATH`, because there is none next to the executable.

To make a bundle locally:

```sh
cargo build --release -p vexdom
cargo build --release --manifest-path desktop/Cargo.toml
scripts/bundle-mac.sh target/release/vexdom desktop/target/release/vexdom-desktop \
  packages/extension/dist/chrome-mv3 0.9.2 dist/mac
```

The icons (`brand/AppIcon.icns`, `brand/Assets.car`, the menu-bar template) are built by
`brand/build.py` and checked in.

---

## Next

- [Overview](overview.md): where the app sits among the processes
- [Transport](transport.md): the control socket, the lockfile and Cap'n Proto
- [Guardrails](guardrails.md): what puts an action in the confirmation list
- [State on disk](state.md): `desktop.json` beside the rest of `~/.vexdom`
