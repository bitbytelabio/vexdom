# agent-browser snapshot vendoring plan (→ `crates/vexdom-snapshot`)

Source of truth: `../agent-browser` @ `f0a9835` ("Prepare v0.45.0"), Apache-2.0, binary-only crate
`cli/` (`cli/Cargo.toml:1-6`, edition 2021). Context: ADR-003 (vendor into `crates/`, NOTICE,
no `axe.min.js`), ADR-005 (content-script AX tree, content-script refs), ADR-007 (delta in action
replies, `--max-output`), `DECISIONS.md:22-66`.

Citation convention: `snapshot.rs`, `element.rs`, `actions.rs`, `diff.rs`, `e2e_tests.rs`, `mod.rs`
= `cli/src/native/<file>`; `bidi/snapshot.rs`, `cdp/types.rs` = `cli/src/native/<dir>/<file>`;
`output.rs`, `commands.rs`, `mcp.rs`, `color.rs` = `cli/src/<file>`. Repo-root files are written out.
agent-browser is not in the codebase-memory index; every claim below comes from reading the files.

**Evidence run.** The "take" ranges below were copied verbatim into a scratch crate (edition 2024,
rustc 1.100.0, deps `serde`, `serde_json`, `similar 2.7.0`, `getrandom 0.2`, dev `futures-util`).
After one edition-2024 fix (trap T1) `cargo test` printed `test result: ok. 53 passed; 0 failed`.
That is 50 upstream tests plus 3 probe tests that produced the samples in §6.2 and checked traps.
The scratch crate is outside the repo and may be cleared.

---

## 1. Inventory: what to take, what to cut

LOC = physical lines of the range. "Crates" = external crates the item itself uses.

### 1.1 `snapshot.rs`: tree, refs, render (take)

| Item | Lines | LOC | Crates | Coupling / required change |
|---|---|---|---|---|
| `INTERACTIVE_ROLES` | 105-126 | 22 | – | – |
| `TABLE_CELL_ROLES` | 128 | 1 | – | – |
| `CONTENT_ROLES` | 130-137 | 8 | – | – |
| `STRUCTURAL_ROLES` | 139-160 | 22 | – | **dead**: no reader. Upstream hides the warning with `#[allow(dead_code)]` (`mod.rs:46`). Drop it |
| `ZERO_WIDTH_CHARS`, `normalize_snapshot_name` | 162-178 | 17 | – | – |
| `SnapshotOptions` | 180-187 | 8 | – | `selector` is read only by the CDP path (386-399, 494-537) |
| `SnapshotRefContext` | 189-194 | 6 | – | `session_id`/`frame_id`/`document_is_known` exist only for RefMap document buckets. Replace them with `doc_id` (§2) |
| `TreeNode` + `empty`/`clear`/`label` | 196-272 | 77 | – | – |
| `LabelState` | 274-279 | 6 | – | – |
| `HiddenInputKind` | 281-303 | 23 | – | – |
| `CursorElementInfo` | 305-314 | 10 | – | `pub(crate)` with private fields, so a separate crate cannot build one. Make it `pub` and add a constructor or `Deserialize` |
| `RoleNameTracker` | 316-345 | 30 | – | **edition-2024 compile error at 341** (T1) |
| `hint_targets` | 685-717 | 33 | – | – |
| `assign_refs` | 719-839 | 121 | – | uses the RefMap durable/document API (730, 789-805, 821-823, 838). Rewrite per §2 |
| `finish_output` | 841-857 | 17 | – | – |
| `snapshot_from_ax_nodes` | 859-899 | 41 | – | async generic `hints` callback (863-871, 883-885); see Recommendation step 3. It does **not** call `drop_natively_interactive` (compare 548 with 872-873) and does not do selector roots (494-537) |
| `HINT_FUNCTION` (JS) | 901-918 | 18 | – | moves to the content script (TS); the Rust side gets only results |
| `CURSOR_DETECTION_JS` (JS) | 1029-1112 | 84 | – | moves to the content script. Choose a variant: see T18 |
| `cursor_info_from_value` | 1252-1317 | 66 | serde_json | – |
| `drop_natively_interactive` | 1532-1545 | 14 | – | the pure entry point must call it (T17) |
| `promote_hidden_inputs` | 1547-1577 | 31 | – | – |
| `build_tree` (+ nested `set_depth`) | 1579-1713 | 135 | – | calls `normalize_ax_role` (`element.rs:1160-1166`); recursive (T26) |
| `render_tree` | 1715-1846 | 132 | serde_json (name escaping 1773-1777) | recursive (T26) |
| `compact_tree`, `count_indent` | 1848-1897 | 50 | – | – |
| `extract_ax_string`, `extract_ax_string_opt`, `NodeProperties`, `extract_properties` | 1899-1970 | 72 | serde_json | `element.rs:1168-1179` holds a copy of `extract_ax_string`. Keep one |
| `build_dedup_set` | 1972-1984 | 13 | – | **dead** outside its tests. Drop it |

Live code taken: 920 LOC across the rows above. JS that moves to TS: 102. Dead code dropped: 35.

### 1.2 `snapshot.rs`: CDP and I/O (cut, replaced by the content script)

| Item | Lines | What it touches | vexdom replacement |
|---|---|---|---|
| `stage_snapshot` | 347-367 | `CdpClient`, staged `RefMap::for_snapshot` | the daemon stages a cloned `RefMap` in the same way (pattern: `bidi/snapshot.rs:907-918`) |
| `MAX_IFRAME_SNAPSHOT_DEPTH` | 369 | – | keep the value 4 as the content script's iframe-nesting cap |
| `take_snapshot` | 371-424 | `DOM.enable`/`Accessibility.enable` (379-384), selector scope plus `Runtime.releaseObjectGroup` (386-399), `find_frame_host_session` (402-410) | content script |
| `take_snapshot_inner` | 426-683 | `resolve_ax_session` (439-440); `Page.getFrameTree`/loader identity (443-451, 472-488); `Accessibility.getFullAXTree` (464-470); selector roots (492-537, pure logic); cursor scan (539-548); hint JS over CDP (562-566); link `href` over CDP (568-579); iframe resolve (581-606); iframe recursion and text splice (613-680) | content script walk; inline iframe children (T16) |
| `call_on_nodes` | 920-956 | `DOM.resolveNode` + `Runtime.callFunctionOn` | content script computes hints and `href`s |
| `frame_loader`, `loader_replaced` | 958-973 | CDP `loaderId` | `docId` (§2.3) |
| `IframeMetadata`, `resolve_iframe_frame` | 975-1012 | `DOM.describeNode` | content script sends the iframe `title`/`name` |
| `dom_attribute` | 1014-1027 | CDP attribute array (pure; `as_chunks` needs Rust ≥1.88) | its rules (case-insensitive name, trimmed, non-empty, `title` before `name`) move into TS |
| `outermost_matches_expression` | 1114-1122 | JS builder | content script `-s` scoping |
| `SELECTOR_SCOPE_GROUP`, `resolve_selector_backend_ids`, `array_element_object_ids` | 1124-1225 | Runtime/DOM over CDP | content script; keep the error texts at 529-532, 1156, 1160, 1164 |
| `CURSOR_SCAN_GROUP`, `find_cursor_interactive_elements` | 1227-1250 | CDP | content script |
| `CursorScan`, `scan_cursor_interactive_elements` (JS 1331-1461) | 1319-1530 | CDP | the JS becomes TS (T18) |
| `collect_backend_node_ids` | 1986-2006 | CDP DOM tree JSON | – |

### 1.3 `element.rs`: `RefMap`, `RefEntry`, `parse_ref`

| Item | Lines | LOC | Crates | Coupling / change |
|---|---|---|---|---|
| `RefEntry` | 9-20 | 12 | – | `allow_role_fallback` (17) is read only by the CDP resolvers (652-654, 928). `document` (19) is read only by `ensure_ref_document_current` (559). Replace both with `doc_id` |
| `DocumentRefs` | 22-27 | 6 | – | replaced (§2.3) |
| `RefMap` struct | 29-34 | 6 | – | `documents` becomes `doc_id`; `next_ref` becomes the ref high-water mark |
| `new`, `add`, `add_with_frame` | 37-78 | 42 | – | keep |
| `add_exact_backend_node`, `add_selector` | 80-120 | 41 | – | not used by snapshot. Drop them |
| `bind_recorded_document`, `bind_document` | 122-142 | 21 | – | replaced by `doc_id` |
| `get`, `entries_sorted`, `ref_ids`, `remove`, `begin_snapshot`, `for_snapshot` | 144-184 | 41 | – | keep. `entries_sorted` sorts by number (155-160) |
| `observe_document` … `invalidate_all_documents` | 186-280 | 95 | – | CDP loader model. Replace (§2.3) |
| `next_ref_num`, `set_next_ref_num` | 282-288 | 7 | – | keep as `ref_base` (§2.3) |
| `parse_ref` | 291-314 | 24 | – | keep verbatim (T11) |
| `normalize_ax_role` | 1159-1166 | 8 | – | keep |
| `select_unique_node_id` | 1124-1158 | 35 | – | optional: pure recovery by unique role and name (§2.3) |
| `STALE_DOCUMENT_MARKER`, `STATE_NOT_VERIFIED`, `document_gone` | 533-546 | 14 | – | keep the message texts; the logic is CDP-specific |
| `ensure_ref_document_current`, `current_document` | 551-609 | 59 | CDP | cut (§2.3) |

### 1.4 `cdp/types.rs`: AX types (take)

| Item | Lines | LOC | Crates | Note |
|---|---|---|---|---|
| `string_or_int`, `opt_vec_string_or_int` | 4-47 | 44 | serde, serde_json | `nodeId` and `childIds` accept a string or an integer |
| `GetFullAXTreeResult` | 310-314 | 5 | serde | can be dropped; the daemon gets a richer wire struct (§5) |
| `AXNode`, `AXValue`, `AXProperty` | 316-345 | 30 | serde | `Deserialize` only; add `Serialize` for recording/replay. The JSON key is `backendDOMNodeId` (T2) |

### 1.5 `actions.rs`: snapshot replies, delta, baselines (take and refactor)

| Item | Lines | LOC | Crates | Coupling / change |
|---|---|---|---|---|
| `SnapshotRevision` | 238-245 | 8 | serde_json (`Map`) | make it `pub(crate)` |
| `DaemonState.snapshot_revisions`, `.last_snapshots` | 713-719 | – | – | become a crate-owned `Baselines` struct |
| `DaemonState::remember_snapshot` | 928-936 | 9 | – | prunes with `browser.has_page_session` (930-932); the caller passes the live tab set |
| `snapshot_options` | 8620-8641 | 22 | serde_json | parses the command JSON (`maxDepth` key, 8636); belongs in the daemon, or `SnapshotOptions: Deserialize` |
| `snapshot_response` | 8664-8754 | 91 | serde_json | `DaemonState`: `remember_snapshot` (8674-8679), `ref_map` (8680-8692), `active_frame_id` (8676), `snapshot_revisions` (8718-8752); command flags `delta`/`full` (8704, 8720). Becomes a pure `Baselines::reply(...)` |
| `snapshot_delta_response` | 8756-8831 | 76 | serde_json | pure; take verbatim |
| `SnapshotScope`, `LastSnapshot` | 10218-10257 | 40 | – | take (for `diff snapshot`) |
| `implicit_baseline_scope` | 10259-10313 | 55 | serde_json | pure; take |
| `diff_snapshot_scope` | 10315-10348 | 34 | serde_json | reads `last_snapshots` and `active_frame_id`; pass them in |
| `diff_snapshot_response` | 10370-10412 | 43 | serde_json | file I/O for `--baseline` (10379-10403). The daemon reads the file; the crate takes `&str` |

### 1.6 `diff.rs` (take the text half)

| Item | Lines | LOC | Crates | Note |
|---|---|---|---|---|
| `ScreenshotDiffResult`, `diff_screenshot` | 4-11, 21-100 | 88 | `image` | **cut**: not part of snapshots |
| `SnapshotDiffResult` | 13-19 | 7 | – | take |
| `diff_snapshots`, `diff_text`, `diff_unified` | 102-165 | 64 | `similar` (`TextDiff::from_lines`, unified diff with `context_radius(3)`), serde_json | take; keep `similar = "2"` (T31) |
| `snapshot_comparison_text` | 276-326 | 51 | – | take (T10, T12) |

### 1.7 `output.rs`: truncation and content boundaries

| Item | Lines | LOC | Crates | Coupling |
|---|---|---|---|---|
| `BOUNDARY_NONCE`, `get_boundary_nonce` | 6-17 | 12 | `getrandom 0.2` | process-global nonce |
| `OutputOptions` | 19-24 | 6 | – | `from_flags` (26-34) reads `crate::flags::Flags`. Cut |
| `truncate_if_needed` | 36-59 | 24 | – | counts chars; the message names `--max-output` (50) |
| `format_with_boundaries` | 61-76 | 16 | – | literal `AGENT_BROWSER_PAGE_CONTENT` marker (67, 72) |
| `format_diagnostic` | 78-86 | 9 | – | – |
| `print_with_boundaries` | 88-94 | 7 | – | stdout. Cut |
| `format_snapshot_delta` | 96-104 | 9 | serde_json | pretty-printed JSON (101) |
| `boundary_origin` | 106-113 | 8 | serde_json | picks `origin`, then `finalUrl`, then `url` |
| `render_snapshot_diff` | 5339-5377 | 39 | `crate::color` (`color.rs:31-108`) | needs a color stub or must go |
| Snapshot printing in the CLI | 1345-1369 | 25 | – | daemon/CLI side, not the crate |

### 1.8 `bidi/snapshot.rs`: test support (optional take)

| Item | Lines | LOC | Note |
|---|---|---|---|
| `ax_nodes_from_snapshot`, `ax_value`, `property`, `push_node` | 193-285 | 93 | pure converter from Firefox `accessible.snapshot` JSON to `AXNode` (serde_json). It carries the role map (228-237) and state map (238-248). Useful as a test-fixture builder and as the reference for the content script's role vocabulary |
| `take_snapshot` (Firefox) | 804-901 | 98 | **the model for ADR-005**: outside durable ids are fed into `backend_d_o_m_node_id` (833-862, 865-866), then `snapshot_from_ax_nodes` runs (867-900). Read it as the design reference; do not vendor it |

---

## 2. Refs: agent-browser vs ADR-005

### 2.1 How agent-browser assigns refs

- **Who gets a ref.** A node with a role in `INTERACTIVE_ROLES` or `TABLE_CELL_ROLES`, a `CONTENT_ROLES` node with a non-empty name, or any node whose backend id is in the cursor-interactive map (`snapshot.rs:734-751`).
- **Which ref.** When the document is known, the ref is looked up in `RefMap.documents[(page_session, frame)].refs[backend_id]` and reused, or a new `e{next_ref}` is allocated and remembered (`snapshot.rs:789-799`). Unknown documents and nodes without an id get a fresh snapshot-local `e{next_ref}` (`snapshot.rs:801-805`). `next_ref` only grows (`element.rs:282-288`; test `element.rs:2190`). This is the promise "without recycling IDs" (`docs/content/docs/snapshots.mdx:93`).
- **`nth`.** Duplicates are counted per (role, name) per frame context (`snapshot.rs:729-787`). `nth` is stored only on `RefEntry` and is used only to refuse ambiguous recovery (`element.rs:1100-1105`).
- **Document identity.** CDP `(session, loaderId)` (`element.rs:186-219`), or for Firefox the document root's `sharedId` (`bidi/snapshot.rs:864-866`).

### 2.2 vexdom: the content-script id becomes the ref

**Wire contract.** The content script keeps a per-document `WeakMap<Element, n>` plus `n → WeakRef<Element>` and sends `n` as `AXNode.backendDOMNodeId`. Then:

- Every vendored map key (cursor, hints, URLs, frame contexts, selector roots) keeps its existing `i64` key with no change.
- The ref string is `format!("e{n}")`, which `parse_ref` (`element.rs:291-314`), `entries_sorted` (`element.rs:155-160`), the `removedRefs` sort (`actions.rs:8697-8702`) and the diff ref-stripping (`diff.rs:305-307`) already understand.

`n` must be a positive decimal integer with no leading zeros (T10).

**Which elements get an id.** Rust stays the authority on who gets a ref (`snapshot.rs:734-751`). The content script assigns ids lazily to a superset: any element it emits with an interactive, cell or content role, or that is cursor-interactive, or a label or generic node that wraps a hidden radio or checkbox. Assigning lazily keeps ids dense, which saves tokens. Rust renders a ref only when its own predicate holds *and* an id is present. "Predicate holds but no id" is a contract bug that a test should catch. Assigning an id to every element also works, but makes ids sparse (T9b).

**`assign_refs` edits (`snapshot.rs:719-839`).**

- In the first loop, add `should_ref &= node.backend_node_id.is_some()`, placed after line 751.
- Replace lines 789-805 with `let ref_id = format!("e{}", node.backend_node_id.unwrap());`.
- Delete `next_ref` (730, 838), `durable_ref`/`remember_durable_ref` (793-798) and `bind_recorded_document` (821-823).
- Remove the `session_id`, `frame_id` and `document_is_known` parameters. Keep `node_contexts`, because `nth` is per frame and `RefEntry.frame_id` comes from it (754-758, 775-779, 807-811).
- Update the high-water mark: `ref_map.ref_base = max(ref_base, n + 1)`.

**`RefMap` edits (`element.rs:22-289`).**

- `documents: HashMap<(String, Option<String>), DocumentRefs>` becomes `doc_id: Option<String>` (one `RefMap` per tab).
- `RefEntry.document` becomes `doc_id` and `allow_role_fallback` is dropped.
- Keep `map`, `next_ref` (renamed `ref_base`), `begin_snapshot` (clears the entries, keeps the base: `element.rs:173-175`) and `for_snapshot` (`element.rs:178-184`).
- Add `observe(doc_id) -> bool`: a changed `docId` clears all entries, so `removedRefs` lists them, and keeps `ref_base`.

### 2.3 Staleness: agent-browser mechanism → vexdom replacement

| agent-browser | Where | vexdom |
|---|---|---|
| Per (tab, frame) `(session, loaderId)` buckets; an unknown document keeps no refs | `element.rs:186-229` | `docId`: a random id the content script makes once per document instance and returns with every snapshot. The daemon's `RefMap.observe(docId)` resets on change |
| Refuse a snapshot whose loader changed during capture ("The document changed while the snapshot was being captured…") | `snapshot.rs:472-488` | not needed: the walk is synchronous in one task. If the reply's `docId` differs from what the request expected, the daemon resets the map (as above) |
| Refuse an action whose ref came from a replaced document. Message: `Ref @{ref_id} is from a document that is no longer loaded (the page navigated or the frame was removed). Take a new snapshot and use a fresh ref.` | `element.rs:551-589`, text 561-563 | the daemon sends `{ref, docId}` and the content script refuses on a mismatch with the **same text**. The daemon can also refuse early from `RefEntry.doc_id` |
| Monotonic ids across documents | `element.rs:282-288`, test 2190-2200 | content-script counters restart per document, so `e5` would be **recycled**. Fix: the daemon sends `refBase = RefMap.ref_base` with `page.snapshot`; a new document starts counting there and returns `maxRef` |
| Backend node gone: fall back to the one node with the same role and name, unless `nth` is set; otherwise `Element ref {id} is stale` | `element.rs:626-656`, 1092-1158 | the content script sees `WeakRef.deref()` missing or `!isConnected`, so the ref is stale. Optional fallback: run the pure `select_unique_node_id` (`element.rs:1124-1158`) over a fresh snapshot, keeping the "several elements now match" refusals |
| Unknown ref → `Unknown ref: {id}` | `element.rs:704, 973`; `bidi/target.rs:90` | agent-browser drops a ref that is absent from the latest snapshot (`begin_snapshot`). Decide whether vexdom keeps that gate or lets the content script resolve any id it issued (ADR-005 says "usable by every element action") |
| Iframe refs share one counter | `snapshot.rs:627-639` recurse with the same `ref_map` | the top-frame content script walks same-origin `contentDocument`s itself (ADR-005), so one counter covers them. A cross-origin frame with its own script instance would collide (T9c) |

---

## 3. Tests that can be vendored

Status: **as-is** = passed verbatim in the probe; **adapt** = the behavior holds but the setup or
expected values change; **drop** = tests cut code; **TS** = becomes a content-script test.

| Test (fn line) | File | Status |
|---|---|---|
| `test_normalize_snapshot_name_preserves_word_boundaries` (2027) | `snapshot.rs` | as-is |
| `test_cursor_scan_keeps_native_tags_without_interactive_role` (2054), helpers `tree_node`/`cursor_entry` (2035-2051) | `snapshot.rs` | as-is |
| `test_interactive_roles` (2078), `test_content_roles` (2087) | `snapshot.rs` | as-is |
| `test_compact_tree_basic` (2093), `_keeps_only_direct_ancestors` (2109), `_radio_checkbox` (2125), `_empty_interactive` (2145), `compact_tree_reference` + `_matches_reference_on_generated_trees` (2152, 2188) | `snapshot.rs` | as-is |
| `test_count_indent` (2226), `test_role_name_tracker` (2233) | `snapshot.rs` | as-is |
| `test_promote_label_with_hidden_radio` (2362), `_preserves_existing_name` (2391), `_skips_without_hidden_input` (2407), helpers (2338-2359) | `snapshot.rs` | as-is |
| `test_iframe_display_name_is_presentation_only` (2430) | `snapshot.rs` | as-is |
| `test_dedup_set_*` ×4 (2249, 2261, 2271, 2278) | `snapshot.rs` | drop (dead code) |
| `loader_replaced_detects_…` (2013); `resolve_ax_session` ×3 (2295, 2311, 2324) | `snapshot.rs` | drop (CDP) |
| `test_dom_attribute_reads_name_and_title_ignoring_blank_and_src` (2419) | `snapshot.rs` | TS (iframe display-name rules) |
| `durable_refs_invalidate_replaced_iframe_document`/`_session`/`_do_not_share_unknown_document` (70, 81, 90; CDP websocket mock 16-67) | `snapshot.rs` | rewrite as `docId`/`refBase` tests |
| `test_parse_ref_at_prefix`/`_equals_prefix`/`_bare`/`_invalid` (2156, 2162, 2167, 2173) | `element.rs` | as-is |
| `test_ref_map_basic` (2181), `test_ref_map_clear_preserves_monotonic_numbering` (2190) | `element.rs` | as-is |
| `refs_carry_the_document_they_were_issued_in` (2202), `document_identity_is_per_page_and_frame` (2225), `test_durable_refs_are_scoped_by_document_and_frame` (2249), `invalidating_one_page_preserves_other_page_documents` (2270) | `element.rs` | adapt to `doc_id` (they test the replaced API) |
| `test_select_unique_node_id_*` ×4 (2096, 2106, 2121, 2142) | `element.rs` | as-is if `select_unique_node_id` is taken |
| `test_diff_identical` (172), `_additions` (180), `_deletions` (188), `_unified_output` (195), `test_snapshot_diff_struct` (202), `test_diff_snapshots_identical_fast_path` (212), `bench_diff_snapshots_…` (224, `#[ignore]`) | `diff.rs` | as-is |
| `content_comparison_ignores_only_generated_ref_annotations` (333) | `diff.rs` | as-is |
| `snapshot_delta_tree_splice_preserves_changes_missing_from_ref_metadata` (23763), `test_snapshot_delta_unchanged_is_tiny` (23795), `test_snapshot_delta_reports_replacements_and_removals` (23812) | `actions.rs` | as-is |
| `test_bare_diff_snapshot_inherits_and_checks_snapshot_options` (25725) | `actions.rs` | adapt (uses `precondition_code`, `browser.rs:249-252`, a 4-line helper) |
| `test_format_with_boundaries_applies_max_output` (5752) | `output.rs` | as-is (truncation) |
| `test_format_with_boundaries_wraps_content` (5764), `test_failure_text_quoting_the_page_stays_inside_boundaries` (5779), `test_snapshot_diff_keeps_page_text_inside_boundaries` (5799), `test_snapshot_delta_uses_boundaries_and_max_output` (5828), `test_boundary_origin_supports_read_metadata` (5861) | `output.rs` | as-is only if boundaries are vendored (§Recommendation says no) |
| fixtures `leaf`/`page` (1060-1066), `tag_snapshot_nodes` (1248), `render_with` (1266) | `bidi/snapshot.rs` | as-is (need `ax_nodes_from_snapshot` and `futures_util::FutureExt::now_or_never`, or a sync API) |
| `renders_chrome_format` (1315), `duplicate_buttons_get_distinct_refs_with_nth` (1352), `duplicate_names_in_different_frames_do_not_get_nth` (1366), `refs_survive_insertion` (1405), `link_targets_show_only_with_urls_and_never_as_values` (1482) | `bidi/snapshot.rs` | as-is under the upstream allocator; **adapt the expected ref numbers** under ADR-005 (refs become `e{id}`, and `render_with` gives the document root id 1, so the first button is `e2`) |
| `changed_document_allocates_new_refs_and_rejects_stale_refs` (1425) | `bidi/snapshot.rs` | adapt: keep 1426-1438, drop the `DaemonState`/`resolve_target` part (1439-1445) |
| `e2e_snapshot_cursor_interactive` (12002), `e2e_snapshot_hints_ambiguous_controls` (12120), `e2e_snapshot_cursor_many_elements` (13091), `e2e_snapshot_selector_includes_shadow_root` (13157), `e2e_snapshot_cursor_scan_*` (13220, 13296, 13358, 13397, 13443), `e2e_snapshot_selector_lists_matches_in_page_order` (13482), `e2e_ref_recovery_*` (11624-11720) | `e2e_tests.rs` | not vendorable (need Chrome). Use their HTML and assertions as **specs for the content-script port** |

Upstream tests passing in the probe: snapshot 19, element 10, delta 3, diff 7, bidi 5, output 6 = 50.

---

## 4. Licensing (Apache-2.0)

**Upstream facts.**

- License: `LICENSE` (Apache-2.0), copyright line `Copyright 2025 Vercel Inc.` (`LICENSE:189`); `cli/Cargo.toml:5` says `license = "Apache-2.0"`.
- No NOTICE covers the CLI. `git ls-files` finds only `docs/src/components/stageshow/vendor/NOTICE.md`, which covers the docs site's MIT Stageshow kit. That code is not in the vendored set, so §4(d) has nothing to carry over.
- No per-file copyright or SPDX headers in the needed files: `snapshot.rs:1`, `element.rs:1`, `diff.rs:1`, `output.rs:1` and `cdp/types.rs:1` all start with `use`.

**Obligations (`LICENSE:89-128`).**

- §4(a) give a copy of the license. vexdom's root `LICENSE` is already Apache-2.0 (`LICENSE:1-3`, `package.json:5`). Set `license = "Apache-2.0"` in the crate.
- §4(b) modified files must carry prominent change notices.
- §4(c) keep the upstream attribution notices: here, the copyright line.
- §6 trademarks: use "agent-browser"/"Vercel" only to describe where the code came from. Rename the `AGENT_BROWSER_PAGE_CONTENT` marker (`output.rs:67, 72`), as ADR-004 also requires.

**NOTICE addendum** (vexdom `NOTICE` currently ends at line 7):

```
This product includes software derived from agent-browser
(https://github.com/vercel-labs/agent-browser), version 0.45.0 (commit f0a9835),
Copyright 2025 Vercel Inc., licensed under the Apache License, Version 2.0.
The derived files are in crates/vexdom-snapshot/ and were modified by the vexdom authors.
```

**Per-file header** (one per vendored file):

```rust
// Derived from agent-browser v0.45.0 (f0a9835), cli/src/native/snapshot.rs.
// Copyright 2025 Vercel Inc. Licensed under the Apache License, Version 2.0.
// Modified by the vexdom authors: refs taken from content-script ids; CDP capture removed;
// synchronous hint/url inputs; edition 2024.
```

**Non-Apache files.**

- `native/a11y/axe.min.js` is MPL-2.0 (`a11y/mod.rs:3-4`, embedded at `a11y/mod.rs:19`). Next to it are `LICENSE-axe-core.txt` (MPL-2.0) and `LICENSE-axe-core-THIRD-PARTY.txt` (MIT list).
- `native/react/installHook.js` is Meta's (`installHook.js:4`).
- None of them is used by the needed set: grepping `snapshot.rs`, `element.rs`, `diff.rs`, `cdp/types.rs` and `bidi/snapshot.rs` finds no reference to axe, a11y or installHook. The axe mentions in `output.rs` (922-1005, 4501-4506) are outside the vendored ranges. **Confirmed: `axe.min.js` is not needed** (ADR-003).

**Dependency licenses** (from the registry `Cargo.toml`s): `similar 2.7.0` is Apache-2.0; `serde`, `serde_json` and `getrandom 0.2` are MIT OR Apache-2.0.

---

## 5. Size, layout and public API

**Estimated vendored LOC** (code / tests, after the cuts above):

| Module | From | Code | Tests |
|---|---|---|---|
| `ax.rs` | `cdp/types.rs:4-47, 316-345` + new wire struct | ~130 | ~40 (wire JSON) |
| `tree.rs` | `snapshot.rs:105-178` (without 139-160), 196-345 (types), 1252-1317, 1532-1713, 1899-1970 | ~520 | ~120 |
| `refs.rs` | `element.rs:9-314` (trimmed), 1159-1166, [1124-1158]; `snapshot.rs:316-345, 685-839` | ~330 | ~180 |
| `render.rs` | `snapshot.rs:180-187, 841-899, 1715-1897` | ~250 | ~200 |
| `delta.rs` | `actions.rs:238-245, 8664-8831, 10218-10313` | ~280 | ~120 |
| `diff.rs` | `diff.rs:13-19, 102-165, 276-326` | ~125 | ~80 |
| `output.rs` | `output.rs:36-59` | ~30 | ~15 |
| `tests/support` | `bidi/snapshot.rs:193-285, 1060-1066, 1248-1312` | ~160 (test-only) | ~210 |
| **Total** | | **~1,650** | **~950** |

**Layout** (`crates/vexdom-snapshot`, edition 2024, `publish = false`, deps `serde` (derive), `serde_json`, `similar = "2"`; no tokio, no futures):

```
src/lib.rs      re-exports, provenance doc comment
src/ax.rs       AXNode/AXValue/AXProperty (+Serialize), string_or_int helpers, PageSnapshot (wire)
src/tree.rs     role tables, TreeNode, build_tree, extract_*, CursorInfo (+from_value),
                promote_hidden_inputs, drop_natively_interactive, normalize_snapshot_name
src/refs.rs     RefMap, RefEntry, parse_ref, normalize_ax_role, RoleNameTracker,
                assign_refs (content-script ids), hint_targets, [select_unique_node_id]
src/render.rs   SnapshotOptions, render_tree, compact_tree, finish_output, render()
src/delta.rs    SnapshotRevision, Baselines (cap 32), SnapshotReply, snapshot_delta_response,
                SnapshotScope, implicit_baseline_scope
src/diff.rs     SnapshotDiffResult, diff_snapshots, snapshot_comparison_text
src/output.rs   truncate()
tests/          firefox_shape.rs (bidi fixtures), page_snapshot.rs (golden content-script JSON)
```

**Public API for the daemon:**

```rust
#[derive(Deserialize)] #[serde(rename_all = "camelCase")]
pub struct PageSnapshot {             // reply of the extension action `page.snapshot`
    pub doc_id: String, pub url: String, pub max_ref: u64,
    pub nodes: Vec<AXNode>,                                   // backendDOMNodeId = content-script id
    #[serde(default)] pub cursor: HashMap<i64, CursorInfo>,   // scan result per id
    #[serde(default)] pub hints: HashMap<i64, String>,        // HINT_FUNCTION results (superset)
    #[serde(default)] pub urls: HashMap<i64, String>,         // link href (only with `urls`)
    #[serde(default)] pub frames: HashMap<i64, String>,       // node → frame context
    #[serde(default)] pub frame_names: HashMap<i64, String>,  // iframe title/name fallback
}
pub struct SnapshotOptions { pub selector: Option<String>, pub interactive: bool,
                             pub compact: bool, pub depth: Option<usize>, pub urls: bool }
pub fn render(page: &PageSnapshot, opts: &SnapshotOptions, refs: &mut RefMap) -> String;
pub fn parse_ref(input: &str) -> Option<String>;
impl RefMap { pub fn new() -> Self; pub fn observe(&mut self, doc_id: &str) -> bool;
              pub fn begin_snapshot(&mut self); pub fn get(&self, r: &str) -> Option<&RefEntry>;
              pub fn entries_sorted(&self) -> Vec<(String, RefEntry)>;
              pub fn ref_ids(&self) -> HashSet<String>; pub fn ref_base(&self) -> u64; }
impl Baselines { pub fn new(cap: usize) -> Self;
                 pub fn reply(&mut self, tab: &str, url: &str, opts: &SnapshotOptions,
                              tree: String, refs: &RefMap, force_full: bool) -> SnapshotReply;
                 pub fn forget(&mut self, tab: &str); }
pub enum SnapshotReply { Full { .. }, Unchanged { .. }, Delta { .. } }  // same JSON shapes as actions.rs:8726/8765/8815
pub fn diff_snapshots(before: &str, after: &str) -> SnapshotDiffResult;
pub fn snapshot_comparison_text(tree: &str) -> String;
pub fn truncate(content: &str, max_chars: Option<usize>) -> String;
```

The daemon keeps: the action call, staging/committing a cloned `RefMap` (pattern: `bidi/snapshot.rs:907-918`), reading the `--baseline` file, the `removedRefs` list (`actions.rs:8692-8702`), command-JSON parsing and the fence.

---

## 6. Sample outputs

### 6.1 Upstream assertions and fixtures (quoted)

**`-i`, real page (example.com).** `README.md:41`:

```
- link "Learn more" [ref=e1]
```

**`-i` with hints.** Example in `docs/content/docs/snapshots.mdx:55-60`; the e2e test asserts the same strings plus `hint="in:Invoice 1"` / `hint="in:Invoice 2"` (`e2e_tests.rs:12158-12167`):

```
- link [ref=e1] hint="img:cart"
- button [ref=e2] hint="icon:icon-trash; in:Blue mug"
- button [ref=e3] hint="icon:fa-heart; in:Blue mug"
- button [ref=e4] hint="img:pencil; id:edit-btn; in:Blue mug"
- textbox "Email" [ref=e5] hint="name:login-email"
- textbox "Email" [ref=e6] hint="name:newsletter-email"
```

**Full mode, first line (CDP e2e).** `e2e_tests.rs:9931`: `- button "Primary action" [ref=e1]`.

**Cursor-interactive suffix.** `e2e_tests.rs:13138` asserts it contains `[cursor:pointer, onclick]`.

**Full mode, AX-node pipeline.** `bidi/snapshot.rs:1341-1348`:

```
button "OK" [ref=e1]
textbox "Email" [ref=e2]: a@b.c
checkbox "Agree" [checked=true, ref=e3]
button "Send" [disabled, ref=e4]
StaticText "hello"
```

**Duplicate names.** `bidi/snapshot.rs:1361-1362`: `button "OK" [ref=e1] hint="id:2"` / `button "OK" [ref=e2] hint="id:3"`. `nth` is `Some(0)`/`Some(1)` (1359-1360).

**`-u`.** `bidi/snapshot.rs:1512-1516`: `- link "Docs" [ref=e1]` without the flag; `- link "Docs" [ref=e1, url=https://x.test/docs]` with it.

**Durable refs.** `refs_survive_insertion` (`bidi/snapshot.rs:1420-1421`): after inserting `New` before `OK`, the output has `button "OK" [ref=e1]` and `button "New" [ref=e2]`. In `changed_document_…` (1431, 1437), the same page under a new loader renders `button "OK" [ref=e2]`.

**`-c`.** `snapshot.rs:2115-2120`. Input:

```
- main
  - empty-section
    - empty-paragraph
  - section
    - empty-paragraph
    - paragraph
      - StaticText "Saved successfully"
  - trailing-section
- empty-root
```

Output (exact equality):

```
- main
  - section
    - paragraph
      - StaticText "Saved successfully"
```

With `interactive = true`, the same holds for `button "Save" [ref=e1]`. `snapshot.rs:2099` asserts that `- status\n  - StaticText "Saved successfully"` is kept in non-interactive compact mode and dropped with `-i` (2103).

**Delta.**

- `actions.rs:23763-23792`: applying `treeChange` (`startLine`, `deleteCount`, `lines`) to the old tree reproduces the new tree exactly. This includes changes that the ref metadata cannot express (a checkbox gaining `[checked]`).
- `actions.rs:23795-23809`: an identical tree gives `kind: "unchanged"` with `baseRevision` and no `tree`.
- `actions.rs:23812-23847`: `replace @e1`, `remove @e2`, `add @e3`.
- Delta wire shape: `output.rs:5829-5833`, `{"kind":"delta","changes":[{"op":"add","ref":"@…","node":{…}}],"treeChange":{"startLine":0,"deleteCount":0,"lines":[…]}}`.

**Docs drift.** `docs/content/docs/snapshots.mdx:43-46` and `skill-data/core/references/snapshot-refs.md:44-64` show `@e1 [heading] "…"`, which `render_tree` never prints (`snapshot.rs:1760-1841`). Trust the code.

### 6.2 Probe outputs (generated by the vendored code)

Input: content-script-shaped AXNodes. The page is a nav with two links; a main with a heading, a required email textbox, an Agree checkbox, an icon button with a precomputed hint and a Save button; and a cursor-pointer div. The input is in the probe's `samples.rs`.

**Full:**

```
- navigation
  - link "Home" [ref=e1]
  - link "Cart" [ref=e2]
- main
  - heading "Checkout" [level=1, ref=e3]
  - textbox "Email" [required, ref=e4]: a@b.c
  - checkbox "Agree" [checked=false, ref=e5]
  - button [ref=e6] hint="icon:lucide-trash; in:Blue mug"
  - button "Save" [ref=e7]
- generic [ref=e8] clickable [cursor:pointer]
  - StaticText "Shipping is free"
```

**`-i`** (`-i -c` printed the same; `-c` printed the same as full):

```
- link "Home" [ref=e1]
- link "Cart" [ref=e2]
- heading "Checkout" [level=1, ref=e3]
- textbox "Email" [required, ref=e4]: a@b.c
- checkbox "Agree" [checked=false, ref=e5]
- button [ref=e6] hint="icon:lucide-trash; in:Blue mug"
- button "Save" [ref=e7]
- generic "Shipping is free" [ref=e8] clickable [cursor:pointer]
```

**`-u`:** `  - link "Home" [ref=e1, url=https://shop.test/]`.

**Delta `-i`** after checking Agree and renaming Save to Saved, on the same page plus 30 product links. Tree 1206 bytes, delta 309 bytes:

```json
{"origin":"https://shop.test/checkout","snapshot":{"baseRevision":1,
 "changes":[{"field":"name","op":"replace","ref":"@e7","value":"Saved"}],
 "kind":"delta","revision":2,
 "treeChange":{"deleteCount":3,"startLine":4,"lines":[
  "- checkbox \"Agree\" [checked=true, ref=e5]",
  "- button [ref=e6] hint=\"icon:lucide-trash; in:Blue mug\"",
  "- button \"Saved\" [ref=e7]"]}}}
```

The same change on the small page (317-byte tree) came back as `"kind":"full"` with the whole tree **and** a `refs` map (T5).

**Unchanged:** `{"origin":"https://shop.test/checkout","snapshot":{"baseRevision":1,"kind":"unchanged","revision":2}}`.

---

## Semantic traps

- **T1 Edition 2024.** `snapshot.rs:341` `.filter(|(_, &count)| count > 1)` does not compile ("cannot explicitly dereference within an implicitly-borrowing pattern"). Fix: `|&(_, &count)|`. That was the only error. `is_none_or` (`element.rs:203`, `actions.rs:8723`) needs Rust ≥1.82.
- **T2 Wire key.** `backend_d_o_m_node_id` with `rename_all = "camelCase"` is `backendDOMNodeId` (`cdp/types.rs:316-330`), and it must be an integer `i64`. `nodeId`/`childIds` may be strings or integers (`cdp/types.rs:4-47`). A child listed by two parents renders twice. Unknown child ids are dropped silently (`snapshot.rs:1622-1631`). Every node no one lists as a child becomes a root (`snapshot.rs:1686-1698`).
- **T3 Key order.** agent-browser's `serde_json` has no `preserve_order` (`cli/Cargo.lock:1989-1998`: no `indexmap` dependency), so `serde_json::Map` is a BTreeMap. The `refs` object and the delta `changes` order are **lexicographic**: the probe printed `["e1","e10","e11",…,"e19","e2"]` (`actions.rs:8771-8792`). Cargo feature unification in the vexdom workspace (any crate turning on `serde_json/preserve_order`) would silently change this order. Pin it with a test.
- **T4 Delta scope.** agent-browser computes a delta only for `snapshot --delta`/`--full` (`commands.rs:1531-1537`, `mcp.rs:3366-3371`). No action reply carries one, so ADR-007's delta in the click/fill reply is new behavior and goes in known-diffs. A plain `snapshot` returns before the delta baseline is stored (`actions.rs:8704-8708` returns before 8743). The `diff snapshot` baseline is a separate store (`actions.rs:8674-8679`, 10397-10403). Deltas therefore compare against the last *delta-mode* tree, which may not be the tree the agent saw last.
- **T5 Threshold.** The rule is delta JSON bytes ≥ 70% of tree **bytes** (`actions.rs:8822-8823`), not tokens. Because the delta JSON has fixed overhead, small trees always come back `full`. In delta mode a `full` reply repeats the tree's role/name data as a `refs` map (`actions.rs:8726, 8825`), and the CLI pretty-prints deltas (`output.rs:101`). Both cost tokens.
- **T6 One contiguous splice.** `treeChange` is a single common-prefix/common-suffix splice (`actions.rs:8797-8814`), so unchanged lines between two edits are sent again (the probe's `deleteCount: 3` for two changes).
- **T7 Forced full.** Any URL change, including hash or query, any options change, or `full` forces a full reply (`actions.rs:8720-8723`).
- **T8 Eviction.** At 32 baselines the one with the **lowest revision number** is evicted, not the least recently used (`actions.rs:8733-8742`).
- **T9 Ref ids.**
  - (a) Content-script counters restart per document. Without `refBase`, refs are recycled, which breaks the promise at `snapshots.mdx:93` (§2.3).
  - (b) Giving every element an id makes refs sparse (`e4500` instead of `e45`).
  - (c) Separate content-script instances (cross-origin frames) need disjoint id ranges.
- **T10 Ref format.** Refs must be `e` + ASCII digits. `parse_ref` (`element.rs:291-314`), numeric `entries_sorted` (`element.rs:155-160`), the `removedRefs` sort (`actions.rs:8697-8702`) and the diff stripping of only `ref=e<digits>` (`diff.rs:305-307`) all depend on it.
- **T11 `parse_ref`.** `parse_ref("@e")` and `parse_ref("ref=e")` return `Some("e")`, because only the bare form checks `len() > 1` (`element.rs:294-311`; confirmed by the probe). Leading zeros such as `e007` are accepted and do not equal `e7`.
- **T12 Raw value text.** Value text is printed raw after `": "` (`snapshot.rs:1833-1838`). A newline in a value breaks every line-based step: `compact_tree`/`count_indent`, the delta line splice, and the diff. An unbalanced `"` flips `snapshot_comparison_text`'s quote state **across lines**, because it never resets at `\n` (`diff.rs:281-292`). The probe showed `[ref=e2]` left unstripped on the next line. The content script should flatten and cap values.
- **T13 Three escapers.** Names go through serde_json escaping (`snapshot.rs:1773-1777`), the hint through Rust `Debug` formatting (`hint={:?}`, `snapshot.rs:1821`), and the value not at all. Any TS re-implementation must match all three.
- **T14 Compact is textual.** It keeps a line if it contains `ref=` or `": "`, or (non-interactive only) ` "` (`snapshot.rs:1870`). So `heading "Note: x"` is kept with `-i` (probe). `-i -c` ≈ `-i`, because `-i` already drops every non-ref node (`snapshot.rs:1752-1758`).
- **T15 Depth.** `-d` counts rendered indent, not DOM depth (`snapshot.rs:1736-1740`). Pass-through nodes do not count, and `-i` output is nearly flat, so `-d` barely matters there. Upstream renders each iframe from indent 0 before indenting it (`snapshot.rs:627-679`), so depth applies per frame; an inline tree changes that.
- **T16 Iframes.** Upstream finishes and compacts each child frame on its own and drops `(empty page)`/`(no interactive elements)` children (`snapshot.rs:641-645`). It splices child text after the `[ref=eN]` marker (`snapshot.rs:651-679`) and caps nesting at 4 (`snapshot.rs:369`). An inline `childIds` tree gives slightly different output. Iframes always get a ref (`snapshot.rs:125`) and never get a hint (`snapshot.rs:699`).
- **T17 Missing steps in the pure entry.** `snapshot_from_ax_nodes` skips `drop_natively_interactive` (compare `snapshot.rs:548` with 872-873) and selector roots (`snapshot.rs:494-537`). Add the first one when the scan reports `isNative`.
- **T18 Two cursor scanners.**
  - `CURSOR_DETECTION_JS` (`snapshot.rs:1032-1112`): no shadow roots; skips every native tag; no `isNative`.
  - The CDP scan (`snapshot.rs:1331-1461`): open shadow roots, `display: contents`, href-less links that wrap a control, `isNative`.

  ADR-005 needs the second. `cursor_info_from_value` reads `isNative` (`snapshot.rs:1312-1315`).
- **T19 Role vocabulary.** The content script must emit Chrome's internal role names: `INTERACTIVE_ROLES` includes `DisclosureTriangle`, `ColorWell` and `Iframe` (`snapshot.rs:105-126`). `RootWebArea`/`WebArea` are skipped (`snapshot.rs:1745`), `InlineTextBox` and ignored nodes are dropped (`snapshot.rs:1591`), `LabelText`/`generic` can be promoted (`snapshot.rs:1557`), and `IframePresentational` is normalized (`element.rs:1160-1166`). The Firefox mapping is the template (`bidi/snapshot.rs:228-237`).
- **T20 Explicit false states.** `checked=false` and `expanded=false` print only if the node actually carries the false value (`snapshot.rs:1786-1791`, 1946-1953). Firefox emits only true states (`bidi/snapshot.rs:238-248`), so it differs from Chrome there. `selected`, `disabled` and `required` print only when true (`snapshot.rs:1792-1806`).
- **T21 Text and generic nodes.** Adjacent `StaticText` siblings are merged, and a lone child that repeats its parent's name is dropped (`snapshot.rs:1641-1682`). This needs text emitted as `StaticText` children. A `generic` node with no ref and ≤1 child is collapsed (`snapshot.rs:1726`).
- **T22 Table cells.** Table cells always get a ref, even with no name (`snapshot.rs:128`, 737), so big tables blow up `-i` output.
- **T23 Label vs ref name.** With `-i`, an unnamed node prints its cursor text (`snapshot.rs:266-271`, 1767-1768), but `RefEntry.name` stays the empty AX name (`snapshot.rs:815-816`; probe `e8` has name `""`). `normalize_snapshot_name` runs only at render time (`snapshot.rs:1727`, 1774), so refs and the delta `replace` ops carry raw NBSP and zero-width characters.
- **T24 Hints.** `hint_targets` counts labels across the ref'd nodes (`snapshot.rs:686-717`), so the content script must send a superset. Each hint part is capped at 48 characters (`snapshot.rs:917`). Two different things are called hints: the cursor `hints` (`[cursor:pointer, onclick]`, `snapshot.rs:1825-1831`) and the DOM `hint=` (`snapshot.rs:1820-1822`).
- **T25 `--cursor` is a no-op.** The flag is parsed (`commands.rs:1525-1527`) but `snapshot_options` never reads it (`actions.rs:8621-8641`); the top-frame scan always runs (`snapshot.rs:541-547`).
- **T26 Recursion.** `set_depth` (`snapshot.rs:1700-1706`) and `render_tree` are recursive, so a pathologically deep DOM can overflow the stack. Cap depth in the content script or render on a thread with a larger stack.
- **T27 `diff --baseline`.** The baseline file drops exactly one trailing `\n` or `\r\n` (`actions.rs:10387-10394`). The comparison strips refs from both sides (`actions.rs:10403-10406`).
- **T28 `similar` version.** It is pinned at `"2"` (`cli/Cargo.toml`, lock 2.7.0; cargo reported 3.2.1 available). Upgrading may change the unified-diff text (not verified).
- **T29 Output helpers.** Truncation counts Unicode scalars, not bytes or tokens, and the message says `Use --max-output to adjust` (`output.rs:36-59`). The boundary nonce is process-global (`output.rs:6-17`), unlike vexdom's per-result fence tag (`src/daemon/guardrails/fence.ts:25-36`).
- **T30 `removedRefs`.** Delta-mode replies have no `removedRefs`; non-delta replies do (`actions.rs:8704-8707`). The MCP `snapshot` tool defaults to `interactive: true` (`mcp.rs:3349`).

## Recommendation

1. **Create `vexdom-snapshot` from verbatim copies** of the §1 "take" ranges. Edit only the following, each listed in the file's change header (§4):
   - (E1) T1 pattern fix;
   - (E2) `assign_refs`/`RefMap` rewritten for content-script ids, `docId` and `refBase` (§2.2-2.3);
   - (E3) make the pipeline synchronous: `hints`, `urls`, `frames` and `frame_names` become maps in `PageSnapshot`. This drops the generic future at `snapshot.rs:863-871` and the futures dependency;
   - (E4) call `drop_natively_interactive` from `render`;
   - (E5) `CursorElementInfo` becomes a `pub CursorInfo` with `Deserialize`;
   - (E6) drop the dead `STRUCTURAL_ROLES`/`build_dedup_set`;
   - (E7) `snapshot_response` becomes a `Baselines` struct with no `DaemonState`.
2. **Ref contract.** Ref = `e{backendDOMNodeId}`. The content script holds a `WeakMap`/`WeakRef` id table per document, assigns ids lazily to a superset of ref candidates, starts a new document at the daemon's `refBase`, and returns `docId` and `maxRef`. Rust's predicate decides who gets a ref. Keep the stale-document message text from `element.rs:561-563`. Decide separately whether to keep the "ref not in the latest snapshot means `Unknown ref`" gate and the role/name recovery (`select_unique_node_id`).
3. **Port the content-script side from the cut JS:** the CDP scanner `snapshot.rs:1331-1461` (not `CURSOR_DETECTION_JS`), `HINT_FUNCTION` (`snapshot.rs:902-918`), the selector semantics (`snapshot.rs:1115-1122`, 494-537) and the iframe naming rules (`snapshot.rs:1007-1027`). Use the §3 e2e cases as the specs. Emit Chrome role names, explicit false states, `StaticText` children, and flattened, capped values (T12, T19-T21).
4. **Delta (ADR-007).**
   - Vendor `snapshot_delta_response` verbatim with its 3 tests.
   - In vexdom, *every* tree shown to the agent (a `snapshot` or an action reply) updates the baseline (fixes T4).
   - The baseline key is the tab, the cap is 32, and a URL or options change forces full.
   - For the model, send compact JSON, and in a `full` reply send the tree without the redundant `refs` map. Measure it against `bench/tokens` before choosing between a text delta and a JSON delta.
   - Pin the ref ordering with a test (T3).
5. **Output.** Vendor `truncate_if_needed` only, reworded for vexdom's `--max-output`. **Do not vendor** `format_with_boundaries` or the nonce: vexdom's fence (GLOSSARY "Fence"; `fence.ts`) and ADR-007's "note stated once" replace them, and the literal would also break ADR-004. Leave `render_snapshot_diff` (needs `color`) in the CLI layer.
6. **Tests.** Vendor the 16 + 6 + 7 + 3 as-is tests from §3 (snapshot, element, diff, delta) and the `bidi/snapshot.rs` fixture tests with re-based ref numbers. Rewrite the CDP document-identity tests as `docId`/`refBase` tests. Add golden `PageSnapshot` JSON fixtures recorded from the real `page.snapshot` once it exists.
7. **NOTICE and headers** as in §4; take nothing under `a11y/` or `react/`.
