---
name: emulation
description: Make the tab pretend — another viewport, a phone or tablet, a location, offline, extra headers or HTTP basic credentials, print media, dark mode or reduced motion — then put it all back.
---

# Emulation

`act {"do":"emulate", ...}` applies each section you give and leaves the rest as it is. It runs only
in a Chromium browser and shows "Vexdom is debugging this browser" until you reset. This is the
user's own tab: **always finish with `act {"do":"emulate","reset":true}`**, which puts everything
back and removes the bar.

## Sections

- `viewport: {"width":400,"height":800,"deviceScaleFactor":2,"mobile":true}` — test a breakpoint.
- `device: "iPhone 15"` — viewport, pixel ratio, touch and user agent of a preset: iPhone 12, 14,
  15, 16, 16 Pro, 17, iPad, iPad Pro, Pixel 5, 7, 9, Galaxy S21, S25 (case and spaces ignored;
  `UNKNOWN_DEVICE` lists them).
- `geolocation: {"latitude":48.8566,"longitude":2.3522}` — the page asks for a position and gets
  this one (the site may still need the user's permission).
- `offline: true` — every request fails as if the network were gone; `false` restores it.
- `headers: {"x-feature-flag":"beta"}` — extra headers on every request of the tab.
- `credentials: {"username":"…","password":"…"}` — HTTP basic auth (a staging site's password
  prompt). Merged with `headers`; the password is never echoed back.
- `media: {"type":"print"}` — see the print stylesheet; `{"colorScheme":"dark"}` —
  `prefers-color-scheme`; `{"reducedMotion":"reduce"}` — `prefers-reduced-motion`.

The result echoes what was applied. Most sections take effect at once; headers, credentials and a
user agent change apply to the next request, so reload after setting them.

## Typical checks

1. Responsive: `{"do":"emulate","device":"Pixel 9"}`, `navigate {"action":"reload"}`,
   `snapshot {"as":"image"}`, compare with the desktop capture.
2. Dark mode: `{"do":"emulate","media":{"colorScheme":"dark"}}`, then look; the `page-theming`
   playbook measures contrast.
3. Offline handling: `{"do":"emulate","offline":true}`, act, read the page's error state, then
   `{"do":"emulate","offline":false}`.

Reset when done, even after a failure.

## From a shell

`vexdom set viewport <w> <h> [--scale n] [--mobile]`, `vexdom set device "iPhone 15"`,
`vexdom set geo <lat> <lon> [--accuracy n]`, `vexdom set offline [on|off]`,
`vexdom set headers '{"x-flag":"beta"}'`, `vexdom set credentials <user> <pass>`,
`vexdom set media [screen|print] [--color-scheme dark] [--reduced-motion reduce]`,
`vexdom set reset`.
