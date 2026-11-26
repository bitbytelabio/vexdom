---
name: input
description: Reach the inputs ordinary clicks and fills cannot — insert text without key events, send trusted keys, and drive the real mouse by coordinates for canvases, maps and hover-only widgets.
---

# Text, keys and the mouse

Start with the ordinary actions — `fillInput`, `typeText`, `pressKey`, `clickElement`. Reach for
these when the page ignores them.

## Insert text

`act {"do":"insertText","text":"Hello","target":"e7"}` inserts at the caret as a paste or an input
method would: no key events. Use it for rich editors (Slack, Notion, Google Docs, code editors) that
mishandle synthetic key presses, or to add to a field without clearing it. Without `target` it
inserts into the focused element; place the caret first with
`act {"do":"focusInput","target":"e7","caret":"end"}`.

## Trusted keys (Chromium only)

Pages can tell a script's key event from a person's (`isTrusted`). `trusted:true` sends the key
through the browser itself:

- `act {"do":"pressKey","key":"Enter","trusted":true}`; a chord is the key plus `modifiers`
  (`{"key":"a","modifiers":["ctrl"],"trusted":true}` selects all); `phase:"down"` and `"up"` hold
  and release a key across calls.
- `act {"do":"typeText","target":"e7","text":"hello","trusted":true}` types each character as a
  real key press; characters with no key are inserted.

Use it for shortcuts the page handles itself, games, and editors that drop untrusted keys. With a
`target`, the element is focused first.

## The mouse (Chromium only)

`act {"do":"mouse", ...}` drives the real pointer at viewport coordinates (CSS pixels of the top
document). Every event is trusted.

- `{"do":"mouse","op":"move","x":300,"y":200}`; `human:true` travels a curved, uneven path
  (`seed` makes it repeatable, `steps`, `durationMs`).
- `{"do":"mouse","op":"down"}` and `{"op":"up"}` press and release (`button`: `left`, `middle`,
  `right`). A held button stays down between calls, so down → move → move → up is a drag, a
  selection or a drawn stroke.
- `{"do":"mouse","op":"wheel","deltaY":400}` turns the wheel (`deltaX` for sideways) — what maps,
  zoomable canvases and scroll-jacking pages listen for.

Get coordinates from `act {"do":"query","target":"e5","what":"box"}` (aim at the centre) or from
`snapshot {"as":"image"}` — image pixels are not CSS pixels on a high-density screen, so prefer the
box. Each mouse action answers with the snapshot delta it caused.

`act {"do":"trustedClick","target":"e5"}` is the one-call version for a single click that must be
real: it travels to the element, dwells and clicks.

## From a shell

`vexdom keyboard inserttext <text>`, `vexdom press Control+a --trusted`,
`vexdom keydown Shift` / `keyup Shift`, `vexdom keyboard type <text> --trusted`,
`vexdom mouse move <x> <y> [--human] [--seed n] [--steps n] [--duration ms]`,
`vexdom mouse down [button]`, `vexdom mouse up [button]`, `vexdom mouse wheel <dy> [dx]`.
