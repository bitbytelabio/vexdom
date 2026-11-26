# Captchas

How Vexdom handles a captcha: which widgets it detects, which it answers itself, and which it
hands back to you.

https://github.com/user-attachments/assets/7818f665-be35-4fa3-84cc-32100880b7ee

---

## The short version

It **ticks the checkbox** with a real browser-level click, and it **answers image challenges**:
each round, a short-lived Claude Code session looks at the grid, answers, and is closed. Only the
puzzles it cannot read (Arkose, AWS WAF, drag-to-fit) come back to you.

It finds the widget wherever the page put it: inside closed shadow roots, cross-origin iframes, and
frames nested within frames.

Answering a captcha is [gated for approval](../approvals.md) by default (the `captcha-solve` rule),
because it acts on another site's security control. One approval covers every round of that captcha
in the same run.

**Chromium only.** Both captcha actions use the browser's debugger, so they run in Chrome, Edge and
other Chromium browsers. Firefox leaves them out of the actions it offers, and a call that reaches
its extension anyway answers `UNSUPPORTED`. While a solve runs, Chrome shows its "is debugging this
browser" bar.

---

## How the agent knows there is one

`getPageInfo` carries a `captcha` field whenever the page holds a captcha that needs answering:
the vendor, whether it is already `solved`, and the next step. It is read without the debugger,
from every frame the browser knows of, so it costs nothing on pages without a captcha and is absent
there. Invisible scoring captchas (reCAPTCHA v3 and the like) are left out, because there is nothing
to answer.

When the agent sees that field it calls `solveCaptcha` straight away, instead of hunting for a
checkbox no selector can reach.

---

## The two actions

There is no CLI verb for these; an MCP client calls them through `act`, and the side-panel agent the
same way.

**`findCaptcha`** (`act {"do":"findCaptcha"}`) reports what is there without touching it: the
vendor, how many frames deep it sits, its on-screen bounds, whether it is satisfied and, while a
challenge is open, its prompt and grid.

Recognised: Cloudflare Turnstile (including the full-page interstitial), reCAPTCHA v2 and v3,
hCaptcha, GeeTest, Arkose FunCaptcha, AWS WAF Captcha.

**`solveCaptcha`** (`act {"do":"solveCaptcha"}`) answers it:

1. Finds the widget and waits out one that is still loading (Turnstile's "Verifying…").
2. Scrolls it into view if it is below the fold, and ticks the checkbox.
3. If the vendor responds with an image challenge, answers it round by round until the widget is
   satisfied.

| State | Meaning |
| --- | --- |
| `solved` | The widget accepted it. Carry on |
| `challenge` | An image challenge is still open. The result carries it as an image for the caller to answer |
| `pending` | The widget took the click but had not settled inside the wait |
| `needsHuman` | A puzzle Vexdom does not answer. Its bounds come back so it can be shown to you |
| `invisible` | A scoring captcha with nothing to click |

The whole solve has a budget of two minutes unless the call sets `timeoutMs`.

---

## Image challenges

The extension photographs the challenge and hands it to the Bridge, which starts a one-shot Claude
Code session (the captcha analyst) to look at that single picture. The session answers and is stopped
at once; nothing carries over to the next round. Back in the page, the answer is clicked with real
pointer movement, the challenge's own button is pressed, and whatever comes next is photographed.
This happens for every caller: the side panel, an MCP client and `vexdom mcp call`.

Two kinds are answered:

- **Tile grids**: reCAPTCHA's 3×3 and 4×4, including the "click verify once there are none left"
  grids that replace each picked tile with a new picture. Each tile's number is painted on the
  photograph, which is taken only once every tile has finished fading in.
- **Pictures to tap**: hCaptcha's canvas challenges ("click the animal icon that is different").
  The answer is a set of points on the photograph.

When the analyst gives up (ten rounds, or the time budget runs low) or cannot answer a round, the
result comes back as `state: "challenge"` with the picture attached, and the caller answers it itself
by calling `solveCaptcha` again with `tiles`, `points` or `reload`. A call that already carries
`tiles` or `reload` skips the analyst.

A vendor can keep setting new challenges to a browser it distrusts, especially an automated or
headless one, however well each round is answered. That is the vendor's verdict on the browser, not
a wrong answer.

---

## If it is not a captcha

`CAPTCHA_NOT_FOUND` means there is no known widget on the page, in any frame, so whatever is
blocking the run is something else: a rate limit, a login wall, a slow request.

---

## See also

- [Approvals](../approvals.md): the `captcha-solve` rule, and **Always on ‹host›** for a site you use daily
- [Page actions](page-actions.md): `trustedClick`, the same real-click mechanism (also Chromium only), for pages that reject synthetic clicks
- `guide {"topic":"solveCaptcha"}` over MCP, or `vexdom tools`, for the exact input
