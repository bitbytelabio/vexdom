# Install

One script installs Vexdom Bridge (the `vexdom` command) and, on macOS, the Vexdom app. Then
`vexdom setup` adds the extension to your browser and pairs the two. There is no account, no API
key and no cloud service of Vexdom's own.

Vexdom has two halves:

- **Vexdom Bridge**, one Rust binary, `vexdom`. It is the background process the extension talks to
  (the Bridge), the command line, the MCP server (`vexdom mcp`) and the browser's native-messaging
  host, all in one file.
- **The extension**, which hosts the side panel and acts on your tabs. Chromium browsers load it
  unpacked from a folder `setup` writes; Firefox installs a copy signed by Mozilla. There is no
  store listing.

---

## What you need

| | Requirement | Check |
| --- | --- | --- |
| **System** | macOS on Apple silicon or Intel, or Linux with glibc on x64 or arm64. Windows is not supported | `uname -sm` |
| **Browser** | Chrome, Edge, Brave, Arc, Vivaldi, Opera, Chromium, or Firefox 140 or newer | |
| **Agent** (side panel only) | [Claude Code](https://claude.com/claude-code) on your `PATH`, signed in | `claude --version` |

The side panel runs each instruction through Claude Code, so install it (`npm i -g
@anthropic-ai/claude-code`) and sign in with `claude` before your first run; see
[The side-panel agent](agents.md). Driving the browser only from an [MCP client](mcp-clients.md)
needs no agent: that path starts nothing.

---

## 1. Install the Bridge

```sh
curl -fsSL https://github.com/bitbytelabio/vexdom/releases/latest/download/install.sh | sh
```

The script downloads `vexdom-<version>-<target>.tar.gz` from the latest
[release](https://github.com/bitbytelabio/vexdom/releases), checks it against its `.sha256`, and
unpacks it into `~/.local/share/vexdom`:

```
~/.local/share/vexdom/bin/vexdom                    the Bridge, the CLI and the MCP server
~/.local/share/vexdom/extension/chrome-mv3/         the extension build, for setup to copy
```

On macOS it also installs `Vexdom.app` (into `/Applications` when it can write there, else
`~/Applications`), replacing an older copy and clearing the download quarantine. See
[The Mac app](mac-app.md).

If the script says so, add the folder to your `PATH`:

```sh
export PATH="$HOME/.local/share/vexdom/bin:$PATH"
```

| Variable | Effect |
| --- | --- |
| `VEXDOM_VERSION=0.9.2` | Install that release instead of the latest |
| `VEXDOM_INSTALL_DIR=<dir>` | Install somewhere other than `~/.local/share/vexdom` |
| `VEXDOM_ARCHIVE=<file>.tar.gz` | Install an archive you already downloaded (offline; installs no `Vexdom.app`) |
| `VEXDOM_NO_APP=1` | On macOS, skip `Vexdom.app` |

**With Homebrew** (macOS), instead of the script:

```sh
brew install --cask bitbytelabio/tap/vexdom
```

The cask installs `Vexdom.app` into `/Applications`, clears its quarantine flag, and links the
`vexdom` inside the app onto your `PATH`, so there is no `~/.local/share/vexdom`. Vexdom updates
itself (`vexdom update`, or the app), so `brew upgrade` skips it unless you pass `--greedy`.
`brew uninstall --cask vexdom` removes both.

## 2. Run setup

```sh
vexdom setup
```

`setup` starts the Bridge, registers the native-messaging host with every browser it finds (so a
browser can start the Bridge when it is down), checks the agent, and asks which browser gets the
extension:

```
  Vexdom 0.10.0 — your real browser, driven by agents

  ✓ Vexdom Bridge  running on 127.0.0.1:8785 (pid 4242)
  ✓ Wake-up            Chrome, Firefox can start the Bridge when it is down
  ✓ Agent              Claude Code

  Which browser should get the extension?

     1  Chrome    unpacked
     2  Firefox   signed add-on
     3  Another browser: load it unpacked

  › 1
```

`--browser chrome` (or `edge`, `brave`, `arc`, `vivaldi`, `opera`, `chromium`, `firefox`) skips
the question, `--unpacked` writes the folder for any browser, `--no-open` prints links instead of
opening them, `--no-wait` returns without waiting for the browser, and `--no-pair` issues no code.
With no terminal to ask on, `setup` prints a plan for every browser instead. `--json` prints the
same as JSON, which is what the Mac app reads.

### Chrome, Edge, Brave, Arc, Vivaldi, Opera, Chromium

`setup` copies the extension to `~/vexdom/extension/chrome-mv3` and prints the steps:

1. Open the browser's extensions page (`chrome://extensions`, `edge://extensions`,
   `brave://extensions`…) and turn on **Developer mode**.
2. Press **Load unpacked** and choose `~/vexdom/extension/chrome-mv3`. In the macOS folder picker,
   press ⇧⌘G and paste the path; on Linux, Ctrl+L.
3. Click Vexdom in the toolbar (the puzzle-piece menu lists it; pin it there).

Keep the folder where it is: the browser derives the extension's ID from its path, and the pairing
belongs to that ID. `extensionDir` in [config](configuration.md) moves it, after which you load
the new folder again.

`page.evaluate` (`vexdom eval`, and the agent's `evaluate` action) runs through Chrome's user
scripts, so it needs **Allow User Scripts** turned on in Vexdom's **Details** on the extensions
page.

### Firefox

`setup --browser firefox` opens `vexdom-<version>-firefox.xpi`, the add-on Mozilla signed for the
same release. Firefox asks whether to let github.com install software, then whether to add Vexdom;
say yes to both. Or download the `.xpi` from the release and use **Install Add-on From File…** in
`about:addons`. If `setup` says the file is not attached yet, Mozilla is still signing that
version; it appears within minutes.

The features built on Chrome's debugger (network routes and HAR, dialogs, emulation, init scripts,
tab keep, trusted input and the mouse, PDF, video, traces and profiles, vitals, React, accessibility
audits, diagnostics capture, captchas) are not available in Firefox and answer `UNSUPPORTED`; see
[Limits](limits.md#the-debugger-features-are-chromium-only).

## 3. Pair

Usually nothing to do: the extension pairs with the Bridge by itself through the native-messaging
host, and `setup` waits until it sees the browser connect:

```
  ✓ Chrome is connected: Vexdom 0.9.2 from the unpacked.
    Open the side panel from the toolbar and tell it what to do.
```

If the extension asks for a code instead, enter the one `setup` printed (it works once and expires
in ten minutes; `vexdom pair` prints another). [Pair](pair.md) covers the modes and what pairing
protects.

## 4. Check

```sh
vexdom doctor
```

`doctor` checks the install, the Bridge, the native host, the connected extension, the agent, your
config files and file permissions, and says how to fix each warning; `vexdom doctor --fix` repairs
what it safely can. See [Maintenance](maintenance.md#doctor).

---

## From source

For working on Vexdom itself, or running an unreleased commit. You need Rust (the version
`rust-toolchain.toml` pins, with the `wasm32-unknown-unknown` target), Bun 1.4.5, the Cap'n Proto
compiler (`capnp`), binaryen's `wasm-opt` and `cargo install boltffi_cli --version 0.32.1 --locked`;
[internals/contributing.md](../internals/contributing.md) has the full list.

```sh
git clone https://github.com/bitbytelabio/vexdom.git
cd vexdom/packages/extension
bun run setup
```

That installs the extension's dependencies, builds the extension (compiling ext-core to wasm first)
and builds the Bridge:

```
packages/extension/dist/chrome-mv3     the unpacked extension
target/release/vexdom                  the Bridge, the CLI and the MCP server
```

`bun run bridge:install` puts `vexdom` on your `PATH` (`cargo install`, into `~/.cargo/bin`). Load
`dist/chrome-mv3` with **Load unpacked**, or run `vexdom setup --unpacked`. A browser never reloads
an unpacked extension after a rebuild: press ↻ on its card.

---

## Next

**[Your first run →](first-run.md)**: a tour of the side panel and a first instruction to try.
