# Research: a content-script accessibility tree for `page.snapshot`

Topic `content-script-ax` (MAP.md research list). Serves [ADR-005](../DECISIONS.md) (snapshot with refs from a
content-script accessibility tree) and ADR-011 (snapshot pipeline early, with `page.snapshot` in the extension).
Written 2026-11-25. Nothing here is implemented; numbers marked **[bench]** were measured for this note (method in §0.2).

**Result in five lines**

1. agent-browser's `snapshot_from_ax_nodes` needs only `nodeId`, `role`, `name`, `value`, `properties`
   (level/checked/expanded/selected/disabled/required), `childIds`, `backendDOMNodeId`, `ignored`, plus three side
   maps (cursor info, link URLs, hints) — a content script can produce all of it; the Firefox BiDi path already does
   exactly this from a non-CDP tree (§1).
2. Playwright's in-page `roleUtils.ts` (Apache-2.0) matched Chrome's own AX tree on 2145 of 2147 ref-worthy nodes of a
   4,925-element page, at 13.2 ms a pass in a Chrome isolated world and ~62 ms in a Firefox Xray sandbox;
   `dom-accessibility-api` (MIT) matched 2119 of 2335, misses shadow DOM text, and throws in the Firefox sandbox
   unless handed a bound `getComputedStyle`; `aria-query` computes nothing (§2).
3. Traverse the flat tree (shadow root children, slot assigned nodes), reach closed roots through
   `chrome.dom.openOrClosedShadowRoot` (Chrome 88+, no permission) / `element.openOrClosedShadowRoot` (Firefox 63+),
   and snapshot each frame in its own content script; Chrome has **no** `runtime.getFrameId`, so frame stitching needs
   another key there (§3).
4. Refs: the content script keeps a per-document durable integer id per element (WeakMap + `Map<id, WeakRef>`) and a
   random per-instance document key; the daemon maps `(frameId, docKey, id)` to the `backendDOMNodeId` the vendored
   `RefMap` turns into `e{n}` — the same split bidi/snapshot.rs uses (§4).
5. Recommendation: vendor Playwright's role/name code (patched: inert, closed roots, `summary`), write our own walker
   that emits a compact wire tree, convert it to `AXNode` in Rust, and run the pipeline unchanged (§Recommendation).

---

## 0. Sources and method

### 0.1 Sources (versions as read on 2026-11-25)

| Key | Source |
| --- | --- |
| ACC | W3C *Accessible Name and Description Computation 1.2*, WD 2026-11-04, <https://www.w3.org/TR/accname-1.2/> (anchors cited as `ACC#id`) |
| HAAM | W3C *HTML Accessibility API Mappings 1.0*, WD 2026-10-26, <https://www.w3.org/TR/html-aam-1.0/> |
| ARIA | W3C *WAI-ARIA 1.2*, REC 2023-06-06, <https://www.w3.org/TR/wai-aria-1.2/> |
| CAAM | W3C *Core-AAM 1.2*, CRD 2026-11-05, <https://www.w3.org/TR/core-aam-1.2/> |
| CR-DOM | Chrome docs, `chrome.dom` (`browser.dom`), last updated 2026-10-29, <https://developer.chrome.com/docs/extensions/reference/api/dom> |
| CR-CS | Chrome docs, content scripts, <https://developer.chrome.com/docs/extensions/develop/concepts/content-scripts> (`#capabilities`, `#isolated_world`) |
| CR-RT | Chrome docs, `runtime` API, <https://developer.chrome.com/docs/extensions/reference/api/runtime> |
| CR-DBG | Chrome docs, `debugger` API (for its `#permissions` section, as contrast) |
| CHROMIUM | `extensions/common/api/runtime.json` and `third_party/blink/renderer/core/dom/events/event_target.cc` at `main` on chromium.googlesource.com |
| GECKO | `remote/shared/Realm.sys.mjs` @ `37ca8e54`, `toolkit/components/extensions/ExtensionContent.sys.mjs` @ `abc222b8` (github.com/mozilla-firefox/firefox) |
| MDN | mdn/content `main`: WebExtensions `dom.openOrClosedShadowRoot`, `runtime.getFrameId`, `scripting.executeScript`, *Content scripts*, *Sharing objects with page scripts* (Xray vision); Web API `Element.checkVisibility`, `HTMLElement.inert`, `HTMLDialogElement.showModal`, `Element.attachShadow`, `Window.postMessage`, `Performance.now`, `HTMLElement.click`, `WeakRef` |
| BCD | mdn/browser-compat-data `main`: `webextensions/api/{dom,runtime,scripting}.json`, `api/{Element,SVGElement}.json` |
| AB | agent-browser v0.45.0 @ `f0a9835`, `../agent-browser/cli/src/native/` (`snapshot.rs`, `element.rs`, `bidi/snapshot.rs`, `bidi/target.rs`, `cdp/types.rs`) |
| DAA | `dom-accessibility-api@0.7.1` npm tarball (`dist/*.mjs`), repo eps1lon/dom-accessibility-api |
| AQ | `aria-query@5.3.2` npm tarball, repo A11yance/aria-query |
| PW | microsoft/playwright `v1.65.1`: `packages/injected/src/{roleUtils,domUtils,ariaSnapshot}.ts`, `packages/isomorphic/cssTokenizer.ts`, `tests/library/role-utils.spec.ts`, `NOTICE` |
| VX | this repo: `packages/extension/src/lib/actions/page/{dom,get-page-info,switch-frame,click-element}.ts`, `packages/extension/src/lib/actions/client.ts`, `packages/extension/src/lib/frames/host.ts`, `packages/extension/src/lib/bridge/{invoke,socket}.ts`, `packages/extension/src/extension/entrypoints/content.ts`, `dist/*/manifest.json` |

### 0.2 Benchmark method [bench]

- **Page**: a generated fixture (`?cards=185`) with nav lists, a labelled form (email, placeholder-only, wrapped label,
  password, search, two selects, `aria-labelledby` textarea, checkbox, `display:none` radio inside a `cursor:pointer`
  label, range, submit), a 25-row table with `th scope=row`, a `role=presentation` table, hidden / `display:none` /
  `aria-hidden` / `visibility:hidden` / closed `<details>` / `inert` blocks, 185 article cards (heading link, `<p>`,
  `<img alt>`, button, icon button with `aria-label` and `::before` content, `role=button` link, `cursor:pointer`
  `onclick` div, `tabindex` span, sr-only text, unnamed `<section>`, list, card `<header>`/`<footer>`), and every 10th
  card an open-shadow `x-btn` (slotted text), an open-shadow `x-card` and a **closed**-shadow `x-closed`. 4,925
  elements counting open shadow trees (20,234 at `?cards=800`).
- **Chrome 158.0.8127.89** headless, driven over CDP from Bun; each library ran in the main world and in an isolated
  world made with `Page.createIsolatedWorld` — the Blink isolated world is what content scripts run in (CR-CS
  `#isolated_world`). Median of 5 passes.
- **Firefox 161.0** headless over WebDriver BiDi `script.evaluate`; "sandbox" = a BiDi sandbox, which Gecko builds as
  `Cu.Sandbox(win, {sandboxPrototype: win, wantXrays: true})` (GECKO `Realm.sys.mjs:221-231`), the same
  construction as a content script's `Cu.Sandbox(principal, {sandboxPrototype: contentWindow, wantXrays: true,
  isWebExtensionContentScript: true, …})` (GECKO `ExtensionContent.sys.mjs:1081-1092`) except for principal and
  flags. `performance.now()` showed 1 ms granularity on this `file://` page, so means of 15 passes are reported.
- **Oracle**: after the isolated-world pass every element got a `data-bi` index; `DOM.getDocument({pierce:true})`
  joined it to `Accessibility.getFullAXTree`. Compared set = nodes whose Chrome role or library role is ref-worthy
  (AB `INTERACTIVE_ROLES ∪ TABLE_CELL_ROLES ∪ CONTENT_ROLES`, `snapshot.rs:105-137`), Chrome `image`→`img`,
  `DisclosureTriangle`→`button`.
- **Libraries**: DAA `isInaccessible` + `getRole` + `computeAccessibleName` per element; PW `beginAriaCaches()`,
  `isElementHiddenForAria` + `getAriaRole` + `getElementAccessibleName` per element; "own" = a ~150-line prototype of
  the walker proposed in §7 (one top-down pass, inherited hidden state, style cache, simplified accname). Bundled with
  `Bun.build({minify:true})`, gzip level 9.
- Harness (ephemeral scratchpad, not in the repo): `scripts/{bench.js,chrome-driver.ts,firefox-driver2.ts,
  firefox-driver4.ts,chrome-shapes2.ts,payload.js,chrome-payload.ts}` and `bench/{bench.html,shapes.html}` under
  `<session scratchpad>/`.
- **Caveats**: the fixture is synthetic and the "own" prototype was written knowing it, so its agreement is an upper
  bound; real pages have heavier CSS and DOM. Firefox's `file`-type content process crashed twice (two crash reports,
  `JSActorMessage: MessageHandlerFrameParent:sendCommand`) when a *second* realm re-ran the whole bench after another
  realm had; every Firefox figure below comes from a fresh browser where that realm ran first. Not reproduced in a
  real content script — a P4 check.

---

## 1. What the Rust pipeline reads

### 1.1 Entry point

`snapshot_from_ax_nodes(nodes: &[AXNode], urls: &HashMap<i64,String>, cursor_elements: &HashMap<i64,
CursorElementInfo>, options, ref_map: &mut RefMap, refs: SnapshotRefContext, hints: FnOnce(Vec<(usize,i64)>) ->
Future<Vec<(usize,String)>>) -> String` (AB `snapshot.rs:863-899`). It runs `build_tree` → `promote_hidden_inputs` →
`assign_refs` → hints → urls → `render_tree` → `finish_output`. Unlike the CDP path it does **not** fetch iframes:
`take_snapshot_inner` recurses into `Iframe` nodes and splices child text (AB `snapshot.rs:613-680`), while the
BiDi caller puts a child frame's document *as the child of the frame node* before calling the shared pipeline
(AB `bidi/snapshot.rs:709-731`). Our JSON must do the same.

### 1.2 `AXNode` fields: needed vs ignored

`AXNode` derives `Deserialize` with `rename_all = "camelCase"` and no `deny_unknown_fields` (AB `cdp/types.rs:316-330`),
so extra keys are dropped silently.

| Field | Read? | How (AB) | Notes for our JSON |
| --- | --- | --- | --- |
| `nodeId` | yes | `string_or_int` (`types.rs:7`, `:319`); key of `id_to_idx` (`snapshot.rs:1595-1617`) | unique per snapshot; index is fine |
| `childIds` | yes | `opt_vec_string_or_int` (`types.rs:23`); builds `children`/`parent_idx` (`snapshot.rs:1620-1630`) | roots = nodes nobody lists as a child (`snapshot.rs:1683-1697`); an orphan becomes an extra root |
| `parentId` | **no** | not a field of `AXNode` (`parent_id` exists only on `FrameInfo`, `types.rs:221`) | omit |
| `role` | yes | `extract_ax_string` → `normalize_ax_role` (`IframePresentational`→`Iframe`, `element.rs:1160-1166`) | `AXValue.type` is a required `String` (`types.rs:334-339`) — every `AXValue` must carry `"type"` or deserialization fails |
| `name` | yes | `extract_ax_string` (string/number/bool → string, `snapshot.rs:1899-1909`) | `StaticText` carries its text here |
| `value` | yes | `extract_ax_string_opt`: non-empty string or number, bool → none (`snapshot.rs:1911-1922`) | printed as `: value` when ≠ name (`snapshot.rs:1833-1838`); BiDi drops it for links (`bidi/snapshot.rs:271`) |
| `description` | **no** | parsed, never read (no `.description` use in `snapshot.rs`) | omit |
| `properties` | yes, 6 names | `extract_properties` (`snapshot.rs:1931-1975`): `level` → `as_i64`; `checked` → string or bool; `expanded`/`selected`/`disabled`/`required` → **`as_bool` only** | a string `"true"` for `expanded` is dropped silently; `focused`, `invalid`, `focusable`, `editable`… are ignored |
| `ignored` | yes | ignored (unless `RootWebArea`) or `InlineTextBox` → `TreeNode::empty()`; children still attach (`snapshot.rs:1591-1594`) | an empty role renders transparently (`snapshot.rs:1724-1733`) — omitting presentational nodes and re-parenting their children is equivalent and smaller |
| `backendDOMNodeId` | yes | key for cursor info, URLs, hints and durable refs (`snapshot.rs:742-747`, `:788-806`, `:885-891`) | our per-document durable id, mapped to a daemon-wide `i64` (§4) |

Behaviour that constrains the emitter:

- **Text**: consecutive `StaticText` siblings are concatenated into the first; a lone `StaticText` child whose name
  equals the parent's is dropped (`snapshot.rs:1633-1679`). Emit DOM text nodes as separate `StaticText` children.
- **Rendering**: `generic` with no ref and ≤1 child is transparent, empty `StaticText` is skipped,
  `RootWebArea`/`WebArea` are skipped (`snapshot.rs:1724-1748`); any other role prints a line, so Chrome-internal
  wrappers (`MenuListPopup`, `ListMarker`) show up in agent-browser output.
- **Refs** go to `INTERACTIVE_ROLES` (ARIA widget roles plus Chrome names `DisclosureTriangle`, `ColorWell`,
  `Iframe`), table cells always, `heading/listitem/article/region/main/navigation` only when named, and any node in
  `cursor_elements` (`snapshot.rs:105-137`, `:736-753`). Duplicate role+name in one frame context gets `nth`
  (`snapshot.rs:765-786`).
- **Cursor info** is parsed from the very JSON `CURSOR_DETECTION_JS` returns — `hasCursorPointer`, `hasOnClick`,
  `hasTabIndex`, `isEditable`, `text`, `hiddenInputType`, `hiddenInputChecked`, `isNative` — by the `pub(crate)`
  `cursor_info_from_value` (`snapshot.rs:1253-1320`). `promote_hidden_inputs` turns a `LabelText`/`generic` node
  wrapping a `display:none` radio/checkbox into that role (`snapshot.rs:1552-1577`).
- **Hints** are asked only for ref'd controls whose printed label is empty or shared (`hint_targets`,
  `snapshot.rs:686-718`); `HINT_FUNCTION` is plain DOM JS (`snapshot.rs:902-919`).
- **Durable refs** need a document identity: `observe_document(page_session, frame, session, loader)` keeps the
  `backend id → ref` bucket while the loader is unchanged (`element.rs:188-219`, `:231-257`).

### 1.3 Precedent: Firefox's tree mapped into the same shape

`ax_nodes_from_snapshot`/`push_node` (AB `bidi/snapshot.rs:195-285`) turn Firefox DevTools' nested
`accessible.snapshot` into a flat `Vec<AXNode>`:

- roles renamed only where the pipeline keys on Chrome names: `document`→`RootWebArea`, `text leaf`→`StaticText`,
  `label`→`LabelText`, `internal frame`→`Iframe`, `entry`/`password text`→`textbox`, editable `search`→`searchbox`,
  `spin button`→`spinbutton`; everything else passes through (`:228-237`);
- states → boolean properties `checked`, `expanded`, `selected`, `unavailable`→`disabled`, `required`, only when the
  state is present (so never `expanded=false`), plus integer `level` (`:238-258`);
- `nodeId` = list index, `childIds` from nesting, `ignored: None`, `description: None` (`:267-283`);
- `backendDOMNodeId` = a durable integer from `(context, sharedId, documentId)` handed out by a session counter
  (`:833-866`); `node_contexts` records the frame of each id so duplicates and refs are per frame;
- cursor info from the shared `CURSOR_DETECTION_JS` run over the document and every open and closed root
  (`:621-651`), link hrefs into `urls` (`:855-860`); "Firefox has no CDP loaderId; the document root's sharedId scopes
  durable refs" (`:864-866`).

`page.snapshot` is the same move with the content script as the source.

---

## 2. Role and accessible name in the page

### 2.1 What is excluded or hidden

| Rule | Source |
| --- | --- |
| Not exposed: element and descendants with host-language "not displayed" (`display:none`, `visibility:hidden`, `hidden` attr) | ARIA `#tree_exclusion` |
| Should not be exposed: `aria-hidden="true"` and descendants (parent overrides child `aria-hidden=false`) | ARIA `#tree_exclusion`; CAAM `#ariaHiddenTrue` |
| Exception: a focused element stays exposed even under `aria-hidden` | ARIA `#tree_inclusion`; CAAM `#ariaHiddenTrueElementExposed` |
| `inert` nodes are not exposed (an inert node can have non-inert descendants, e.g. a modal dialog) | HAAM `#att-inert` |
| `showModal()` makes everything outside the dialog inert, as if `inert` were set | MDN `showModal` |
| `hidden` maps to `aria-hidden=true` only while it keeps UA `display:none` | HAAM `#att-hidden` |
| For naming, hidden = `display:none`, `visibility:hidden|collapse`, `content-visibility:hidden`, `aria-hidden`; **not** `opacity:0` and **not** off-screen | ACC `#comp_hidden_not_referenced` (note) |
| Hidden content still names a node when reached through `aria-labelledby`/`describedby` whose target is hidden | ACC `#comp_hidden_not_referenced`, `#comp_labelledby` |
| Children Presentational: descendants of `button, checkbox, img, menuitemcheckbox, menuitemradio, meter, option, progressbar, radio, scrollbar, separator, slider, switch, tab` are not exposed, their text is | ARIA `#tree_exclusion` |
| `checkVisibility()` is false without a box (`display:none`/`contents`) or under `content-visibility:hidden`; options add opacity/visibility checks | MDN `checkVisibility` |
| Closed `<details>` content and non-slotted light children are not rendered; PW detects the first via `checkVisibility()` "up the flat tree including user-agent ShadowRoots" | PW `domUtils.ts:104-116`, `roleUtils.ts:337-380` |

Chrome agrees [bench, `shapes.html`]: the closed-details link and the inert button have no AX node, the
`display:none` radio is `ignored` (`notRendered`).

### 2.2 Implicit roles that depend on context (HAAM)

| Element | Role | HAAM |
| --- | --- | --- |
| `a`/`area` with `href` / without | `link` / `generic` | `#el-a`, `#el-a-no-href` |
| `header` / `footer` scoped to `body` | `banner` / `contentinfo` | `#el-header-ancestorbody`, `#el-footer-ancestorbody` |
| `header` / `footer` scoped to `main` or sectioning content | `sectionheader` / `sectionfooter` (Chrome 158 reports these) | `#el-header`, `#el-footer`; [bench] |
| `aside` in body/main / in sectioning content | `complementary` / `complementary` only when named, else `generic` | `#el-aside-ancestorbodymain`, `#el-aside` |
| `section` | `region` when named, else `generic` | `#el-section` |
| `form` | `form` (PW and Chrome: only when named) | `#el-form`; PW `roleUtils.ts:113` |
| `img alt=""` (also `alt=" "`) | `none`/`presentation` | `#el-img-empty-alt` |
| `input type=password` / `file` | no corresponding role (Chrome: `textbox` / `button`) | `#el-input-password`, `#el-input-file` |
| `select` multiple or size>1 / otherwise | `listbox` / `combobox` | `#el-select-listbox`, `#el-select-combobox` |
| `td` in a `table` / in `grid` | `cell` / `gridcell` | `#el-td`, `#el-td-gridcell` |
| `th` | `columnheader`/`rowheader`/`cell` by header status | `#el-th*` |
| `summary` | computed role `generic` (Mac `AXDisclosureTriangle`; Chrome `DisclosureTriangle`) | `#el-summary`; [bench] |
| `label` | no corresponding role (Chrome `LabelText`) | `#el-label`; [bench] |
| `div`, `span` | `generic` | `#el-div`, `#el-span` |
| `option` | `option` with `aria-selected` from selectedness | `#el-option` |

Explicit roles: use the first token that is a valid non-abstract role; none valid → as if no role (ARIA
`#document-handling_author-errors_roles`; CAAM `#roleMappingComputedRole` "MUST ignore any abstract or invalid role
token"). Name-prohibited roles (`caption, code, deletion, emphasis, generic, insertion, paragraph, presentation,
strong, subscript, superscript`) get no name (ARIA `#namefromprohibited`; ACC `#comp_init`).

### 2.3 Presentational roles

`none`/`presentation` are dropped except when the element is focusable or otherwise interactive, or carries a global
ARIA attribute — then the implicit role wins; a required owned element (`tr`, `td`, `li`…) inherits the
presentational role of its container unless it has an explicit role (ARIA `#conflict_resolution_presentation_none`).
PW implements inheritance through `kPresentationInheritanceParents` and the focusable/global-attribute exception
(PW `roleUtils.ts:229-300`). Chrome agrees [bench]: `td` of `table role=presentation` comes out `generic`.

### 2.4 accname 1.2 in one pass

Per node, first rule that yields text wins (ACC `#computation-steps`):
`#comp_init` (name-prohibited → "") → `#comp_hidden_not_referenced` → `#comp_labelledby` (each IDREF, once; not
inside another labelledby traversal) → `#comp_embedded_control` (textbox value, chosen option, range
`aria-valuetext`/`aria-valuenow`) → `#comp_label` (`aria-label`, not on `slot`; skipped for an embedded control during
name-from-content) → `#comp_host_language_label` (HAAM §4.1: `label` elements, `alt`, `legend`, `caption`, `title`,
`placeholder`…) → `#comp_name_from_content` (roles of ARIA `#namefromcontent`, or when referenced, or inside a label:
`::marker`/`::before` + **rendered child nodes** + `::after`) → `#comp_text_node` → `#comp_recursive_name_from_content`
→ `#comp_tooltip` (`title`). "Rendered child nodes" = the shadow root's children if the node hosts one, else a slot's
assigned nodes, else the children (`#comp_name_from_content_find_child*`, added in 1.2 per its change log "Add steps for
shadow roots and slots (#167)"). Each node is consulted once. Whitespace joining around inline vs block content is
still under discussion (ACC note `#note-concatenate-inline-contents-with-space`).

HAAM per element (§4.1): text inputs → `aria-labelledby`/`aria-label` → associated `label`s (DOM order, excluding the
control's own value when nested) → `title` → `placeholder` → `aria-placeholder` (`#input-type-text-…`); submit/reset →
labels → `value` → localized "Submit"/"Reset" → `title`; `button` → labels → subtree → `title`; `img` → `alt` (even
empty) → `title` → lone `figcaption`; `fieldset` → first child `legend` → `title`; `table` → first `caption` → `title`;
`a` → subtree → `title`; `iframe` → `title` (`#iframe-element-accessible-name-computation`); `summary` → subtree →
`title`.

### 2.5 Options evaluated

| | `dom-accessibility-api` 0.7.1 | `aria-query` 5.3.2 | Playwright `roleUtils.ts` v1.65.1 | Own (prototype) |
| --- | --- | --- | --- | --- |
| License | MIT, © 2020 Sebastian Silbermann (`LICENSE.md`) | Apache-2.0 | Apache-2.0 (file header; repo `NOTICE`); `cssTokenizer.ts` CC0 (its header) | ours |
| What it is | `computeAccessibleName/Description`, `getRole`, `isInaccessible`, `isDisabled` | role/property/element-role **data tables** only — no name or tree computation | `getAriaRole`, `getElementAccessibleName`, `isElementHiddenForAria`, states (`getAriaChecked/Expanded/Level/Disabled/Selected/Pressed`), caches | walker + role table + simplified accname |
| Maintenance | latest 2025-11-27 (0.7.0 2024-07-24); repo pushed 2026-04-19; 75 open issues + PRs | latest 2024-09-20; repo pushed 2024-12-06; 13 open issues + PRs | release v1.65.1; repo pushed 2026-11-25; WPT accname manual + non-manual suites run in CI with ~16 listed skips (`tests/library/role-utils.spec.ts:36-130`) | — |
| Size (min / gzip) [bench] | 13.4 KB / 4.5 KB | 103.2 KB / 13.5 KB | 29.2 KB / 8.7 KB (with `domUtils` + `cssTokenizer`) | ~6 KB est. |
| Chrome 158 isolated world, 4,925 el. [bench] | 17.4–17.5 ms (main world 12.0–12.2) | n/a | 13.2 ms (main 11.7–11.9) | 6.1–6.2 ms (main 5.4–5.5) |
| Same at 20,234 el. [bench] | 79.7 ms | n/a | 53.9 ms | 27.9 ms |
| Firefox 161 BiDi sandbox, 4,925 el. [bench] | 91.5–92.2 ms (main 15.0) | n/a | 61.5–62.3 ms (main 19.7–20.1) | 28.8–30.2 ms (main 4.7–5.3) |
| Works in the Firefox sandbox | **throws** `'getComputedStyle' called on an object that does not implement interface Window` from `isInaccessible` (it calls an unbound `defaultView.getComputedStyle`, `is-inaccessible.mjs:16-25`) unless `{getComputedStyle: window.getComputedStyle.bind(window)}` is passed [bench] | — | yes (method calls on `ownerDocument.defaultView`, `domUtils.ts:86-93`) [bench] | yes |
| Agreement with Chrome, ref-worthy nodes [bench] | 2119 / 2335 (189 extra) | — | **2145 / 2147** | 2146 / 2149 (fit to the fixture) |
| Shadow DOM in names | **no** — children come from `childNodes` or slot assigned nodes, never `shadowRoot` (`accessible-name-and-description.mjs:204-214`, `:267`); open issue #768 "computeAccessibleName ignores text inside of shadow DOM" (2021-10-26) | — | yes — shadow root children, slot assigned nodes, skips slotted light children (`roleUtils.ts:1038-1092`) | yes |
| Known gaps | `section`→`region` and `form`→`form` unconditionally, `header`/`footer` unscoped, `th`→`columnheader` always (#1012), `td` never `gridcell`, no role for `password`/`date`…, first role token unvalidated (`getRole.mjs:17-66`, `:106-187`); hidden ignores `inert`/`content-visibility` (`:31-40`); `getRole` recomputed per check, style cache per call only (#1011 "Performance") | — | `summary` → `generic` (Chrome `DisclosureTriangle`), `inert` not treated as hidden (only a TODO, `roleUtils.ts:71`), only `element.shadowRoot` (open roots) | presentational inheritance, closed `<details>` content (mismatches seen) |

Mismatch detail [bench, full diff]: DAA — 185 unnamed `section`s reported as `region`, 25 `th scope=row` as
`columnheader`, the two presentation-table cells, the closed-`<details>` link and the inert button kept, the password
input without role, the placeholder-only input without name. PW — `summary` role, inert button kept. Own —
two cells of the presentation table, the link inside the closed `<details>`. All three miss the 19 buttons inside
closed shadow roots (Chrome exposes them).

Payload [bench, PW walker incl. text]: 8,489 nodes for the 4,925-element page; CDP-shaped `AXNode` JSON 1,236,760 B,
a compact wire form (§6) 396,361 B; build 35.4 ms on a cold single run, `JSON.stringify` 2.6 ms. The extension
reaches the daemon over a WebSocket (VX `socket.ts:543`), so no native-messaging size cap applies, but 3× is worth
taking.

---

## 3. Traversal

- **Flat tree**: for a shadow host visit `shadowRoot` children, for a `slot` its `assignedNodes()` (or its own
  children when nothing is assigned), otherwise children, skipping light nodes that are `assignedSlot`-ed elsewhere
  (PW `ariaSnapshot.ts:168-205`; ACC `#comp_name_from_content_find_child*`). Light children of a host that are not
  slotted are not rendered (PW `roleUtils.ts:337-366`). `aria-owns` re-parents: visit owned nodes under the owner and
  keep a visited set so they are not emitted twice (PW `ariaSnapshot.ts:96-139`).
- **Closed shadow roots**: Chrome `chrome.dom.openOrClosedShadowRoot(element)` / `browser.dom…` (Chrome 88+, CR-DOM
  `#availability`); `dom` is one of the APIs content scripts call directly (CR-CS `#capabilities`) and the reference
  page has no permissions section (contrast CR-DBG `#permissions`). Firefox has no `dom` namespace
  (BCD `version_added: false`, note "Use `element.openOrClosedShadowRoot`, available since Firefox 63"; MDN
  `dom.openOrClosedShadowRoot`). Only some elements can host a root — autonomous custom elements and `article, aside,
  blockquote, body, div, footer, h1–h6, header, main, nav, p, section, span` (MDN `attachShadow`) — the same list
  agent-browser uses to find closed hosts (AB `bidi/snapshot.rs:90-104`), so call the API only on those.
  User-agent roots (`details`, `input`, `video`) are not author roots; whether `openOrClosedShadowRoot` returns them
  is not stated by CR-DOM or MDN (unverified) — rely on `checkVisibility()` for their effect.
- **Frames**: the manifest content script runs in the top frame only (`content_scripts` has no `all_frames`, VX
  `dist/*/manifest.json`; `content.ts:11-13`); child frames get it on demand through
  `scripting.executeScript({target: {tabId, frameIds}})` (VX `client.ts:70-83`). For a snapshot, inject with
  `allFrames: true`; each `InjectionResult` carries `frameId` and `documentId` (MDN `scripting.executeScript`).
  Each frame snapshots its own document; the background stitches child trees under the parent's `Iframe` node, as
  BiDi does (§1.1), up to AB's depth cap of 4 (`snapshot.rs:369`). Mapping an `<iframe>` element to a frameId:
  Firefox `runtime.getFrameId(iframeElement)` (Firefox 96+, MDN, BCD); **Chrome has no `runtime.getFrameId`** (BCD
  `chrome: false`; absent from CHROMIUM `runtime.json`; not in CR-RT). In Chrome use a nonce: the parent posts
  `{vexdomFrame: nonce}` to `iframe.contentWindow` (cross-origin `postMessage` is allowed; MDN warns any window can
  post, so treat it as a hint and check `event.source === window.parent`), the child's content script forwards the
  nonce with `runtime.sendMessage`, the background reads `sender.frameId` (unverified end to end — P4 spike).
  `documentId` on `MessageSender`/`InjectionTarget` is Chrome 106+ but Firefox 153+ (BCD), above vexdom's
  `strict_min_version` 140, so it cannot be the document key on Firefox. `Iframe` node name = `title` (HAAM
  `#iframe-element-accessible-name-computation`); AB falls back to `title` then `name` for display (`snapshot.rs:598-604`, `:1007-1009`).
- **`<select>`**: Chrome 158 shapes it as `combobox` (value = selected text, `expanded=false`) → `MenuListPopup` →
  `option` × n with `selected`; `select multiple` as `listbox` → ignored wrapper → `option`s [bench `shapes.html`].
  Options of a closed select have no box, so a box-based visibility test drops them; PW exempts options inside
  `select` from the visibility check (`roleUtils.ts:329-332`).
- **contenteditable**: Chrome reports the editing host as `generic` with `value` = its text and `editable=richtext`
  [bench]; agent-browser's cursor scan marks it `editable` (`snapshot.rs:1063-1064`).
- **Cursor-interactive**: port `CURSOR_DETECTION_JS` (AB `snapshot.rs:1032-1112`) into the walk so each element's
  style is read once. In a Chrome isolated world `el.onclick` is `null` for handlers the page assigned
  (`oc.onclick = fn` and inline `onclick=""` alike), while `hasAttribute('onclick')` still sees the inline attribute
  [bench]; Blink returns an attribute listener only when it "BelongsToTheCurrentWorld" (CHROMIUM
  `event_target.cc:818-831`). The Firefox sandbox saw all three [bench]. `addEventListener` listeners are invisible
  everywhere. Cost of the stand-alone scan: 2.6 ms Chrome isolated, 10.7 ms Firefox sandbox, 4,925 el. [bench].

---

## 4. Refs

### 4.1 How agent-browser keeps them durable

`RefMap` holds `ref → RefEntry {backend_node_id, role, name, nth, frame_id, document}` and, per `(page, frame)`, a
`DocumentRefs {session, loader, refs: backend_id → ref}` (AB `element.rs:9-35`). `assign_refs` reuses the existing
ref of a backend id when the document is known, else allocates `e{next}` from a monotonic counter (`snapshot.rs:788-806`).
A new loader for the same frame replaces the bucket; same-document changes keep it (`element.rs:188-219`, `:550`).
On use: the document must still be the one the ref was issued in (`ensure_ref_document_current`, `element.rs:551-589`);
the cached node is tried first, a detached node counts as stale (`is_detached`, `element.rs:1033-1061`); then recovery
re-reads the AX tree and accepts **only a unique** role+name match, refusing refs that had `nth`
(`element.rs:1092-1158`). Firefox mirrors it: refs map to `(context, sharedId, documentId)`; recovery uses
`browsingContext.locateNodes` with an accessibility locator and is refused for `nth`, for nodes that were inside a
shadow root, and for a changed document (AB `bidi/target.rs:74-178`). Playwright instead reissues a ref whenever the
element's role or name changed (PW `ariaSnapshot.ts:224-237`).

### 4.2 Proposal

- **In the content script** (module state, lives as long as the content script's world for that document):
  `ids = new WeakMap<Element, number>()`, `els = new Map<number, WeakRef<Element>>()`, `next = 1`, and
  `docKey = crypto.randomUUID()` minted once per content-script instance. Same element object → same id on every
  snapshot, whatever its name does (agent-browser semantics). After a snapshot, drop map entries whose `deref()` is
  `undefined`. A `WeakRef` may keep returning a disconnected element because collection timing is unspecified (MDN
  `WeakRef`), so `isConnected` is the staleness test, never `deref() === undefined`. The DOM is never written to
  (no `data-*` tags; contrast AB's tag mode `snapshot.rs:1096`, which page `MutationObserver`s would see).
- **In the daemon**: map `(frameId, docKey, id)` to an `i64` from a session counter — the role `durable_id` plays in
  BiDi (AB `bidi/snapshot.rs:833-866`) — use it as `backendDOMNodeId`, put the frame in `node_contexts`, and call
  `observe_document(tab, None, tab, Some(topDocKey))`. The vendored `RefMap` then prints `e{n}`, keeps it across
  snapshots and handles multi-frame uniqueness unchanged.
- **Acting on `@eN`**: `RefEntry.backend_node_id` → `(frameId, docKey, id)`; send the action to that frame with
  `target: {ref: {doc: docKey, id, role, name, nth}}`. The content script answers `STALE_REF` when `docKey` is not its
  own (the frame navigated); otherwise `els.get(id)?.deref()` if connected; otherwise recover by role+name with the
  same refusals as §4.1 (unique only; not for `nth`; not for nodes that were in a shadow root).

| Situation | Outcome |
| --- | --- |
| Same element, attributes/text changed | same id, same `e{n}` |
| Element moved in the DOM | same object → same id |
| Element replaced (framework re-render) | old id disconnected → recovery by unique role+name, else `STALE_REF` |
| Same-document navigation (`pushState`) | document and content script survive → refs valid (as AB, `element.rs:550`) |
| Cross-document navigation or frame reload | new content script, new `docKey` → old refs fail fast |
| Content script re-injected after extension reload | new instance, new `docKey` → refs fail fast |

### 4.3 Wiring into `resolveTarget`

`targetSchema` has `selector/text/role/nth` and `resolveTarget` pools `document.querySelectorAll(selector ??
INTERACTIVE)`, then filters by role, text and `isVisible` (VX `dom.ts:4-9`, `:78-102`); `document.querySelectorAll`
never enters shadow roots, so refs are the first way to target shadow DOM content. Add `ref` to the schema and branch
first: `if (target.ref) { const el = resolveRef(target.ref); if (!options.includeHidden && !isVisible(el)) throw …;
return el; }`. Callers assume `HTMLElement` (`click-element.ts:13-25` calls `el.click()`); a ref may point at an
`SVGElement` (an `svg` with a role), and `SVGElement` has `focus`/`blur` but no `click` (BCD `api/SVGElement.json`;
MDN `HTMLElement.click`), so `resolveRef` must either reject non-`HTMLElement`s for such actions or the click path must
fall back to dispatching a `click` event. Routing: today an invocation goes to the focused frame
(`invokeInTab` → `focusedFrame`, VX `client.ts:14-20`); a ref carries its own frame and must override the focus.

---

## 5. What `getPageInfo` / `dom.ts` already gives

| Piece | Where | Use for `page.snapshot` |
| --- | --- | --- |
| `elementState` — `:disabled`/`aria-disabled`, checkbox/radio `checked`, `aria-expanded`, `selected`, `aria-current`, `required`, `aria-invalid`, select text, `filled` | `dom.ts:154-185` | **adapt**: checked lacks `mixed` (`indeterminate`, `aria-checked="mixed"` → `ariaFlag` returns undefined, `:137-140`); no `details[open]`→expanded; aria-disabled not inherited (PW walks the chain, `roleUtils.ts:1225-1240`) |
| `computedRole` | `dom.ts:62-68` | **replace**: first role token unvalidated (`:63`), `select multiple`→`combobox`, `a` without href → undefined, `header/footer` unscoped, no `section/img/li/table/th/td` |
| `accessibleText`, `fieldName` | `dom.ts:104-124` | **replace for names**: ignores `aria-labelledby`, uses `textContent` (hidden text, no shadow DOM, no CSS content), first label only, falls back to the `name` attribute, which is not in accname (HAAM §4.1.1). Keep for `target.text` matching so existing behaviour does not move |
| `isVisible`, `isExposed` | `dom.ts:126-135` | **keep for actions, not for the tree**: zero-size boxes are not hidden for AX (ACC note), and `closest('[aria-hidden="true"],[inert]')` stops at shadow boundaries |
| `cssPath`, `documentBounds`, `describeElement`, `submitsOnClick` | `dom.ts:187-229` | keep as is for action results; `documentBounds` per ref feeds `screenshot --annotate` (ADR-007) |
| Landmark roles, region labels | `get-page-info.ts:21-42`, `:115-129` | superseded by the role computation |
| Frame listing (`src`, `name`, `title`, `sandbox`) | `get-page-info.ts:99-113`, `switch-frame.ts:47-53` | reuse for `Iframe` nodes and their display name |
| Document header (url, title, viewport, scroll) | `get-page-info.ts:66-82` | reuse for the snapshot header |
| `frameIdOf` via `runtime.getFrameId` | `switch-frame.ts:55-65` | Firefox only (§3); its error text says "Chrome 116 or Firefox 96", but no Chrome version has the API (BCD, CHROMIUM) |
| Secret sealing | `invoke.ts:91` (`sealForPage` on every page result) | snapshot values pass through it; still never emit password values (Chrome masks them as `••••••` [bench]) |

---

## 6. Proposed result schema

The content script returns a compact tree; the daemon expands it into `AXNode`s (a ~30-line `From` impl) and calls
`snapshot_from_ax_nodes` unchanged. Index 0 is the root.

```ts
interface SnapshotResult {
  v: 1;
  docKey: string;              // crypto.randomUUID(), one per content-script instance
  url: string; title: string;
  viewport: { w: number; h: number; scrollX: number; scrollY: number };
  nodes: WireNode[];
  frames: FrameSlot[];         // child frames to stitch under their Iframe node
  stats: { elements: number; nodes: number; ms: number; truncated: boolean };
}
interface WireNode {
  id?: number;                 // durable element id (absent on StaticText) → backendDOMNodeId (after daemon mapping)
  r: string;                   // role (vocabulary below)
  n?: string;                  // accessible name; StaticText: the text
  v?: string | number;         // value: textbox text, combobox selected text, range valuenow; never for links or passwords
  s?: { level?: number; checked?: 'true' | 'false' | 'mixed'; expanded?: boolean; selected?: boolean;
        disabled?: boolean; required?: boolean; focused?: boolean };   // focused is carried, the pipeline ignores it
  c?: number[];                // child indexes, flat-tree order
  k?: CursorInfo;              // only for cursor-interactive elements
  u?: string;                  // link href (a.href), for --urls
  h?: string;                  // hint, only for controls with an empty or shared role+label (hint_targets rule)
  sh?: true;                   // element lives in a shadow tree (recovery is refused for it, as in bidi/target.rs)
}
interface CursorInfo {         // field names exactly as CURSOR_DETECTION_JS returns them (snapshot.rs:1253-1320)
  hasCursorPointer: boolean; hasOnClick: boolean; hasTabIndex: boolean; isEditable: boolean;
  text: string; isNative: boolean; hiddenInputType: 'radio' | 'checkbox' | null; hiddenInputChecked: string | null;
}
interface FrameSlot { node: number; frameId?: number; probe?: string; title?: string; name?: string; sameOrigin: boolean }
```

Daemon expansion: `nodeId = index`; `role = {type:"role", value:r}`; `name = {type:"computedString", value:n}`;
`value = {type: number ? "number" : "string", value:v}`; each `s` entry → `{name, value:{type, value}}` with `level` an
integer, `checked` a string, the rest JSON booleans (§1.2); `childIds = c`; `backendDOMNodeId = map(frameId, docKey,
id)`; `ignored = false`. `k` → `cursor_info_from_value`, `u` → `urls`, `h` → the `hints` closure's answer.

Role vocabulary: ARIA computed roles (what `getComputedRole` reports, CAAM `#roleMappingComputedRole`), except where
the pipeline keys on Chrome names — `RootWebArea` (root; `n` = title), `StaticText` (text nodes), `Iframe`
(frame elements). Labels as `generic` (or omitted) still get `promote_hidden_inputs` (`snapshot.rs:1557`); `summary`
as `button` (+`expanded` from `details.open`) stays ref'd. Do not emit `ListMarker`/`MenuListPopup`; options go
directly under their `combobox`. These are the deliberate divergences from Chrome-backed agent-browser output.

Example (abridged):

```json
{"v":1,"docKey":"5d1f…","url":"https://shop.example/","title":"Shop","viewport":{"w":1280,"h":900,"scrollX":0,"scrollY":0},
 "nodes":[{"id":1,"r":"RootWebArea","n":"Shop","c":[1,4]},
  {"id":7,"r":"navigation","n":"Primary","c":[2]},
  {"id":8,"r":"link","n":"Pricing","u":"https://shop.example/pricing","c":[3]},{"r":"StaticText","n":"Pricing"},
  {"id":12,"r":"main","c":[5,7]},
  {"id":13,"r":"heading","n":"Sign in","s":{"level":1},"c":[6]},{"r":"StaticText","n":"Sign in"},
  {"id":20,"r":"generic","k":{"hasCursorPointer":true,"hasOnClick":false,"hasTabIndex":false,"isEditable":false,"text":"Quick view","isNative":false,"hiddenInputType":null,"hiddenInputChecked":null},"c":[8]},
  {"r":"StaticText","n":"Quick view"}],
 "frames":[],"stats":{"elements":31,"nodes":9,"ms":1.2,"truncated":false}}
```

Input: `{ selector?: string; interactive?: boolean; maxNodes?: number; includeFrames?: boolean }`. `selector` sets
the walk root (the BiDi path scopes the same way, `bidi/snapshot.rs:810-822`); `interactive` may drop `StaticText` and
non-ref ancestors because interactive rendering flattens them anyway (`snapshot.rs:1750-1756`); `depth`/`compact`/
`urls` stay daemon-side render options.

---

## 7. Algorithm

1. **Start**: `t0 = performance.now()`; `beginAriaCaches()` (PW caches computed style, hidden state, role and name
   per element for the pass, `roleUtils.ts:1304-1341`); root = `document.querySelector(selector) ?? document.body`.
   Detect an open modal (`document.querySelector('dialog:modal')`) — outside it everything is inert (MDN `showModal`).
2. **Visit element `el`** (iterative DFS over the flat tree, §3, visited set for `aria-owns`):
   1. Skip `script, style, noscript, template`.
   2. *Excluded subtree*: `display:none` (non-slot), `aria-hidden="true"` (unless it is the focused element), `inert`
      on itself or a flat-tree ancestor, outside an open modal, `checkVisibility()` false (closed `details`,
      `content-visibility`; skip this test for `option` in `select`) → do not emit, do not descend (§2.1).
   3. *Invisible but descendable*: `visibility:hidden|collapse` → do not emit `el`, still descend (a child can be
      `visibility:visible`).
   4. `role = getAriaRole(el)` (first valid token; implicit from §2.2; presentational resolution §2.3), defaulting to
      `generic`; frame elements → `Iframe`; text inputs keep `textbox`; password → `textbox` without value.
   5. `none`/`presentation` → do not emit, descend with the parent as the attachment point.
   6. `name = getElementAccessibleName(el)` (§2.4); `''` for name-prohibited roles. Normalise whitespace; drop
      zero-width characters and map NBSP to space, as `normalize_snapshot_name` does (`snapshot.rs:162-179`).
   7. States: `level` (`h1–h6`, `aria-level` on heading/listitem/row/treeitem), `checked` (`true|false|mixed`,
      `indeterminate`), `expanded` (`details.open`, `aria-expanded` on allowed roles), `selected`, `disabled`
      (native, disabled fieldset/optgroup, `aria-disabled` chain), `required` — PW getters `roleUtils.ts:1094-1240`.
      Value: textbox/searchbox `value` (not password), combobox selected option text, range `valuenow`,
      contenteditable text.
   8. Cursor scan on the same computed style (port of `CURSOR_DETECTION_JS` incl. its native-tag and hidden-input
      rules); record `k` when it fires and `isConnected`, non-zero box.
   9. `id = ids.get(el) ?? assign()`; `sh = true` when `el.getRootNode() instanceof ShadowRoot`; links get `u = el.href`.
   10. Children: shadow root (open via `el.shadowRoot`; closed via `openOrClosedShadowRoot` for valid hosts), else slot
       assigned nodes, else children; text nodes → `StaticText` unless whitespace-only or not rendered
       (`roleUtils.ts:368-380`); textarea/input text never becomes a child. Children-presentational roles (§2.1) emit no
       descendant elements, only their text.
   11. Same-origin `about:blank`/`srcdoc` frames (no content script of their own) → walk `contentDocument` inline;
       other frames → leave an `Iframe` node and a `FrameSlot` (Firefox `runtime.getFrameId`, Chrome probe nonce).
3. **Hints**: count `(role, label)` over ref-worthy nodes (label = name, else cursor text); for controls with an empty
   or repeated pair compute `h` with the `HINT_FUNCTION` logic (`snapshot.rs:902-919`) — one round trip, no callback.
4. **Prune**: drop map entries of collected elements; stop at `maxNodes` with `truncated: true`.
5. **Finish**: `endAriaCaches()`; return the result; the background seals it (`invoke.ts:91`).
6. **Background** (for `includeFrames`): `scripting.executeScript({target:{tabId, allFrames:true}})` or per-frame
   messages, collect child results with their `frameId`, attach each under its `FrameSlot.node` (depth ≤ 4), send to
   the daemon.
7. **Daemon**: expand wire nodes to `AXNode`, map ids, `observe_document` with the top `docKey`, call
   `snapshot_from_ax_nodes`; keep the result as the baseline for deltas (ADR-007).

Budget at 5k elements [bench]: role+name ~13 ms Chrome / ~62 ms Firefox, plus cursor and serialization; expect
≲25 ms in Chrome and ≲90 ms in Firefox before real-page validation.

---

## Semantic traps

1. **Isolated worlds hide `onclick` properties in Chrome** — page-set handlers read `null` (CHROMIUM
   `event_target.cc:818-831`; [bench]); the Firefox sandbox sees them [bench]. Cursor detection must lean on
   `cursor:pointer`, `tabindex`, `contenteditable` and the `onclick` *attribute*, and will differ by browser.
2. **Firefox Xray**: unbound DOM functions throw (DAA case [bench]); page expandos are invisible and redefined
   built-ins show their native version (MDN *Sharing objects*, *Content scripts* DOM access); DOM-heavy code ran
   3–6× slower than in the page [bench] — the Firefox budget is the binding one.
3. **No `runtime.getFrameId` in Chrome** (BCD; CHROMIUM `runtime.json`; CR-RT). `frameIdOf` throws there
   (`switch-frame.ts:55-65`), so `frames/host.ts` `locateChild` (`:36-53`) appears to always return `null` in Chrome
   (read from code, not run). **Observed 2026-11-25** in Chrome for Testing 157.0.8103.78 (macOS arm64, headful;
   headless `--load-extension` loaded no page): an MV3 content script with `all_frames: true` reported
   `typeof chrome.runtime.getFrameId === "undefined"` in both the top document and its iframe.
4. **Closed shadow roots** need the extension-only API; `element.shadowRoot` missed 19 exposed buttons [bench].
5. **Shadow boundaries are walls for DOM helpers**: `closest()`, `parentElement`, `getElementById` and IDREFs are
   per tree; `aria-labelledby` resolves in the element's own root (PW `getIdRefs`, `roleUtils.ts:382-403`);
   `isExposed`'s `closest()` misses an `aria-hidden` host above a shadow root (`dom.ts:133-135`).
6. **Hidden is not invisible**: off-screen, `opacity:0` and sr-only text stay exposed and can name things (ACC note);
   a zero-size box is not "hidden" — `isVisible` would drop them (`dom.ts:126-131`).
7. **`visibility:hidden` is overridable** by a `visibility:visible` child; prune subtrees only for
   `display:none`/`aria-hidden`/`inert`/`content-visibility` (ARIA `#tree_exclusion`; PW `roleUtils.ts:314-366`).
8. **Inert without an attribute**: `showModal()` inerts the rest of the document (MDN); PW does not model inert at
   all (TODO `roleUtils.ts:71`; [bench] mismatch).
9. **Closed `<details>` and collapsed `<select>`**: the first has no rendered content (use `checkVisibility()`), the
   second has box-less `option`s that must stay (PW `roleUtils.ts:329-332`).
10. **Context-dependent roles** (§2.2): unnamed `section`/`form` are `generic`; scoped `header`/`footer` are not
    landmarks; `th` row vs column; `td` in grid; password/file have no ARIA role. DAA gets several wrong [bench].
11. **Role tokens and prohibited names**: first *valid* token (ARIA §9.1); `aria-label` on a `generic`/`paragraph`
    yields no name (ARIA `#namefromprohibited`) — `accessibleText` returns it anyway (`dom.ts:105-106`).
12. **Presentational conflicts**: focusable or global-ARIA elements ignore `role=none`; `td`/`tr`/`li` inherit
    `presentation` from their container (ARIA `#conflict_resolution_presentation_none`).
13. **Children Presentational**: no descendant elements under `button`, `checkbox`, `img`, `tab`, … (ARIA
    `#tree_exclusion`) — otherwise a nested link inside a button gets its own ref.
14. **AXValue shape**: `type` is mandatory; `expanded/selected/disabled/required` must be booleans and `level` an
    integer or they vanish (`types.rs:334-339`; `snapshot.rs:1931-1975`).
15. **Ignored ≠ removed**: ignored nodes are transparent, their children still render; an orphan node becomes a root
    (`snapshot.rs:1591-1594`, `:1683-1697`).
16. **Text granularity**: StaticText merging and parent-name dedupe assume one `StaticText` per DOM text run
    (`snapshot.rs:1633-1679`); pre-joining text changes output.
17. **Chrome vocabulary leaks into agent-browser output** (`image`, `LabelText`, `DisclosureTriangle`,
    `MenuListPopup`, `ListMarker`, `sectionheader`, `listitem [level=1]` [bench]); matching it byte for byte is not a
    goal, but the pipeline-keyed names (`RootWebArea`, `StaticText`, `Iframe`) are mandatory.
18. **Refs across name changes**: agent-browser keeps a ref for the same node; Playwright reissues on role/name change
    (§4.1) — pick one and test it; this note picks agent-browser's.
19. **`WeakRef` liveness is not staleness**: test `isConnected` (MDN `WeakRef`; AB `is_detached`, `element.rs:1033-1061`).
20. **Recovery must be unique** and refused for `nth`, shadow-tree nodes and changed documents (AB `element.rs:1092-1158`,
    `bidi/target.rs:119-178`) — a role+name guess can click the wrong row.
21. **Per-document identity on Firefox**: `documentId` arrives only in Firefox 153 (BCD) — use the content script's
    own `docKey`.
22. **Never write to the DOM while walking**: AB's tag mode sets attributes (`snapshot.rs:1096`); pages observe
    mutations. `WeakMap` ids avoid it.
23. **Passwords and secrets in values**: Chrome masks password values [bench]; emit none; everything else goes through
    `sealForPage` (`invoke.ts:91`).
24. **SVG refs**: `SVGElement` has no `click()` (BCD) while `clickElement` calls it (`click-element.ts:25`).
25. **Timer resolution**: Firefox reported whole milliseconds here; MDN documents 100 µs for non-isolated contexts
    (MDN `Performance.now`) — benchmark with repeated passes.
26. **happy-dom is not an oracle**: the `dom` test project runs in happy-dom with `getBoundingClientRect` mocked
    (`vitest.config.ts`, `get-page-info.test.ts`); layout, `checkVisibility` and computed styles need a real browser —
    use Chrome's CDP AX tree as the oracle, as in §0.2.

---

## Recommendation

1. **Role and name**: vendor PW `roleUtils.ts` + `domUtils.ts` (Apache-2.0, keep headers, add a `NOTICE` entry as
   ADR-003 does for agent-browser) and `cssTokenizer.ts` (CC0) at v1.65.1 into `packages/extension/src/lib/actions/page/ax/`. Patch
   three things: treat `inert` and modal-blocked content as hidden; route every `element.shadowRoot` read through a
   `shadowRootOf()` that uses `browser.dom.openOrClosedShadowRoot` (Chrome) / `element.openOrClosedShadowRoot`
   (Firefox) for valid hosts; map `summary` to `button` with `expanded`. Reasons: closest to Chrome [bench],
   accname 1.2 shadow/slot steps, WPT-tested, caches, 8.7 KB gzip, works in the Firefox sandbox. Not
   `dom-accessibility-api` (shadow DOM gap #768, wrong roles, slower, Firefox throw); not `aria-query` (tables
   only); not a from-scratch name algorithm (the prototype is ~2× faster but was fit to one fixture, and matching PW
   means re-deriving ~1.4k lines of edge cases).
2. **Walker and output**: write our own walker (§7) that emits the compact schema (§6); expand to `AXNode` in Rust and
   call `snapshot_from_ax_nodes` unchanged — the same pattern as `ax_nodes_from_snapshot` for Firefox. Reuse
   `cursor_info_from_value` by emitting its field names.
3. **Refs**: content-script durable ids + `docKey`; daemon maps them to `backendDOMNodeId`; the vendored `RefMap`
   prints and keeps `e{n}`; recovery in the content script with agent-browser's refusals.
4. **Frames**: one snapshot per frame, stitched by the background under `Iframe` nodes; Firefox keys frames with
   `runtime.getFrameId`, Chrome with a postMessage nonce (spike first).
5. **Proof**: a CDP harness like §0.2 in CI — fixture pages plus the WPT accname files, scored against
   `Accessibility.getFullAXTree`; that number is the "fidelity lower than CDP" in ADR-005, made measurable.

**Decision needed (ADR-005 wording)**: ADR-005 says the content script assigns `e{n}`. Under this design it assigns
a per-document element id and the daemon's `RefMap` assigns the printed `e{n}` (so refs stay unique across frames and
the pipeline stays unchanged). The alternative — content-script `e{n}` printed as is — needs frame-qualified refs and
a change to `assign_refs`. Recommend amending ADR-005 to the former.

**Not verified here**: Chrome postMessage frame stitching; `openOrClosedShadowRoot` on user-agent roots and its cost
per call; the Firefox numbers inside a real content script (BiDi sandbox stand-in) and the harness crash; real-site
cost and agreement (synthetic fixture only); `about:blank`/`srcdoc` injection behaviour of
`scripting.executeScript({allFrames: true})`.
