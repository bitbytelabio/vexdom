---
version: 1
slug: "desktop-src-ui-rs"
primary_target: "desktop/src/ui.rs"
related_targets: ["desktop/src/main.rs", "desktop/src/tray.rs"]
---

# Desktop app (vexdom-desktop)

Scope: the macOS app `desktop/`: its menu-bar item (the main surface) and the GPUI Settings window
(General, Browsers, Agent, Skills, Activity, Logs, About). Mode: Operate. Glanced at from the menu
bar, CleanShot style; the job is knowing in a second whether vexdom is on and a browser is
connected, then allowing a browser, answering an approval or changing the model without opening a
window. Must not feel like a generic admin dashboard or carry decoration a utility does not need.
Logo work in scope: the macOS app icon, the mark in the app, the extension icons and a menu-bar icon.

## Direction contract

THESIS: vexdom is an appliance: one lamp and one line answer "on? connected?" before anything else, from the menu bar. It refuses the admin-dashboard grid of equal status cards, and a window nobody needs open.

OWN-WORLD: warm grey enamel ground #e8e6e1, raised off-white faces #f6f5f2 lifted by a soft offset shadow (no border), graphite #2f2f2d text, warm label grey #8a8882. Orange #e8641b lives in the icons' switch alone; the lamp is green #2fa84f when connected, amber #d98a1a while waiting, unlit grey when off. One grotesk (Archivo, bundled) at two weights; port, pid and versions in tabular figures.

STORY: click the menu-bar mark, read the state line and its facts, act once (Start or Stop, Allow or Deny, a model, a pairing mode), done. Open Settings… only to change how things work.

FIRST VIEWPORT: the native menu: the state line and its facts, what waits (a submenu per browser and per action), Start/Stop and Restart, Model / Pairing / Notices submenus, preferences as checked items, Settings…, updates, About, Quit. The window, when opened: native tabs centred in the title strip, General first with the Bridge's face (lamp and caption, the state line at 17 px, facts; Start/Stop on the right); a pairing request or approvals pinned above every tab; the last CLI message in a status strip.

FORM: menu-bar app (no Dock icon unless "Show in Dock") with a native macOS settings window, 760 x 600, standard controls; seed: seed:dec65a465606466b.

SIGNATURE: the switch-and-lamp menu-bar mark over a native menu whose first line is the state. The same switch-and-lamp is the app icon and the extension icon.
