---
name: Vexdom desktop
description: The appliance face for the Bridge: one switch, one lamp, light enamel or dark islands.
colors:
  power-orange: "#e8641b"
  power-orange-hover: "#f0742d"
  power-orange-pressed: "#d65a12"
  power-orange-latched: "#c96a33"
  lamp-green: "#2fa84f"
  lamp-amber: "#d98a1a"
  lamp-unlit: "#c6c3bc"
  fault-rust: "#b3361b"
  enamel: "#e8e6e1"
  enamel-face: "#f6f5f2"
  enamel-well-hover: "#dedbd5"
  graphite: "#2f2f2d"
  graphite-hover: "#484845"
  enamel-label: "#5e5c56"
  island-canvas: "#131417"
  island: "#1e1f22"
  island-edge: "#2b2d30"
  island-well-hover: "#393b40"
  island-ink: "#dfe1e5"
  island-ink-hover: "#c9ccd2"
  island-label: "#a9acb2"
  island-lamp-green: "#3fbf62"
  island-lamp-amber: "#e39a2e"
  island-lamp-unlit: "#4a4c51"
  island-fault: "#f0785a"
typography:
  headline:
    fontFamily: "Archivo"
    fontSize: "28px"
    fontWeight: 700
    lineHeight: "34px"
  switch-label:
    fontFamily: "Archivo"
    fontSize: "19px"
    fontWeight: 700
  title:
    fontFamily: "Archivo"
    fontSize: "15px"
    fontWeight: 500
  row-title:
    fontFamily: "Archivo"
    fontSize: "14px"
    fontWeight: 500
  body:
    fontFamily: "Archivo"
    fontSize: "13px"
    fontWeight: 400
  label:
    fontFamily: "Archivo"
    fontSize: "12px"
    fontWeight: 500
  figures:
    fontFamily: "Archivo"
    fontSize: "13px"
    fontWeight: 400
    fontFeature: "tnum"
rounded:
  tab: "5px"
  control: "7px"
  face: "14px"
  round: "9999px"
spacing:
  hairline: "2px"
  tight: "6px"
  row: "10px"
  gap: "14px"
  face-x: "20px"
  window-x: "22px"
  face-hero: "24px"
components:
  button-primary:
    backgroundColor: "{colors.graphite}"
    textColor: "{colors.enamel-face}"
    rounded: "{rounded.control}"
    padding: "7px 14px"
  button-primary-hover:
    backgroundColor: "{colors.graphite-hover}"
  button-secondary:
    backgroundColor: "{colors.enamel}"
    textColor: "{colors.graphite}"
    rounded: "{rounded.control}"
    padding: "7px 14px"
  button-secondary-hover:
    backgroundColor: "{colors.enamel-well-hover}"
  face:
    backgroundColor: "{colors.enamel-face}"
    rounded: "{rounded.face}"
    padding: "24px"
  field:
    backgroundColor: "{colors.enamel}"
    textColor: "{colors.graphite}"
    rounded: "{rounded.control}"
    padding: "7px 10px"
    width: "176px"
  power-switch:
    backgroundColor: "{colors.power-orange}"
    textColor: "{colors.graphite}"
    rounded: "{rounded.round}"
    size: "112px"
  power-switch-latched:
    backgroundColor: "{colors.power-orange-latched}"
    size: "104px"
  tab-selected:
    backgroundColor: "{colors.enamel-face}"
    textColor: "{colors.graphite}"
    rounded: "{rounded.tab}"
    padding: "3px 14px"
---

# Design System: Vexdom desktop

The source of truth is `src/theme.rs` (`Palette::light`, `Palette::dark`); the direction contract
is `.impeccable/surfaces/desktop-src-ui-rs.md`. Light-mode keys above are the components' values;
the `island-*` keys are the same roles in the dark palette.

## Overview

**Creative North Star: "The Appliance Face"**

Vexdom is a household appliance for your browser: a round orange push switch and an indicator
lamp say whether the Bridge is on and a browser is connected before anything else is read. The
window is a glance utility, left in the background: a few raised faces on a ground, each a work
area (the switch and its facts, a browser asking to connect, notices, browsers, the agent). It is
not an admin dashboard: no grid of equal status cards, no charts, no decoration a utility does
not need.

Two lights follow macOS's appearance, live. Light is warm grey enamel with off-white faces lifted
by a soft shadow. Dark takes JetBrains Islands' layout: rounded islands on a near-black canvas,
separated by gaps and lit by a hairline edge instead of a shadow.

**Key Characteristics:**
- One switch, one lamp, one line: the state is readable in a second.
- Orange belongs to the power switch alone.
- Faces (islands) hold work areas; nothing is nested inside a face but rows.
- Standard controls: tabs, buttons, a text field, a menu-bar item.

## Colors

Warm neutrals carry everything; orange and the three lamp colours are the only hues, and each
means one thing.

### Primary
- **Power Orange** (#e8641b): the power switch, raised (off). Hover #f0742d, pressed #d65a12.
- **Latched Orange** (#c96a33): the switch latched in while the Bridge runs; duller, so on and
  off differ at a glance.

### State
- **Lamp Green** (#2fa84f; dark #3fbf62): a browser is connected; Claude Code is ready.
- **Lamp Amber** (#d98a1a; dark #e39a2e): the Bridge runs and waits for a browser; a browser
  asks to connect (the dot on the Overview tab).
- **Lamp Unlit** (#c6c3bc; dark #4a4c51): off, unknown, not paired.
- **Fault Rust** (#b3361b; dark #f0785a): error text only.

### Neutral
- **Enamel** (#e8e6e1) / **Island Canvas** (#131417): the window's ground, and recessed parts in
  the light (fields, the switch's well, secondary buttons).
- **Enamel Face** (#f6f5f2) / **Island** (#1e1f22): a raised face.
- **Island Edge** (#2b2d30): the dark islands' hairline, and their recessed parts.
- **Graphite** (#2f2f2d) / **Island Ink** (#dfe1e5): text and the primary button.
- **Enamel Label** (#5e5c56) / **Island Label** (#a9acb2): labels and secondary text; 4.8:1 or
  better on every surface they sit on, the tab track included.

### Named Rules
**The One Orange Rule.** Orange is the power switch. Selection, focus, links and buttons never
use it; text selection is graphite (light) or ink (dark) at low alpha.

**The Lamp Means State Rule.** Green, amber and unlit are the only state colours and always mean
connected, waiting, off. The switch's well is ringed in the lamp's colour while the Bridge runs.

## Typography

**Display, body and label font:** Archivo (bundled, `fonts/`, SIL OFL), with the system sans as
the fallback if loading fails.

**Character:** a grotesk with a slightly wide, industrial cut, like the lettering on an appliance
panel; two weights do the work.

### Hierarchy
- **Headline** (700, 28/34px): the one state line beside the switch.
- **Switch label** (700, 19px): the switch's action, "Start" or "Stop" (large text, 3.4:1 or
  better on every orange).
- **Title** (500, 15px): a face's heading.
- **Row title** (500, 14px): a browser, a paired session, a notice.
- **Body** (400, 13px): facts, secondary text, button labels (500).
- **Label** (500, 12px): tabs, the status strip.
- **Figures** (400, 13px, `tnum`): the address, versions; numbers line up.

## Layout

A 760 x 600 window (minimum 680 x 520) with a transparent title bar: the traffic lights sit at the
left of a 40px title strip, the five tabs centred in it (⌘1–⌘5: Overview, Browsers, Agent,
Settings, About). The body scrolls under the strip with 22px side margins; faces stack with 14px
gaps. The status strip at the bottom shows the last command's result for 8 seconds (45 seconds for
the steps after the unpacked folder is copied). Overview: the state face (switch left, lamp +
headline + facts + Restart right), then a request face when a browser asks, then the actions
waiting for approval, then notices. Browsers: pairing (mode chips, a pairing code with its `mm:ss`
countdown, Unpair all), the browsers on this Mac, the paired ones. Agent: one face, with Install
Claude Code or Fix while it is not ready. Settings: a side list (General, Skills, Activity, Logs)
beside its page. About: versions with Copy, the update card, help links.

While a required first-run check fails (the `vexdom` command, the browsers' wake-up), a first-run
face of check rows stands in for the tabs, the title strip saying only "Vexdom"; it gives way by
itself once they pass, or on Continue when opened from About.

The app never starts the Bridge on its own unless Settings → General → "Start Vexdom when the app
opens" is on (`~/.vexdom/desktop.json`, on by default); Appearance there can pin light or dark.

## Elevation & Depth

Light: faces are lifted by a soft two-layer shadow and nothing else. Dark: islands have no shadow;
a 1px edge in #2b2d30 separates them from the canvas. The switch is the one physical object: raised
with a soft drop shadow while off, latched in (smaller, a 1px shadow) while on.

### Shadow Vocabulary
- **Lift** (`0 1px 2px rgba(0,0,0,.08), 0 6px 16px -4px rgba(0,0,0,.06)`): light faces, the
  selected tab.
- **Switch raised** (`0 2px 3px rgba(0,0,0,.16), 0 8px 18px -6px rgba(0,0,0,.12)`).
- **Switch latched** (`0 1px 1px rgba(0,0,0,.18)`).

## Shapes

Faces round at 14px, controls at 7px, tab chips at 5px. Circles are kept for the switch (a 168px
well, a 112px knob that shrinks to 104px when latched) and the lamps (10px beside the headline,
8px in rows, 6px on a tab).

## Components

### Power switch (signature)
A round push switch in a recessed well. Off: bright orange, raised, says "Start". On: latched
orange, smaller, says "Stop", the well ringed in the lamp's colour. While the window does not
know the state yet, or a Start/Stop is under way ("Starting", "Stopping"), it is dimmed and ignores
clicks. The same switch and lamp are the app icon, the extension icon and the menu-bar template
(`../brand/build.py`).

### Buttons
- **Primary** (graphite on the face, 7px, 7 x 14px): the one action a face asks for (Allow, the
  chosen model, Start Vexdom). Hover lightens.
- **Secondary** (recessed into the face): Deny, Add extension, Unpair, Open, other models.
- **Busy:** a sent Allow/Deny dims to 50% and ignores clicks until the request list changes.

### Field
One line, 176px, recessed, 7px; a graphite (ink) border when focused. Typing goes through the
platform input handler, so IME composition works; the line scrolls to keep the caret in view;
40 characters at most. Enter allows, Escape denies. It takes focus when a request arrives.

### Tabs
A recessed track with five chips; the selected chip is a raised face. An amber dot on Overview
marks a browser or an action waiting there.

### Chips, switches, ticks, badges
- **Chips:** the tab track reused for one-of-a-few choices (appearance, pairing mode, a
  guardrail's Allow / Ask / Block, skill sources).
- **Switch:** 38 x 22, ink with the knob right when on, recessed when off.
- **Tick box:** 16px, 4px corners; ink filled with a ✓ when ticked ("Keep my skills").
- **Badge:** a recessed 5px tag with a 6px amber lamp: something to do in the browser
  ("Reload needed").

### Log
Settings → Logs: the Bridge log's last 400 lines in Menlo 11/16 on a recessed 360px well, read every
second while shown; it keeps to its end unless scrolled up.

### Menu-bar item
A template image (switch + lamp) that macOS tints; the menu shows the state line (or who asks to
connect, or how many actions wait), Start/Stop, Restart (while running), Open Vexdom, Quit. Closing the window hides the app; it keeps running here.

## Do's and Don'ts

### Do:
- **Do** answer "on? connected?" with the lamp and the headline before anything else.
- **Do** put a status claim beside its proof (the address, the agent's version).
- **Do** keep every control a standard one: tabs, buttons, a field, the menu bar.
- **Do** use tabular figures for numbers.

### Don't:
- **Don't** use orange anywhere but the power switch.
- **Don't** add equal status cards, metrics, charts or sparklines; this is a utility.
- **Don't** put a shadow under a dark island; its edge does the work.
- **Don't** claim a state the window does not know: show "Looking for the Bridge" until the first
  answer.
