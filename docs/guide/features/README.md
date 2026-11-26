# Features

One page per Vexdom capability: what it does, how you reach it (the side panel, the `vexdom` CLI,
or an MCP client through `act`), and where it stops.

If you have not set Vexdom up yet, start with [Install](../install.md), [Pair](../pair.md) and
[First run](../first-run.md).

---

## Talking to it

| | |
| --- | --- |
| [Conversations](conversations.md) | Typing and dictating, conversations across your tabs, the sessions strip, history |
| [New tab](new-tab.md) | Vexdom Home on every new tab: a composer, your frequent sites and recent conversations |
| [Hands-free](hands-free.md) | Talking to the page through a mic you can drag anywhere, with the side panel put away |
| [Instant commands](instant-commands.md) | Commands such as "go back" and "scroll to the top" that run without an agent |
| [A-Eye](a-eye.md) | Pointing at an element instead of describing it |
| [Action cues](action-cues.md) | A ring on each element the agent is about to click, type into or read |
| [Profile](profile.md) | Your details and standing rules, saved once, so forms get real values |
| [Blocked sites](blocked-sites.md) | Sites no agent or MCP client can read or act on |
| [Skills](skills.md) | The bundled playbooks, how a side-panel instruction is routed to one, skills over MCP, and your own |

## Acting on a page

| | |
| --- | --- |
| [Page actions](page-actions.md) | The 84 page actions and 7 Bridge actions, grouped as `guide` lists them |
| [Screenshots](screenshots.md) | Viewport, full page or one element, annotated refs, and when a file is written |
| [Files](files.md) | Uploading a file to a page, and keeping a file a page downloads |
| [Theming](theming.md) | Dark mode on a site that has none, and a WCAG contrast audit |
| [Captchas](captcha.md) | What it will and will not do at a "verify you are human" block |
| [Diagnostics](diagnostics.md) | Console errors, exceptions and failed requests, to find why a page broke |
| [Network](network.md) | Mocking or blocking requests, recording a HAR, reading one request |
| [Emulation](emulation.md) | Another viewport or device, a location, offline, extra headers, print media, dark mode |
| [Capture](capture.md) | Saving the page as a PDF, and recording a video of the tab |
| [DevTools](devtools.md) | Traces and CPU profiles, Core Web Vitals, React internals, an axe-core accessibility audit |
| [WebMCP](webmcp.md) | Calling the tools a site publishes to agents itself |
| [Saved flows](flows.md) | Recording a task once and rerunning it as direct requests |
| [Auth vault](auth-vault.md) | Logging in with a saved account without the agent seeing the password |
| [Plugins](plugins.md) | Extending the Bridge with plugins |

## Doing things over time

| | |
| --- | --- |
| [Monitoring](monitoring.md) | Watching an upload, a build or a deploy in the background |
| [Scheduling](scheduling.md) | Tasks that run on their own, and the timers an agent sets for itself |
| [Site maps](site-maps.md) | Teaching it a site once, so later sessions already know the layout |
| [Recordings](recordings.md) | Doing a repetitive job once yourself, then asking for it "like last time" |

---

## Chromium only

Some features drive Chrome's debugger (the Chrome DevTools Protocol). They work in Chrome, Edge,
Brave, Arc, Vivaldi, Opera and Chromium, and not in Firefox, which offers extensions no debugger:

- [Diagnostics](diagnostics.md), [Network](network.md), [Emulation](emulation.md),
  [Capture](capture.md) and [DevTools](devtools.md), entirely
- [Captchas](captcha.md), dialogs, init scripts, the trusted mouse and `trustedClick`,
  `tabKeep`, and live page code (`injectCode` / `runCode`) on [Page actions](page-actions.md)

The Firefox build leaves these 26 actions out of its list (58 of the 84 remain), and a call that
reaches it anyway fails with `UNSUPPORTED`. While any of them holds the debugger on a tab, Chrome
shows its "Vexdom is debugging this browser" bar. [WebMCP](webmcp.md) does not use the debugger and
works in both.

---

Three pages apply to every feature:

- **[Approvals](../approvals.md)**: what the agent must ask you before doing
- **[Limits](../limits.md)**: where each feature stops
- **[reference/errors.md](../../reference/errors.md)**: every error code

For one action's exact input, ask the MCP server (`guide {"topic":"<action>"}`) or print the
whole manifest with `vexdom tools`.
