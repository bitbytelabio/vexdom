---
name: drag
description: Drag one thing onto another — reorder a list, move a card between columns, pull a slider, draw on a canvas — and confirm it landed.
---

# Drag and drop

`act {"do":"dragElement","from":"e12","to":"e20"}` picks up `from` and drops it on `to`. Use
`fromPoint` / `toPoint` (`{"x":…,"y":…}`, viewport pixels) for a spot with no element of its own:
a canvas, a point along a slider track.

## Before you drag

- Both ends must be on screen at the same moment: nothing scrolls mid-drag. Scroll the container
  so both show (`act {"do":"scrollTo","target":"e20"}`), or make the drag in two shorter hops.
- Snapshot first so you know what the list or board looks like before.

## Modes

The web has two unrelated drag mechanisms; `mode:"auto"` (default) picks one by reading the grabbed
element.

- `"pointer"` presses, moves and releases a pointer: dnd-kit, react-beautiful-dnd, sortable
  lists, sliders, canvases.
- `"native"` fires HTML5 `dragstart`/`dragover`/`drop` with a DataTransfer: elements with
  `draggable="true"`, file-drop-style targets.

If nothing moved, the result's `accepted` says whether the page took the drop; try the other mode.

## When it lands a slot out

The drop point is measured before the drag starts, so a list that reflows as you pass over it can
land one place off. Raise `steps` (default 16) and `settleMs`, hold longer before moving
(`holdMs`, for libraries with an activation delay), then read the order back.

## When the page ignores it

`trusted:true` drags with real browser-level pointer events (Chromium only; pointer mode only;
shows the "Vexdom is debugging this browser" bar). For full control — a drawn path, a hold, a
pause over a target — use the raw mouse from the `input` playbook: `mouse down`, several
`mouse move`, `mouse up`.

## Check

Always read the result back: the delta, a `query` of the new order, or `snapshot {"as":"image"}`
for a canvas. "The drag ran" is not "the card moved".

## From a shell

`vexdom drag @e12 @e20`; for a hand-made path `vexdom mouse move <x> <y>`, `vexdom mouse down`,
`vexdom mouse move <x2> <y2> --steps 20`, `vexdom mouse up`.
