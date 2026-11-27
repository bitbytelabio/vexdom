---
name: Vexdom desktop
description: The appliance face for the Bridge: a menu-bar lamp first, a Settings window of light enamel or dark islands.
colors:
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
    fontSize: "17px"
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

Vexdom is a household appliance for your browser. Its face is the menu-bar item, the switch and
lamp as a template image, whose native menu says whether the Bridge is on and a browser is
connected before anything else is read, and holds the everyday actions (Start/Stop, answering a
browser or an approval, the model, pairing, notices, appearance). The window is Settings, opened
from that menu: a few raised faces on a ground, each a work area (the Bridge and its facts, this
app, browsers, the agent). It is not an admin dashboard: no grid of equal status cards, no charts,
no decoration a utility does not need.

Two lights follow macOS's appearance, live. Light is warm grey enamel with off-white faces lifted
by a soft shadow. Dark takes JetBrains Islands' layout: rounded islands on a near-black canvas,
separated by gaps and lit by a hairline edge instead of a shadow.

**Key Characteristics:**
- One lamp, one line: the state is readable in a second, in the menu bar and atop General.
- The menu bar first; the window only for settings and what needs room (naming a browser, lists).
- Faces (islands) hold work areas; nothing is nested inside a face but rows.
- Standard controls: tabs, buttons, a text field, a native menu.

## Colors

Warm neutrals carry everything; the three lamp colours are the only hues in the window, and each
means one thing. Orange lives in the icons alone (`../brand/build.py`).

### State
- **Lamp Green** (#2fa84f; dark #3fbf62): a browser is connected; Claude Code is ready.
- **Lamp Amber** (#d98a1a; dark #e39a2e): the Bridge runs and waits for a browser; a browser
  asks to connect.
- **Lamp Unlit** (#c6c3bc; dark #4a4c51): off, unknown, not paired.
- **Fault Rust** (#b3361b; dark #f0785a): error text only.

### Neutral
- **Enamel** (#e8e6e1) / **Island Canvas** (#131417): the window's ground, and recessed parts in
  the light (fields, secondary buttons, the tab track).
- **Enamel Face** (#f6f5f2) / **Island** (#1e1f22): a raised face.
- **Island Edge** (#2b2d30): the dark islands' hairline, and their recessed parts.
- **Graphite** (#2f2f2d) / **Island Ink** (#dfe1e5): text and the primary button.
- **Enamel Label** (#5e5c56) / **Island Label** (#a9acb2): labels and secondary text; 4.8:1 or
  better on every surface they sit on, the tab track included.

### Named Rules
**The No Orange Rule.** Orange is the icons' switch. Selection, focus, links and buttons in the
window never use it; text selection is graphite (light) or ink (dark) at low alpha.

**The Lamp Means State Rule.** Green, amber and unlit are the only state colours and always mean
connected, waiting, off.

## Typography

**Display, body and label font:** Archivo (bundled, `fonts/`, SIL OFL), with the system sans as
the fallback if loading fails.

**Character:** a grotesk with a slightly wide, industrial cut, like the lettering on an appliance
panel; two weights do the work.

### Hierarchy
- **Headline** (700, 17px): the one state line on the Bridge's face, under its lamp.
- **Title** (500, 15px): a face's heading.
- **Row title** (500, 14px): a browser, a paired session, a notice.
- **Body** (400, 13px): facts, secondary text, button labels (500).
- **Label** (500, 12px): tabs, the status strip.
- **Figures** (400, 13px, `tnum`): the address, versions; numbers line up.

## Layout

A 760 x 600 window (minimum 680 x 520) with a transparent title bar: the traffic lights sit at the
left of a 40px title strip, the seven tabs centred in it (⌘1–⌘7: General, Browsers, Agent,
Skills, Activity, Logs, About). The window opens hidden and only from the menu bar (or on the very
first launch). The body scrolls under the strip with 22px side margins; faces stack with 14px
gaps. A request face when a browser asks, then the actions waiting for approval, are pinned above
every tab. The status strip at the bottom shows the last command's result for 8 seconds (45
seconds for the steps after the unpacked folder is copied). General: the Bridge's face (lamp +
caption, headline, facts left; Restart and Start/Stop right), this app, browser theme, guardrails,
command line, uninstall. Browsers: pairing (mode chips, a pairing code with its `mm:ss` countdown,
Unpair all), the browsers on this Mac, the paired ones. Agent: one face, with Install Claude Code
or Fix while it is not ready. About: versions with Copy, the update card, help links.

While a required first-run check fails (the `vexdom` command, the browsers' wake-up), a first-run
face of check rows stands in for the tabs, the title strip saying only "Vexdom"; it gives way by
itself once they pass, or on Continue when opened from About.

The app never starts the Bridge on its own unless "Start Vexdom when the app opens" is on (General
or the menu; `~/.vexdom/desktop.json`, on by default); Appearance there can pin light or dark.
"Show in Dock" (off by default) is the only thing that puts the app in the Dock.

## Elevation & Depth

Light: faces are lifted by a soft two-layer shadow and nothing else. Dark: islands have no shadow;
a 1px edge in #2b2d30 separates them from the canvas.

### Shadow Vocabulary
- **Lift** (`0 1px 2px rgba(0,0,0,.08), 0 6px 16px -4px rgba(0,0,0,.06)`): light faces, the
  selected tab.

## Shapes

Faces round at 14px, controls at 7px, tab chips at 5px. Circles are kept for the lamps (10px
beside the caption, 8px in rows, 6px in a badge).

## Components

### The Bridge's face (General)
The lamp and caption, the headline, and while running the facts (Address in figures); on the
right, Restart (secondary, while running) and Start or Stop (primary). While a Start, Stop or
Restart is under way they read "Starting…", "Stopping…", "Restarting…", dim to 50% and ignore
clicks; until the state is known there are no buttons.

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
A recessed track with seven chips; the selected chip is a raised face. What waits on the user is
pinned above every tab, so no tab needs a dot.

### Chips, switches, ticks, badges
- **Chips:** the tab track reused for one-of-a-few choices (appearance, pairing mode, a
  guardrail's Allow / Ask / Block, skill sources).
- **Switch:** 38 x 22, ink with the knob right when on, recessed when off.
- **Tick box:** 16px, 4px corners; ink filled with a ✓ when ticked ("Keep my skills").
- **Badge:** a recessed 5px tag with a 6px amber lamp: something to do in the browser
  ("Reload needed").

### Log
Logs: the Bridge log's last 400 lines in Menlo 11/16 on a recessed 360px well, read every second
while shown; it keeps to its end unless scrolled up.

### Menu-bar item (signature)
A template image (switch + lamp) that macOS tints, over a native menu in CleanShot's manner: the
state line (or who asks to connect, or how many actions wait) and its facts; a submenu per browser
asking and per action waiting; Start/Stop and Restart; Model, Pairing and Notices submenus with a
check on the current choice; Appearance and the app's switches as checked items; Settings…, the
update item, About, Quit. Menu titles use macOS title case ("Show in Dock", "Get a Pairing Code…").
Closing the window hides it; the app keeps running here.

## Do's and Don'ts

### Do:
- **Do** answer "on? connected?" with the lamp and the headline before anything else.
- **Do** put a status claim beside its proof (the address, the agent's version).
- **Do** keep every control a standard one: tabs, buttons, a field, the menu bar.
- **Do** use tabular figures for numbers.

### Don't:
- **Don't** use orange anywhere in the window.
- **Don't** add equal status cards, metrics, charts or sparklines; this is a utility.
- **Don't** put a shadow under a dark island; its edge does the work.
- **Don't** claim a state the window does not know: show "Looking for the Bridge" until the first
  answer.
