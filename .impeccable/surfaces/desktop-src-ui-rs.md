---
version: 1
slug: "desktop-src-ui-rs"
primary_target: "desktop/src/ui.rs"
related_targets: ["desktop/src/main.rs"]
---

# Desktop app (vexdom-desktop)

Scope: the GPUI macOS window `desktop/` (Overview, Browsers, Agent). Mode: Operate. Glanced at, left
open in the background; the job is knowing in a second whether vexdom is on and a browser is
connected, then allowing a browser, unpairing one or changing the model without hunting.
Must not feel like a generic admin dashboard or carry decoration a utility does not need.
Logo work in scope: the macOS app icon, the mark in the app, the extension icons and a menu-bar icon.

## Direction contract

THESIS: vexdom is an appliance: one round switch and one lamp answer "on? connected?" before anything else. It refuses the admin-dashboard grid of equal status cards.

OWN-WORLD: warm grey enamel ground #e8e6e1, raised off-white faces #f6f5f2 lifted by a soft offset shadow (no border), graphite #2f2f2d text, warm label grey #8a8882. Orange #e8641b belongs to the power switch alone; the lamp is green #2fa84f when connected, amber #d98a1a while waiting, unlit grey when off. One grotesk (Archivo, bundled) at two weights; port, pid and versions in tabular figures.

STORY: open the window, read the lamp and its one line, act once (switch, Allow or Deny, Unpair, a model), close it.

FIRST VIEWPORT: native tabs (Overview, Browsers, Agent) centred in the title strip. Overview face: the round switch (~168 px) on the left; on the right the lamp with its label, the state line at 28 px, and a facts list (Address, Agent, Paired). Below, a pairing request face: who asks, a name field, Deny and Allow. Notices as a quiet list at the bottom; the last CLI message in a status strip.

FORM: native macOS utility window, 720 x 560, standard controls; seed: seed:dec65a465606466b.

SIGNATURE: the round power switch with its indicator lamp; the lamp's colour is the state, pressing the switch starts or stops the Bridge. The same switch-and-lamp is the app icon and the menu-bar icon.
