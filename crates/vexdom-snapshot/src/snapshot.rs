// Derived from agent-browser v0.45.0 (f0a9835), cli/src/native/snapshot.rs.
// Copyright 2025 Vercel Inc. Licensed under the Apache License, Version 2.0.
// Modified by the vexdom authors: CDP capture, the in-page scripts (`HINT_FUNCTION`, cursor
// scans), iframe recursion and selector roots removed (the content script walks the page, the
// extension background stitches frames); `render` replaces `snapshot_from_ax_nodes`, reads hints,
// link URLs, cursor info and iframe titles from the wire instead of async callbacks, and calls
// `drop_natively_interactive`; `assign_refs` takes ref numbers from the per-tab `RefMap`
// (docs/internals/decisions.md ADR-021) and refs only elements with a content-script id;
// `cursor_info_from_value` became `cursor_info_from_wire` over the typed `CursorInfo`; tree node
// ids are indexes (no `ignored` nodes, no never-read `depth`/`parent_idx`, no `set_depth`);
// `render_tree` keeps an explicit stack instead of recursing; `SnapshotOptions` derives
// comparison traits; dead `STRUCTURAL_ROLES`, `build_dedup_set` and `RoleNameTracker.entries`
// dropped; edition 2024 and clippy fixes.

use std::collections::HashMap;

use serde_json::Value;

use crate::ax::{AXNode, AXProperty, AXValue, WirePage, from_wire};
use crate::refs::{RefEntry, RefMap, normalize_ax_role};
use crate::wire::{CursorInfo, Offscreen, WireSnapshot};

const INTERACTIVE_ROLES: &[&str] = &[
    "button",
    "link",
    "textbox",
    "checkbox",
    "radio",
    "combobox",
    "listbox",
    "menuitem",
    "menuitemcheckbox",
    "menuitemradio",
    "option",
    "searchbox",
    "slider",
    "spinbutton",
    "switch",
    "tab",
    "treeitem",
    "DisclosureTriangle",
    "ColorWell",
    "Iframe",
];

const TABLE_CELL_ROLES: &[&str] = &["cell", "gridcell", "columnheader", "rowheader"];

/// Characters of a ref'd element's name the tree prints; longer names end in `…`. The ref map
/// keeps the whole name, and page text (`StaticText`, never ref'd) is never cut.
const MAX_REF_NAME: usize = 150;

const CONTENT_ROLES: &[&str] = &[
    "heading",
    "listitem",
    "article",
    "region",
    "main",
    "navigation",
];

const ZERO_WIDTH_CHARS: &[char] = &[
    '\u{FEFF}', // BOM / Zero Width No-Break Space
    '\u{200B}', // Zero Width Space
    '\u{200C}', // Zero Width Non-Joiner
    '\u{200D}', // Zero Width Joiner
    '\u{2060}', // Word Joiner
];

fn normalize_snapshot_name(name: &str) -> String {
    name.chars()
        .filter_map(|ch| match ch {
            '\u{00A0}' => Some(' '),
            ch if ZERO_WIDTH_CHARS.contains(&ch) => None,
            ch => Some(ch),
        })
        .collect()
}

/// How a snapshot is rendered.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SnapshotOptions {
    /// Print only elements with refs, flattened (`-i`).
    pub interactive: bool,
    /// Drop lines with neither a ref, a value nor (without `interactive`) a name, keeping the
    /// ancestors of the lines that stay (`-c`).
    pub compact: bool,
    /// Deepest printed indent level (`-d`); levels count printed lines, not DOM depth.
    pub depth: Option<usize>,
    /// CSS selector of the subtree to snapshot (`-s`). The content script applies it when it walks
    /// the page; here it only tells baselines apart.
    pub selector: Option<String>,
    /// Print link targets as `url=…` (`-u`).
    pub urls: bool,
    /// Only what is on screen (`--viewport`). The content script skips the rest; here it tells
    /// baselines apart.
    pub viewport: bool,
}

struct TreeNode {
    role: String,
    name: String,
    display_name: Option<String>,
    level: Option<i64>,
    checked: Option<String>,
    expanded: Option<bool>,
    selected: Option<bool>,
    disabled: Option<bool>,
    required: Option<bool>,
    value_text: Option<String>,
    backend_node_id: Option<i64>,
    children: Vec<usize>,
    has_ref: bool,
    ref_id: Option<String>,
    cursor_info: Option<CursorElementInfo>,
    url: Option<String>,
    hint: Option<String>,
}

impl TreeNode {
    // Create an empty node
    fn empty() -> Self {
        Self {
            role: String::new(),
            name: String::new(),
            display_name: None,
            level: None,
            checked: None,
            expanded: None,
            selected: None,
            disabled: None,
            required: None,
            value_text: None,
            backend_node_id: None,
            children: Vec::new(),
            has_ref: false,
            ref_id: None,
            cursor_info: None,
            url: None,
            hint: None,
        }
    }

    fn clear(&mut self) {
        self.role = String::new();
        self.name = String::new();
        self.display_name = None;
        self.level = None;
        self.checked = None;
        self.expanded = None;
        self.selected = None;
        self.disabled = None;
        self.required = None;
        self.value_text = None;
        self.backend_node_id = None;
        self.children.clear();
        self.has_ref = false;
        self.url = None;
        self.ref_id = None;
        self.cursor_info = None;
    }

    /// The name an interactive snapshot prints: the accessible name, or the cursor-interactive text when that is empty.
    fn label(&self) -> &str {
        match &self.cursor_info {
            Some(cursor_info) if self.name.is_empty() => &cursor_info.text,
            _ => &self.name,
        }
    }
}

/// How a ref's printed role and label identify it among the refs of one snapshot.
enum LabelState {
    Unique,
    Shared,
    Empty,
}

/// The type of a hidden form input found inside a cursor-interactive element.
#[derive(Clone, Copy)]
enum HiddenInputKind {
    Radio,
    Checkbox,
}

impl HiddenInputKind {
    fn parse(s: &str) -> Option<Self> {
        match s {
            "radio" => Some(Self::Radio),
            "checkbox" => Some(Self::Checkbox),
            _ => None,
        }
    }

    fn as_role(&self) -> &str {
        match self {
            Self::Radio => "radio",
            Self::Checkbox => "checkbox",
        }
    }
}

/// Information about a cursor-interactive element (elements with cursor:pointer, onclick, tabindex, etc.)
#[derive(Clone)]
pub(crate) struct CursorElementInfo {
    kind: String, // "clickable", "focusable", "editable"
    hints: Vec<String>,
    text: String, // textContent from the DOM element (fallback when ARIA name is empty)
    hidden_input_kind: Option<HiddenInputKind>,
    hidden_input_checked: Option<String>, // "true", "false", or "mixed" (tristate)
    native: bool, // native tag (a, summary, ...): dropped when the AX pass already gives it an interactive role
}

struct RoleNameTracker {
    counts: HashMap<String, usize>,
}

impl RoleNameTracker {
    fn new() -> Self {
        Self {
            counts: HashMap::new(),
        }
    }

    fn track(&mut self, role: &str, name: &str) -> usize {
        let key = format!("{}:{}", role, name);
        let count = self.counts.entry(key).or_insert(0);
        let nth = *count;
        *count += 1;
        nth
    }

    fn get_duplicates(&self) -> HashMap<String, usize> {
        self.counts
            .iter()
            .filter(|&(_, &count)| count > 1)
            .map(|(key, &count)| (key.clone(), count))
            .collect()
    }
}

/// Picks ref'd controls whose printed label is empty or shared with another ref, as (tree index, backend id).
fn hint_targets(tree_nodes: &[TreeNode]) -> Vec<(usize, i64)> {
    let mut label_counts: HashMap<(&str, &str), usize> = HashMap::new();
    for node in tree_nodes.iter().filter(|node| node.has_ref) {
        *label_counts
            .entry((node.role.as_str(), node.label()))
            .or_default() += 1;
    }
    tree_nodes
        .iter()
        .enumerate()
        .filter(|(_, node)| node.has_ref)
        .filter_map(|(idx, node)| {
            let is_control = node.cursor_info.is_some()
                || (INTERACTIVE_ROLES.contains(&node.role.as_str()) && node.role != "Iframe");
            let state = match (
                node.label(),
                label_counts[&(node.role.as_str(), node.label())],
            ) {
                ("", _) => LabelState::Empty,
                (_, 1) => LabelState::Unique,
                _ => LabelState::Shared,
            };
            let needs_hint = match state {
                LabelState::Unique => false,
                LabelState::Shared | LabelState::Empty => true,
            };
            node.backend_node_id
                .filter(|_| is_control && needs_hint)
                .map(|bid| (idx, bid))
        })
        .collect()
}

/// Allocates or reuses refs for interactive and named content nodes and records them in `ref_map`.
fn assign_refs(
    tree_nodes: &mut [TreeNode],
    cursor_elements: &HashMap<i64, CursorElementInfo>,
    page: &WirePage<'_>,
    ref_map: &mut RefMap,
    interactive: bool,
) {
    let mut trackers = HashMap::<i64, RoleNameTracker>::new();

    let mut nodes_with_refs: Vec<(usize, usize)> = Vec::new();

    for (idx, node) in tree_nodes.iter().enumerate() {
        let role = node.role.as_str();
        // Interactive mode prints only ref'd lines, so a cell ref would repeat the text of the
        // controls inside it; their lines take the cell's place.
        let mut should_ref = if INTERACTIVE_ROLES.contains(&role) {
            true
        } else if TABLE_CELL_ROLES.contains(&role) {
            !interactive
        } else if CONTENT_ROLES.contains(&role) {
            !node.name.is_empty()
        } else {
            false
        };

        if node
            .backend_node_id
            .is_some_and(|bid| cursor_elements.contains_key(&bid))
        {
            // ref elements that are cursor-interactive
            should_ref = true;
        }

        // A ref resolves through the content script's id of the element.
        let Some(bid) = node.backend_node_id.filter(|_| should_ref) else {
            continue;
        };
        let nth = trackers
            .entry(page.frame_of(bid))
            .or_insert_with(RoleNameTracker::new)
            .track(role, &node.name);
        nodes_with_refs.push((idx, nth));
    }

    let duplicates: HashMap<i64, HashMap<String, usize>> = trackers
        .into_iter()
        .map(|(context, tracker)| (context, tracker.get_duplicates()))
        .collect();

    for (idx, nth) in &nodes_with_refs {
        let node = &tree_nodes[*idx];
        let Some(key) = node.backend_node_id.and_then(|bid| page.ref_key(bid)) else {
            continue;
        };
        let duplicate = format!("{}:{}", node.role, node.name);
        let actual_nth = if duplicates
            .get(&key.frame_id)
            .is_some_and(|context_duplicates| context_duplicates.contains_key(&duplicate))
        {
            Some(*nth)
        } else {
            None
        };

        let ref_id = format!("e{}", ref_map.number(&key));
        ref_map.add(
            ref_id.clone(),
            RefEntry {
                key,
                role: node.role.clone(),
                name: node.name.clone(),
                nth: actual_nth,
            },
        );

        tree_nodes[*idx].has_ref = true;
        tree_nodes[*idx].ref_id = Some(ref_id);
    }

    // Populate cursor_info for ref-bearing nodes
    for (idx, _) in &nodes_with_refs {
        if let Some(bid) = tree_nodes[*idx].backend_node_id
            && let Some(cursor_info) = cursor_elements.get(&bid)
        {
            tree_nodes[*idx].cursor_info = Some((*cursor_info).clone());
        }
    }
}

/// Compacts the rendered tree when asked and substitutes the empty-page messages.
fn finish_output(mut output: String, options: &SnapshotOptions) -> String {
    if options.compact {
        output = compact_tree(&output, options.interactive);
    }

    let trimmed = output.trim().to_string();

    if trimmed.is_empty() {
        if options.interactive {
            return "(no interactive elements)".to_string();
        }
        return "(empty page)".to_string();
    }

    trimmed
}

/// Renders a page snapshot as text with element refs (`- button "Save" [ref=e7]`) and makes its
/// refs the latest snapshot of `refs`.
///
/// `frame_id` is the browser frame the snapshot was taken in (0 for the top frame); the trees of
/// child frames filled into [`FrameSlot`](crate::FrameSlot)s are rendered under their `Iframe`
/// node and keyed by their own frame ids. An element gets a ref when its role calls for one
/// (interactive, table cell outside interactive mode, named content or cursor-interactive) and the
/// content script gave it an id. It keeps its number in later snapshots for as long as its
/// document is in them; numbers are never reused.
#[must_use]
pub fn render(
    snap: &WireSnapshot,
    frame_id: i64,
    opts: &SnapshotOptions,
    refs: &mut RefMap,
) -> String {
    let page = from_wire(snap, frame_id);
    let (mut tree_nodes, roots) = build_tree(&page.nodes);
    let mut cursor_elements = page.cursor_elements();
    drop_natively_interactive(&mut cursor_elements, &tree_nodes);
    promote_hidden_inputs(&mut tree_nodes, &cursor_elements);
    let doc_keys: Vec<&str> = page.doc_keys().collect();
    refs.begin_snapshot(&doc_keys);
    assign_refs(
        &mut tree_nodes,
        &cursor_elements,
        &page,
        refs,
        opts.interactive,
    );
    for (idx, bid) in hint_targets(&tree_nodes) {
        if let Some(hint) = page.source(bid).h.as_ref().filter(|hint| !hint.is_empty()) {
            tree_nodes[idx].hint = Some(hint.clone());
        }
    }
    if opts.urls {
        for node in tree_nodes
            .iter_mut()
            .filter(|n| n.role == "link" && n.has_ref)
        {
            node.url = node
                .backend_node_id
                .and_then(|id| page.source(id).u.clone());
        }
    }
    for &(idx, name) in &page.frame_names {
        let node = &mut tree_nodes[idx];
        if node.role == "Iframe" && node.name.is_empty() {
            node.display_name = Some(name.to_string());
        }
    }
    let mut output = String::new();
    for &root in &roots {
        render_tree(&tree_nodes, root, 0, &mut output, opts);
    }
    let mut output = finish_output(output, opts);
    if let Some(note) = snap.offscreen.as_ref().and_then(offscreen_note) {
        output.push('\n');
        output.push_str(&note);
    }
    output
}

/// The line that tells a viewport-only tree what it left out; `None` when nothing was.
fn offscreen_note(offscreen: &Offscreen) -> Option<String> {
    let parts: Vec<String> = [
        (offscreen.above, "above"),
        (offscreen.below, "below"),
        (offscreen.aside, "to the side"),
    ]
    .into_iter()
    .filter(|&(count, _)| count > 0)
    .enumerate()
    .map(|(i, (count, place))| {
        if i == 0 {
            format!("{count} page regions {place}")
        } else {
            format!("{count} {place}")
        }
    })
    .collect();
    (!parts.is_empty()).then(|| {
        format!(
            "- (viewport only: {}; scroll, or snapshot without viewport)",
            parts.join(", ")
        )
    })
}

/// Builds the info the snapshot pipeline renders from the cursor scan's result for one element.
pub(crate) fn cursor_info_from_wire(element: &CursorInfo) -> CursorElementInfo {
    // Role differentiation: v0.19.0 uses 'clickable' for cursor:pointer or onclick,
    // 'focusable' for tabindex-only elements.
    let has_cursor_pointer = element.has_cursor_pointer;
    let has_on_click = element.has_on_click;
    let has_tab_index = element.has_tab_index;
    let is_editable = element.is_editable;

    let kind = if has_cursor_pointer || has_on_click {
        "clickable"
    } else if is_editable {
        "editable"
    } else {
        "focusable"
    };

    let mut hints: Vec<String> = Vec::new();
    if has_cursor_pointer {
        hints.push("cursor:pointer".to_string());
    }
    if has_on_click {
        hints.push("onclick".to_string());
    }
    if has_tab_index {
        hints.push("tabindex".to_string());
    }
    if is_editable {
        hints.push("contenteditable".to_string());
    }

    CursorElementInfo {
        kind: kind.to_string(),
        hints,
        text: element.text.trim().to_string(),
        hidden_input_kind: element
            .hidden_input_type
            .as_deref()
            .and_then(HiddenInputKind::parse),
        hidden_input_checked: element.hidden_input_checked.clone(),
        native: element.is_native,
    }
}

/// Native elements whose AX role is already interactive are ref'd by the AX
/// pass, so the cursor scan result for them is discarded. A native tag with any
/// other role (for example `<button role="menu">`) keeps its cursor entry.
fn drop_natively_interactive(
    cursor_elements: &mut HashMap<i64, CursorElementInfo>,
    tree_nodes: &[TreeNode],
) {
    let interactive: std::collections::HashSet<i64> = tree_nodes
        .iter()
        .filter(|n| INTERACTIVE_ROLES.contains(&n.role.as_str()) && n.role != "Iframe")
        .filter_map(|n| n.backend_node_id)
        .collect();
    cursor_elements.retain(|bid, info| !(info.native && interactive.contains(bid)));
}

/// Promote LabelText/generic nodes that wrap a hidden radio/checkbox input.
/// When a `<label>` contains a `display:none` `<input type="radio">`, the browser leaves the input
/// out of its accessibility tree and exposes only the label with role="LabelText" and an empty name.
/// We detect these via cursor-interactive scanning and promote the label to the correct input role
/// so consumers see role="radio" in data.refs.
fn promote_hidden_inputs(
    tree_nodes: &mut [TreeNode],
    cursor_elements: &HashMap<i64, CursorElementInfo>,
) {
    for node in tree_nodes.iter_mut() {
        if !matches!(node.role.as_str(), "LabelText" | "generic") {
            continue;
        }
        let cursor_info = match node
            .backend_node_id
            .and_then(|bid| cursor_elements.get(&bid))
        {
            Some(info) => info,
            None => continue,
        };
        if let Some(input_kind) = cursor_info.hidden_input_kind {
            node.role = input_kind.as_role().to_string();
            if node.name.is_empty() && !cursor_info.text.is_empty() {
                node.name = cursor_info.text.clone();
            }
            if let Some(ref checked) = cursor_info.hidden_input_checked {
                node.checked = Some(checked.clone());
            }
        }
    }
}

fn build_tree(nodes: &[AXNode]) -> (Vec<TreeNode>, Vec<usize>) {
    let mut tree_nodes: Vec<TreeNode> = Vec::with_capacity(nodes.len());

    for node in nodes {
        let role = normalize_ax_role(extract_ax_string(&node.role));
        let name = extract_ax_string(&node.name);
        let value_text = extract_ax_string_opt(&node.value);

        let (level, checked, expanded, selected, disabled, required) =
            extract_properties(&node.properties);

        if role == "InlineTextBox" {
            tree_nodes.push(TreeNode::empty());
            continue;
        }

        tree_nodes.push(TreeNode {
            role,
            name,
            display_name: None,
            level,
            checked,
            expanded,
            selected,
            disabled,
            required,
            value_text,
            backend_node_id: node.backend_d_o_m_node_id,
            children: Vec::new(),
            has_ref: false,
            ref_id: None,
            cursor_info: None,
            url: None,
            hint: None,
        });
    }

    // Build parent-child relationships
    for (i, node) in nodes.iter().enumerate() {
        for &child_idx in &node.child_ids {
            if child_idx < tree_nodes.len() {
                tree_nodes[i].children.push(child_idx);
            }
        }
    }

    // Process StaticText aggregation
    for i in 0..tree_nodes.len() {
        if tree_nodes[i].role.is_empty() || tree_nodes[i].children.is_empty() {
            continue;
        }

        let children_indices: Vec<usize> = tree_nodes[i].children.clone();

        // Continuous StaticText nodes at the same level are an artifact of HTML structure rather than semantic meaning.
        // They typically represent a single continuous piece of text on the page that was split due to inline elements, formatting tags, or other structural reasons.
        // Thus, continuous StaticText children are aggregated into the first one.
        let mut start = 0;
        while start < children_indices.len() {
            // Skip non-StaticText nodes
            if tree_nodes[children_indices[start]].role != "StaticText" {
                start += 1;
                continue;
            }

            // Find the end of the current StaticText sequence
            let mut end = start + 1;
            while end < children_indices.len()
                && tree_nodes[children_indices[end]].role == "StaticText"
            {
                end += 1;
            }

            // If we have a sequence of at least two StaticText
            if end > start + 1 {
                // Collect and aggregate all names from the sequence
                let aggregated_name: String = (start..end)
                    .map(|idx| tree_nodes[children_indices[idx]].name.clone())
                    .collect();
                // Always aggregate into the first node of the sequence
                tree_nodes[children_indices[start]].name = aggregated_name;
                // Clear the rest of the nodes in the sequence (from start+1 to end-1)
                for j in (start + 1)..end {
                    tree_nodes[children_indices[j]].clear();
                }
            }
            start = end;
        }

        // Deduplicate redundant StaticText
        if children_indices.len() == 1
            && tree_nodes[children_indices[0]].role == "StaticText"
            && tree_nodes[i].name == tree_nodes[children_indices[0]].name
        {
            tree_nodes[children_indices[0]].clear();
        }
    }

    // Find the roots: every node no one lists as a child
    let mut root_indices = Vec::new();
    let children_exist: Vec<bool> = nodes.iter().map(|_| false).collect();
    let mut is_child = children_exist;
    for node in &tree_nodes {
        for &child in &node.children {
            is_child[child] = true;
        }
    }
    for (i, &is_c) in is_child.iter().enumerate() {
        if !is_c {
            root_indices.push(i);
        }
    }

    (tree_nodes, root_indices)
}

/// Renders the subtree at `idx` depth first, keeping its own stack: a page can nest elements
/// deeper than a thread's stack would allow recursion.
fn render_tree(
    nodes: &[TreeNode],
    idx: usize,
    indent: usize,
    output: &mut String,
    options: &SnapshotOptions,
) {
    let mut stack = vec![(idx, indent)];
    while let Some((idx, indent)) = stack.pop() {
        let node = &nodes[idx];
        if let Some(child_indent) = render_node(node, indent, output, options) {
            let children = node.children.iter().rev();
            stack.extend(children.map(|&child| (child, child_indent)));
        }
    }
}

/// Writes the line of one node, if it prints one, and returns the indent of its children; `None`
/// when `options.depth` cuts the subtree off.
fn render_node(
    node: &TreeNode,
    indent: usize,
    output: &mut String,
    options: &SnapshotOptions,
) -> Option<usize> {
    // Reduce unnecessary indentation and rendering
    if node.role.is_empty()
        || (node.role == "generic" && !node.has_ref && node.children.len() <= 1)
        || (node.role == "StaticText" && normalize_snapshot_name(&node.name).trim().is_empty())
    {
        // Ignored node -- still render children
        return Some(indent);
    }

    if let Some(max_depth) = options.depth
        && indent > max_depth
    {
        return None;
    }

    let role = &node.role;

    // Skip root WebArea wrapper
    if role == "RootWebArea" || role == "WebArea" {
        return Some(indent);
    }

    if options.interactive && !node.has_ref {
        // In interactive mode, skip non-interactive but render children
        return Some(indent);
    }

    let prefix = "  ".repeat(indent);
    let mut line = format!("{}- {}", prefix, role);

    let unescaped_display_name = if !node.name.is_empty() {
        node.name.as_str()
    } else if node.role == "Iframe" {
        node.display_name.as_deref().unwrap_or("")
    } else if options.interactive {
        node.label()
    } else {
        ""
    };
    let mut display_name = normalize_snapshot_name(unescaped_display_name);
    if node.has_ref
        && let Some((cut, _)) = display_name.char_indices().nth(MAX_REF_NAME)
    {
        display_name.truncate(cut);
        display_name.push('…');
    }
    if !display_name.is_empty()
        && let Ok(display_name) = serde_json::to_string(&display_name)
    {
        line.push_str(&format!(" {}", display_name));
    }

    // Properties
    let mut attrs = Vec::new();

    if let Some(level) = node.level {
        attrs.push(format!("level={}", level));
    }
    if let Some(ref checked) = node.checked {
        attrs.push(format!("checked={}", checked));
    }
    if let Some(expanded) = node.expanded {
        attrs.push(format!("expanded={}", expanded));
    }
    if let Some(selected) = node.selected
        && selected
    {
        attrs.push("selected".to_string());
    }
    if let Some(disabled) = node.disabled
        && disabled
    {
        attrs.push("disabled".to_string());
    }
    if let Some(required) = node.required
        && required
    {
        attrs.push("required".to_string());
    }

    if let Some(ref ref_id) = node.ref_id {
        attrs.push(format!("ref={}", ref_id));
    }

    if let Some(ref url) = node.url {
        attrs.push(format!("url={}", url));
    }

    if !attrs.is_empty() {
        line.push_str(&format!(" [{}]", attrs.join(", ")));
    }

    if let Some(ref hint) = node.hint {
        line.push_str(&format!(" hint={:?}", hint));
    }

    // Add cursor-interactive kind & hints
    if let Some(ref cursor_info) = node.cursor_info {
        line.push_str(&format!(
            " {} [{}]",
            cursor_info.kind,
            cursor_info.hints.join(", ")
        ));
    }

    // Value
    if let Some(ref val) = node.value_text
        && !val.is_empty()
        && val != &node.name
    {
        line.push_str(&format!(": {}", val));
    }

    output.push_str(&line);
    output.push('\n');

    Some(indent + 1)
}

/// Retain matching content and its direct ancestor chain, including visible text in noninteractive snapshots.
fn compact_tree(tree: &str, interactive: bool) -> String {
    let lines: Vec<&str> = tree.lines().collect();
    if lines.is_empty() {
        return String::new();
    }

    let result = keep_with_ancestors(&lines, |line| {
        line.contains("ref=") || line.contains(": ") || (!interactive && line.contains(" \""))
    });

    let output = result.join("\n");
    if output.trim().is_empty() && interactive {
        return "(no interactive elements)".to_string();
    }
    output
}

/// The tree lines `keep` accepts, each with its chain of ancestors (lines of smaller indent above
/// it), in tree order.
pub(crate) fn keep_with_ancestors<'a>(
    lines: &[&'a str],
    keep: impl Fn(&str) -> bool,
) -> Vec<&'a str> {
    let mut kept = vec![false; lines.len()];
    // Index and indent of the open ancestors, outermost first. Keeping them in
    // a stack lets a kept line mark its ancestors once instead of walking back
    // over every earlier line, which made compact mode quadratic on big tables.
    let mut ancestors: Vec<(usize, usize)> = Vec::new();

    for (i, line) in lines.iter().enumerate() {
        let indent = count_indent(line);
        while let Some(&(_, ancestor_indent)) = ancestors.last() {
            if ancestor_indent < indent {
                break;
            }
            ancestors.pop();
        }

        if keep(line) {
            kept[i] = true;
            for &(ancestor, _) in &ancestors {
                kept[ancestor] = true;
            }
        }

        ancestors.push((i, indent));
    }

    lines
        .iter()
        .zip(kept)
        .filter_map(|(line, kept)| kept.then_some(*line))
        .collect()
}

fn count_indent(line: &str) -> usize {
    let trimmed = line.trim_start();
    (line.len() - trimmed.len()) / 2
}

fn extract_ax_string(value: &Option<AXValue>) -> String {
    match value {
        Some(v) => match &v.value {
            Some(Value::String(s)) => s.clone(),
            Some(Value::Number(n)) => n.to_string(),
            Some(Value::Bool(b)) => b.to_string(),
            _ => String::new(),
        },
        None => String::new(),
    }
}

fn extract_ax_string_opt(value: &Option<AXValue>) -> Option<String> {
    match value {
        Some(v) => match &v.value {
            Some(Value::String(s)) if !s.is_empty() => Some(s.clone()),
            Some(Value::Number(n)) => Some(n.to_string()),
            _ => None,
        },
        None => None,
    }
}

type NodeProperties = (
    Option<i64>,    // level
    Option<String>, // checked
    Option<bool>,   // expanded
    Option<bool>,   // selected
    Option<bool>,   // disabled
    Option<bool>,   // required
);

fn extract_properties(props: &Option<Vec<AXProperty>>) -> NodeProperties {
    let mut level = None;
    let mut checked = None;
    let mut expanded = None;
    let mut selected = None;
    let mut disabled = None;
    let mut required = None;

    if let Some(properties) = props {
        for prop in properties {
            match prop.name {
                "level" => {
                    level = prop.value.value.as_ref().and_then(|v| v.as_i64());
                }
                "checked" => {
                    checked = prop.value.value.as_ref().map(|v| match v {
                        Value::String(s) => s.clone(),
                        Value::Bool(b) => b.to_string(),
                        _ => "false".to_string(),
                    });
                }
                "expanded" => {
                    expanded = prop.value.value.as_ref().and_then(|v| v.as_bool());
                }
                "selected" => {
                    selected = prop.value.value.as_ref().and_then(|v| v.as_bool());
                }
                "disabled" => {
                    disabled = prop.value.value.as_ref().and_then(|v| v.as_bool());
                }
                "required" => {
                    required = prop.value.value.as_ref().and_then(|v| v.as_bool());
                }
                _ => {}
            }
        }
    }

    (level, checked, expanded, selected, disabled, required)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_snapshot_name_preserves_word_boundaries() {
        assert_eq!(
            normalize_snapshot_name("Capital\u{00A0}Federal"),
            "Capital Federal"
        );
        assert_eq!(normalize_snapshot_name("zero\u{200B}width"), "zerowidth");
    }

    fn tree_node(role: &str, backend_node_id: i64) -> TreeNode {
        let mut node = TreeNode::empty();
        node.role = role.to_string();
        node.backend_node_id = Some(backend_node_id);
        node
    }

    fn cursor_entry(native: bool) -> CursorElementInfo {
        CursorElementInfo {
            kind: "clickable".to_string(),
            hints: Vec::new(),
            text: String::new(),
            hidden_input_kind: None,
            hidden_input_checked: None,
            native,
        }
    }

    #[test]
    fn test_cursor_scan_keeps_native_tags_without_interactive_role() {
        let nodes = [
            tree_node("button", 1),
            tree_node("menu", 2),
            tree_node("DisclosureTriangle", 3),
            tree_node("generic", 4),
        ];
        let mut entries: HashMap<i64, CursorElementInfo> =
            (1..=5).map(|bid| (bid, cursor_entry(true))).collect();
        entries.insert(6, cursor_entry(false));
        entries.insert(1, cursor_entry(false));
        drop_natively_interactive(&mut entries, &nodes);

        // Native entry on an interactive AX role is dropped; everything else stays.
        assert!(!entries.contains_key(&3));
        assert!(entries.contains_key(&2));
        assert!(entries.contains_key(&4));
        assert!(entries.contains_key(&5));
        assert!(entries.contains_key(&6));
        // A non-native entry is kept even when its AX role is interactive.
        assert!(entries.contains_key(&1));
    }

    #[test]
    fn test_interactive_roles() {
        assert!(INTERACTIVE_ROLES.contains(&"button"));
        assert!(INTERACTIVE_ROLES.contains(&"textbox"));
        assert!(INTERACTIVE_ROLES.contains(&"DisclosureTriangle"));
        assert!(INTERACTIVE_ROLES.contains(&"ColorWell"));
        assert!(!INTERACTIVE_ROLES.contains(&"heading"));
    }

    #[test]
    fn test_content_roles() {
        assert!(CONTENT_ROLES.contains(&"heading"));
        assert!(!CONTENT_ROLES.contains(&"button"));
    }

    #[test]
    fn test_compact_tree_basic() {
        let tree = "- navigation\n  - link \"Home\" [ref=e1]\n  - link \"About\" [ref=e2]\n- main\n  - heading \"Title\"\n  - paragraph\n    - text: Hello\n- status\n  - StaticText \"Saved successfully\"\n- empty\n  - paragraph\n";
        let result = compact_tree(tree, false);
        assert!(result.contains("[ref=e1]"));
        assert!(result.contains("[ref=e2]"));
        assert!(result.contains("Hello"));
        assert!(result.contains("- status\n  - StaticText \"Saved successfully\""));
        assert!(!result.contains("- empty"));

        let interactive = compact_tree(tree, true);
        assert!(!interactive.contains("Saved successfully"));
        assert!(interactive.contains("[ref=e1]"));
        assert!(interactive.contains("Hello"));
    }

    #[test]
    fn test_compact_tree_keeps_only_direct_ancestors() {
        for (interactive, content) in [
            (false, "StaticText \"Saved successfully\""),
            (true, "button \"Save\" [ref=e1]"),
        ] {
            let tree = format!(
                "- main\n  - empty-section\n    - empty-paragraph\n  - section\n    - empty-paragraph\n    - paragraph\n      - {content}\n  - trailing-section\n- empty-root\n"
            );
            assert_eq!(
                compact_tree(&tree, interactive),
                format!("- main\n  - section\n    - paragraph\n      - {content}")
            );
        }
    }

    #[test]
    fn test_compact_tree_radio_checkbox() {
        // Radio/checkbox lines have attributes before ref (e.g. [checked=false, ref=e1])
        // so "ref=" appears without a leading "[" — compact_tree must still keep them.
        let tree = "- form\n  - radio \"Single unit\" [checked=false, ref=e1]\n  - checkbox \"I agree\" [checked=false, ref=e2]\n  - button \"Submit\" [ref=e3]\n";
        let result = compact_tree(tree, true);
        assert!(
            result.contains("radio \"Single unit\""),
            "radio should be kept"
        );
        assert!(
            result.contains("checkbox \"I agree\""),
            "checkbox should be kept"
        );
        assert!(
            result.contains("button \"Submit\""),
            "button should be kept"
        );
    }

    #[test]
    fn test_compact_tree_empty_interactive() {
        let result = compact_tree("- generic\n", true);
        assert_eq!(result, "(no interactive elements)");
    }

    /// The original ancestor walk, kept as the reference the stack in
    /// `compact_tree` has to reproduce exactly.
    fn compact_tree_reference(tree: &str, interactive: bool) -> String {
        let lines: Vec<&str> = tree.lines().collect();
        let mut keep = vec![false; lines.len()];
        for (i, line) in lines.iter().enumerate() {
            if line.contains("ref=")
                || line.contains(": ")
                || (!interactive && line.contains(" \""))
            {
                keep[i] = true;
                let mut my_indent = count_indent(line);
                for j in (0..i).rev() {
                    let ancestor_indent = count_indent(lines[j]);
                    if ancestor_indent < my_indent {
                        keep[j] = true;
                        my_indent = ancestor_indent;
                        if ancestor_indent == 0 {
                            break;
                        }
                    }
                }
            }
        }
        let output = lines
            .iter()
            .enumerate()
            .filter(|(i, _)| keep[*i])
            .map(|(_, line)| *line)
            .collect::<Vec<_>>()
            .join("\n");
        if output.trim().is_empty() && interactive {
            return "(no interactive elements)".to_string();
        }
        output
    }

    #[test]
    fn test_compact_tree_matches_reference_on_generated_trees() {
        // Successive rounds shuffle indents, empty branches and kept lines so
        // the stack path is compared against the ancestor walk on many shapes.
        let lines = [
            "- root",
            "  - empty-section",
            "    - empty-paragraph",
            "  - section",
            "    - paragraph",
            "      - StaticText \"Saved\"",
            "      - text: Hello",
            "        - deeper",
            "  - button \"Save\" [ref=e1]",
            "    - StaticText \"Save\"",
            "- other-root",
            "",
            "      - jump",
            "        - jump-deeper",
            "          - checkbox [checked=false, ref=e2]",
            "- trailing",
        ];
        for len in 0..=lines.len() {
            for rotation in 0..4 {
                let mut rotated: Vec<&str> = lines[..len].to_vec();
                rotated.rotate_left(rotation % len.max(1));
                let tree = format!("{}\n", rotated.join("\n"));
                for interactive in [false, true] {
                    assert_eq!(
                        compact_tree(&tree, interactive),
                        compact_tree_reference(&tree, interactive),
                        "tree: {tree:?}, interactive: {interactive}"
                    );
                }
            }
        }
    }

    #[test]
    fn test_count_indent() {
        assert_eq!(count_indent("- heading"), 0);
        assert_eq!(count_indent("  - link"), 1);
        assert_eq!(count_indent("    - text"), 2);
    }

    #[test]
    fn test_role_name_tracker() {
        let mut tracker = RoleNameTracker::new();
        assert_eq!(tracker.track("button", "Submit"), 0);
        assert_eq!(tracker.track("button", "Submit"), 1);
        assert_eq!(tracker.track("button", "Cancel"), 0);

        let dups = tracker.get_duplicates();
        assert!(dups.contains_key("button:Submit"));
        assert!(!dups.contains_key("button:Cancel"));
    }

    // -----------------------------------------------------------------------
    // promote_hidden_inputs
    // -----------------------------------------------------------------------

    fn make_node(role: &str, name: &str, backend_node_id: Option<i64>) -> TreeNode {
        let mut node = TreeNode::empty();
        node.role = role.to_string();
        node.name = name.to_string();
        node.backend_node_id = backend_node_id;
        node
    }

    fn make_cursor_info(
        hidden_kind: Option<HiddenInputKind>,
        hidden_checked: Option<&str>,
        text: &str,
    ) -> CursorElementInfo {
        CursorElementInfo {
            kind: "clickable".to_string(),
            hints: vec!["cursor:pointer".to_string()],
            text: text.to_string(),
            hidden_input_kind: hidden_kind,
            hidden_input_checked: hidden_checked.map(|s| s.to_string()),
            native: false,
        }
    }

    #[test]
    fn test_promote_label_with_hidden_radio() {
        let mut nodes = vec![
            make_node("LabelText", "", Some(1)),
            make_node("LabelText", "", Some(2)),
            make_node("button", "Submit", Some(3)),
        ];
        let mut cursor_elements = HashMap::new();
        cursor_elements.insert(
            1,
            make_cursor_info(Some(HiddenInputKind::Radio), Some("false"), "Option A"),
        );
        cursor_elements.insert(
            2,
            make_cursor_info(Some(HiddenInputKind::Radio), Some("true"), "Option B"),
        );

        promote_hidden_inputs(&mut nodes, &cursor_elements);

        assert_eq!(nodes[0].role, "radio");
        assert_eq!(nodes[0].name, "Option A");
        assert_eq!(nodes[0].checked, Some("false".to_string()));
        assert_eq!(nodes[1].role, "radio");
        assert_eq!(nodes[1].name, "Option B");
        assert_eq!(nodes[1].checked, Some("true".to_string()));
        // button should be untouched
        assert_eq!(nodes[2].role, "button");
    }

    #[test]
    fn test_promote_preserves_existing_name() {
        // If AX tree already has a name, don't overwrite with textContent
        let mut nodes = vec![make_node("LabelText", "AX Name", Some(1))];
        let mut cursor_elements = HashMap::new();
        cursor_elements.insert(
            1,
            make_cursor_info(Some(HiddenInputKind::Radio), Some("false"), "Text Content"),
        );

        promote_hidden_inputs(&mut nodes, &cursor_elements);

        assert_eq!(nodes[0].role, "radio");
        assert_eq!(nodes[0].name, "AX Name"); // preserved, not overwritten
    }

    #[test]
    fn test_promote_skips_without_hidden_input() {
        // Cursor-interactive label WITHOUT a hidden input should not be promoted
        let mut nodes = vec![make_node("LabelText", "", Some(1))];
        let mut cursor_elements = HashMap::new();
        cursor_elements.insert(1, make_cursor_info(None, None, "Click me"));

        promote_hidden_inputs(&mut nodes, &cursor_elements);

        assert_eq!(nodes[0].role, "LabelText"); // unchanged
    }

    #[test]
    fn test_iframe_display_name_is_presentation_only() {
        let mut fallback = make_node("Iframe", "", Some(1));
        fallback.display_name = Some("results".to_string());
        fallback.has_ref = true;
        fallback.ref_id = Some("e1".to_string());

        let mut accessible = make_node("Iframe", "Accessible frame", Some(2));
        accessible.display_name = Some("dom-name".to_string());
        accessible.has_ref = true;
        accessible.ref_id = Some("e2".to_string());

        let mut unnamed = make_node("Iframe", "", Some(3));
        unnamed.has_ref = true;
        unnamed.ref_id = Some("e3".to_string());

        let nodes = vec![fallback, accessible, unnamed];
        for interactive in [false, true] {
            let options = SnapshotOptions {
                interactive,
                ..SnapshotOptions::default()
            };
            let mut output = String::new();
            for idx in 0..nodes.len() {
                render_tree(&nodes, idx, 0, &mut output, &options);
            }

            assert!(output.contains("- Iframe \"results\" [ref=e1]"), "{output}");
            assert!(
                output.contains("- Iframe \"Accessible frame\" [ref=e2]"),
                "{output}"
            );
            assert!(output.contains("- Iframe [ref=e3]"), "{output}");
            assert!(!output.contains("dom-name"), "{output}");
        }
    }
}
