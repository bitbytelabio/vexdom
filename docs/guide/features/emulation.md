# Emulation

Make one tab pretend: another viewport or device, a location, being offline, extra request headers,
HTTP basic credentials, print media, a colour scheme or reduced motion. One action, `emulate`.
Chromium only.

```sh
vexdom set device "iPhone 15"
vexdom set media --color-scheme dark
vexdom set reset
```

---

## CLI and MCP

| CLI | `act {"do":"emulate", …}` |
| --- | --- |
| `vexdom set viewport <w> <h> [--scale n] [--mobile]` | `"viewport":{"width":390,"height":844,"deviceScaleFactor":3,"mobile":true}` |
| `vexdom set device <name>` | `"device":"Pixel 9"` |
| `vexdom set geo <lat> <lon> [--accuracy n]` | `"geolocation":{"latitude":…,"longitude":…,"accuracy":1}` |
| `vexdom set offline [on\|off]` | `"offline":true` |
| `vexdom set headers <json\|@file>` | `"headers":{"X-Test":"1"}` |
| `vexdom set credentials <user> <pass>` | `"credentials":{"username":"…","password":"…"}` |
| `vexdom set media [screen\|print] [--color-scheme light\|dark\|no-preference] [--reduced-motion reduce\|no-preference]` | `"media":{"type":"print","colorScheme":"dark","reducedMotion":"reduce"}` |
| `vexdom set reset` | `"reset":true` |

Over MCP several sections can go in one call; each one given is applied. A call with none of them
fails with `INVALID_INPUT`. The answer echoes what was applied (credentials without the password).

Headers and credentials share one header list: setting headers keeps the credentials, and the other
way round. Credentials go out as an `Authorization: Basic …` header on every request of the tab.

A mobile viewport or device also turns on touch emulation.

---

## Devices

`device` takes one of these names, case and spaces ignored (`iphone15` works; `iPad Air` means
`iPad`). Each sets the viewport, scale factor, mobile flag and user agent.

| Device | Viewport | Scale |
| --- | --- | --- |
| iPhone 12 | 390 × 844 | 3 |
| iPhone 14 | 390 × 844 | 3 |
| iPhone 15 | 393 × 852 | 3 |
| iPhone 16 | 393 × 852 | 3 |
| iPhone 16 Pro | 402 × 874 | 3 |
| iPhone 17 | 402 × 874 | 3 |
| iPad | 820 × 1180 | 2 |
| iPad Pro | 1024 × 1366 | 2 |
| Pixel 5 | 393 × 851 | 2.75 |
| Pixel 7 | 412 × 915 | 2.625 |
| Pixel 9 | 412 × 923 | 2.625 |
| Galaxy S21 | 360 × 800 | 3 |
| Galaxy S25 | 360 × 800 | 3 |

Any other name fails with `UNKNOWN_DEVICE`, and the message lists the supported ones.

---

## How long it lasts

Emulation holds the tab's `emulation` debugger lease until `vexdom set reset` (`"reset": true`),
which clears every override, restores the user agent and detaches. Chrome shows "Vexdom is
debugging this browser" the whole time.

If the debugger goes away first (DevTools opened, the bar's **Cancel**, the tab closed), the
emulation is gone with it. The next `reset` on that tab reports `DEBUGGER_DETACHED`; the next
`emulate` starts afresh.

| Code | When |
| --- | --- |
| `UNSUPPORTED` | Firefox |
| `DEBUGGER_UNAVAILABLE` | DevTools is open on the tab, or Chrome refused to attach |
| `DEBUGGER_DETACHED` | The emulation was lost with the debugger |
| `UNKNOWN_DEVICE` | A device name not in the list |

---

## Background tabs: tabKeep

A related action controls how a tab behind another one runs. It is in the `navigation` group and
also Chromium only.

| CLI | MCP |
| --- | --- |
| `vexdom tab keep live\|throttled\|frozen [tab]` | `act {"do":"tabKeep","mode":"live","tabId":12}` |

`live` emulates focus so the tab keeps running timers and animations as if in front. `frozen`
pauses it; only a tab hidden behind another can be frozen (`TAB_VISIBLE` otherwise), and any other
action on it thaws it first. `throttled` undoes both and releases the debugger.

---

## The playbook

The `emulation` playbook (`crates/vexdom-core/assets/skills/emulation/SKILL.md`) is served as the MCP
prompt `emulation`, the resource `skill://emulation/SKILL.md`, and through
`guide {"topic":"emulation"}`.

---

## See also

- [Network](network.md): mocking the responses themselves
- [Screenshots](screenshots.md): capturing what the emulated page looks like
- [Theming](theming.md): retheming a page instead of switching its colour scheme
