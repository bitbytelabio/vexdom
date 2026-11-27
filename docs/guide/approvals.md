# Approvals and guardrails

Which actions wait for your approval, which are refused outright, how you answer from the side
panel, the Vexdom app, a terminal or an MCP client, and how to change the policy.

In short: reading and clicking need no approval, anything that commits something or sends data
somewhere asks, and a handful of things are refused whatever you say.

---

## Answering

### In the side panel

When a side-panel run reaches a gated action, it stops and shows a card naming what it was about to
do and why that is gated:

| | |
| --- | --- |
| **Allow** | Once. |
| **Deny** | Final. The agent is told to report it and stop, and not to find another route to the same effect. |
| **Always on ‹host›** | Allow, and stop asking for *that action on that host*. |

A card the agent stops waiting for is withdrawn: an answer after that does nothing, so an approval
never runs an action the agent is no longer waiting on.

### From the CLI or an MCP client

The same gate applies to `vexdom` page commands and to MCP clients (ADR-032). The call waits up to
**120 seconds** for an answer, from any of:

- **the Vexdom app**: the request appears in its menu-bar menu, with a notification, as
  *Allow ‹action› on ‹site›?* with **Allow**, **Always Allow on ‹site›** and **Deny** (also pinned
  at the top of its window);
- **a terminal**:

  ```sh
  vexdom requests                    # c-1a2b3c4d  page.submitForm on example.com — Submits a form
  vexdom approve c-1a2b3c4d          # allow it once
  vexdom approve c-1a2b3c4d --always # allow it, and stop asking for that action on that site
  vexdom deny c-1a2b3c4d             # refuse it
  vexdom approve --watch             # ask about each one as it comes
  ```

  A page command that is waiting says so once on stderr: `Waiting for approval c-1a2b3c4d: approve
  it in Vexdom, or run vexdom approve c-1a2b3c4d (vexdom deny c-1a2b3c4d to refuse).` Approving
  needs stdin to be a terminal, so an agent that simply runs `vexdom approve` is refused. That is
  not a boundary: any process running as your user can give itself a terminal (an agent with a
  shell can wrap the command in `script`) and approve. The Vexdom app and the MCP client's own
  prompt are the safer places to answer;
- **the MCP client itself**, when it supports elicitation (Claude Code does): *Allow once*, *Always
  allow on this site*, *Deny*. See [MCP clients](mcp-clients.md#actions-that-need-approval-wait-for-you).

A refusal answers `DECLINED`; no answer in time answers `APPROVAL_TIMEOUT`. Both tell the agent not
to retry or work around it.

What you allow is the action on the site the prompt named. If the tab moved to another site while
you were being asked, the approved action is not run and the call answers `PAGE_CHANGED`; the
agent may ask again.

### Standing approvals

**Always on ‹host›** writes a grant, one action on one host, to `~/.vexdom/approvals.json` (mode
`0600`). It survives restarts and **only ever short-circuits a confirm**: a deny stays denied.

```sh
vexdom approvals              # what no longer asks
vexdom approvals clear        # forget all of them
vexdom approvals clear a.com  # forget one site's
```

---

## The rules

The policy is a table of rules, each naming a condition and an effect. Every rule whose condition
matches is collected and **the most severe effect wins**, so declaration order does not matter.
Actions are named as in the manifest (`page.submitForm`); agents call them by short name
(`act {"do":"submitForm"}`).

| Rule id | Fires when | Default |
| --- | --- | --- |
| `reserved-action` | An internal `vexdom.*` action is called from outside | **deny** |
| `non-http-navigation` | A `javascript:`, `data:` or `file:` URL dressed up as a navigation | **deny** |
| `unreadable-navigation` | A URL that resolves to no destination Vexdom can check | **deny** |
| `raw-html-read` | `extractText` with `format: "html"` | **deny** |
| `network-body-read` | `readNetwork` with `includeBodies: true` | **deny** |
| `off-scope-navigation` | A side-panel run navigating off the sites it is about | confirm |
| `url-payload` | A navigation whose query string and fragment exceed `urlPayloadBytes` (512) | confirm |
| `form-submission` | Anything that commits a form, however spelled | confirm |
| `site-tool-call` | `callSiteTool`: running a tool the site registered through [WebMCP](features/webmcp.md) | confirm |
| `file-upload` | `attachFile`: putting one of your files into a page | confirm |
| `file-download` | `captureDownload`: letting a page write a file to your disk | confirm |
| `leaves-pinned-tab` | Moving to a tab the run was not pointed at | confirm |
| `captcha-solve` | `solveCaptcha`: ticking a site's "I am a human" box and answering its challenge | confirm |
| `code-injection` | `injectCode`: installing JavaScript the agent wrote into the page | confirm |
| `external-code-injection` | `injectCode` from the CLI or an MCP client | **deny** |
| `external-code-execution` | `runCode` from the CLI or an MCP client | **deny** |
| `secret-in-url` | A sealed secret placed in a navigation URL | **deny** |
| `secret-release` | A sealed secret about to be typed into the page | confirm |
| `secret-off-scope` | …and it was read on a site outside the run's scope | confirm |
| `config-require-approval` | The action is named in your `requireApproval` list | confirm |

**`form-submission` matches more than `submitForm`.** It also catches `fillInput` and `typeText`
with `pressEnter: true`, and `pressKey` with `Enter`.

**`file-download` only decides whether the capture happens.** Executables, files over 100 MB and
downloads from a host outside the run's scope are refused whatever this rule says. See
[Files](features/files.md).

**`code-injection` is the one prompt that shows the code.** The panel's **Live tool** switch starts
off, and while it is off `injectCode` is refused before this rule is consulted. The prompt has a
**Review** button with the full source and no "always" option. From the CLI or an MCP client page
code is denied: nobody is there to read it. `page.evaluate` (`vexdom eval`) has no gate of its own
(ADR-028); list it in `requireApproval` if you want one.

**`raw-html-read` is denied by default** because `outerHTML` carries comments, `aria-hidden` nodes
and off-screen text: what a page can hide from you but still hand to the model.

**`network-body-read` is denied by default** because response bodies carry session tokens and
personal data in bulk. Method, URL, status and timing need no approval, and headers come back
sanitized. See [Network](features/network.md).

### Scope: which sites a side-panel run may reach

A run's scope is derived once, when it starts, from the host of the tab it started on, hosts you
named in your instruction, and `guardrails.hosts`. Nothing read from a page widens it.
`example.com` covers its subdomains; `["*"]` turns confinement off. A run that starts on a blank
tab with no host named is unconfined. CLI and MCP callers are unconfined.

For a site no agent or client may touch at all, use [Blocked sites](features/blocked-sites.md): the
extension enforces that list itself, before any of these rules, and `vexdom read <url>` checks it
too (`SITE_BLOCKED`).

A page read over HTTP (`vexdom read <url>`, `snapshot {"as":"markdown","url":…}`) is checked like a
navigation to that URL, and again at each redirect hop: the blocked-sites list, scope and
`url-payload` apply. The MCP `snapshot` tool also refuses addresses on your computer or local
network (`READ_REFUSED`) unless `vexdom mcp` was started with `VEXDOM_READ_PRIVATE=1`.

---

## Waiving the wait for CLI and MCP callers

```json
{ "guardrails": { "unattended": "allow" } }
```

turns every confirm for a CLI or MCP caller into allow, so nothing waits and nothing asks. It never
loosens a deny. Your MCP client's own permission prompts stop protecting you the moment someone
allowlists the vexdom tools to stop being asked, so think before setting it.

---

## Changing the policy

In `~/.vexdom/config.json`:

```json
{ "requireApproval": ["page.submitForm", "page.closeTab"] }
```

```json
{ "guardrails": { "rules": { "captcha-solve": "allow", "off-scope-navigation": "deny" } } }
```

`requireApproval: []` gates no form. A prompt you see on every other call is a prompt you stop
reading, so keep the list short.

The same rows are on the extension's settings page (**Guardrails**) and in the Mac app. Each row
is an override of the shipped default: turning it on writes one line, turning it off removes the
line. Four rows are shown but locked: `reserved-action`, `non-http-navigation`,
`unreadable-navigation` and `secret-in-url`. A run takes its policy when it starts. A connection
that has acted for a side-panel run cannot change settings (`BLOCKED`).

---

## Secrets are sealed before you are asked

Before any rule runs, credentials are taken out of what the browser hands back. A password, key,
token, cookie value or card number becomes a placeholder naming its kind and site:

```
Your new password is ⟦password:7f3a@mail.example.com⟧
```

The agent never sees the value. It becomes plaintext again only inside the browser, when the
placeholder is the `value` of `fillInput`, the `text` of `typeText` or `insertText`, or a cookie
value being set, which is when `secret-release` asks you first. A `⟦…⟧` in a result is a credential
being handled, not an error. [How it works](../internals/guardrails.md).

---

## What this does not cover

The policy governs what an agent may do **to a page**. What the spawned Claude Code may do **to your
machine** is a separate mechanism; see [internals/guardrails.md](../internals/guardrails.md). No gate
makes an agent immune to [prompt injection](limits.md#prompt-injection-is-a-real-risk).

---

## See also

- [Configuration](configuration.md): the keys
- [MCP clients](mcp-clients.md): approvals from a client
- [Limits](limits.md)
