# Limits

Known limitations: what Vexdom needs to run, where its security boundary sits, which features
depend on Chrome's debugger, and the ceilings compiled into it.

---

## It needs a real browser, open

There is no headless mode and no browser launched for you. Vexdom drives the browser you are
looking at, in your real profile with your real logins. Close it and every page action returns
`EXTENSION_OFFLINE`. To read a static page without a tab, `vexdom read <url>` (MCP:
`snapshot {"as":"markdown","url":…}`) fetches it over HTTP; the browser must still be connected,
because each URL is first checked against its [Blocked sites](features/blocked-sites.md) list.
Over MCP, addresses on your computer or local network are refused (`READ_REFUSED`) unless
`vexdom mcp` was started with `VEXDOM_READ_PRIVATE=1`; `vexdom read <url>` may read them.

## macOS and Linux only

Releases cover macOS (arm64, x64) and Linux with glibc (x64, arm64). Windows is not supported.

## Pages that refuse content scripts

`chrome://` pages, the Chrome Web Store and the new-tab page cannot host a content script, so most
actions return `TAB_UNREACHABLE` there. `navigate` still works and is the way out.

## The debugger features are Chromium only

These are built on the Chrome DevTools Protocol through `chrome.debugger`, which Firefox does not
offer to extensions. In Firefox the CLI verbs answer `UNSUPPORTED`, and an MCP client's `guide`
does not list them (so `act` answers `UNKNOWN_ACTION`); in Chromium `guide` marks their groups
`(Chromium only)`:

- network routes and HAR recording ([Network](features/network.md)), console and request capture
  ([Diagnostics](features/diagnostics.md));
- dialogs, emulation ([Emulation](features/emulation.md)), init scripts, `tabKeep`;
- trusted input: `trustedClick`, `mouse`, `pressKey`/`typeText` with `trusted: true`;
- PDF and video ([Capture](features/capture.md)); traces and profiles, vitals, React and
  accessibility audits ([DevTools](features/devtools.md));
- captcha solving, `injectCode`/`runCode`.

In Chrome they share one debugger connection per tab, so they work together on the same tab, but:
Chrome shows **"Vexdom is debugging this browser"** while any of them holds the tab; attaching
fails with `DEBUGGER_UNAVAILABLE` while DevTools is open on that tab; and if the debugger leaves (you
dismiss the bar, the tab closes) the next call answers `DEBUGGER_DETACHED` and routes, HAR,
emulation and dialog handling on that tab start over.

Console and request capture see only what happens while attached: a failure before the first
`vexdom console` / `startDiagnostics` is not recorded.

## Several browsers, one run at a time per tab

Several browsers can be paired and connected at once. The CLI and MCP clients reach one browser at a
time, the one you were last in. In the side panel one instruction runs at a time per tab session
(`RUN_IN_PROGRESS`); eight tab sessions may be open and three may run at once (`maxConcurrentRuns`,
ceiling 8).

## Pairing controls which browser, not which process

Pairing binds a browser. It does not authenticate local programs: anything running as your user can
read `~/.vexdom/daemon.json` and drive an already-paired browser. **Vexdom assumes your user account
is the trust boundary.** Someone at your keyboard who loads a copy of the extension claiming its ID
is paired by itself in `auto` mode; `vexdom pairing ask` shows you each new browser first.

## Prompt injection is a real risk

An agent reading a hostile page can follow instructions embedded in it. No prompt makes a model
immune. Vexdom's mitigations, none a guarantee:

- page text is [fenced](../internals/guardrails.md) in random markers and marked as data;
- credentials are sealed, so the agent cannot read, repeat or send them;
- a side-panel run is [scoped](approvals.md#scope-which-sites-a-side-panel-run-may-reach) to the
  sites it is about, and consequential actions wait for you.

Site-published [WebMCP](features/webmcp.md) tool names and descriptions are page text too.

## Approvals from the CLI and MCP wait

An action that needs approval blocks the CLI command or MCP call for up to 120 seconds while it
waits for you (`APPROVAL_TIMEOUT` after). An unattended script either answers from another terminal
or sets `guardrails.unattended: "allow"`, after which nothing asks. Page code (`injectCode`,
`runCode`) is never available outside the side panel.

`vexdom approve` refuses when stdin is not a terminal, which stops an agent that simply runs it.
It does not stop a process running as your user that gives itself a terminal (an agent with a
shell can use `script`), so it is not a security boundary. Answer in the Vexdom app or in your MCP
client's own prompt when you can.

## The side panel runs Claude Code only

Other agent CLIs are not supported in the side panel. Any MCP client can drive the browser through
`vexdom mcp`.

## Speech goes to Google

Voice input uses Chrome's built-in Web Speech API, which streams audio to Google. Type instead if
that is not acceptable. [Hands-free](features/hands-free.md) listens the same way while it is on.

## Recording and mapping limits

**Recordings** run for at most 15 minutes, follow one tab, live in extension storage, and drop
passwords, hidden fields, one-time codes and card numbers. **Site mapping** is read-only, one host
and one tab, capped at 15 pages / 10 screenshots / 10 minutes by default (ceilings 40 / 24 / 30
minutes).

## Screenshots of very tall pages

Full-page capture is capped at 48 tiles and a 16 384 px canvas side; past that the result says
`truncated: true`.

## Artifacts

PDFs, HAR files and traces travel from the browser in one message capped at 48 MiB
(`ARTIFACT_TOO_LARGE`). A HAR keeps at most 5,000 entries and 32 MiB of bodies. Video recording
needs `ffmpeg` on the Bridge's `PATH`.

## Themes do not survive a reload

`applyTheme` changes the live document; a reload puts it back. See [Theming](features/theming.md).

## Loopback ports

The Bridge binds the first free port of 8785, 8786, 8787 (`VEXDOM_PORTS` changes the list). If all
are taken it does not start, and the extension only probes the default three.

## Unpacked extension

Chromium browsers load the extension unpacked: no store updates it, the browser does not reload it
after an update (press ↻), and Chrome may mention developer-mode extensions on launch.

---

## See also

- [Approvals](approvals.md): what is gated and why
- [Troubleshooting](troubleshooting.md)
- [reference/errors.md](../reference/errors.md): error codes
