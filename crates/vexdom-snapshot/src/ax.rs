// Derived from agent-browser v0.45.0 (f0a9835), cli/src/native/cdp/types.rs (AXNode, AXValue,
// AXProperty); `from_wire` follows cli/src/native/bidi/snapshot.rs (`push_node`).
// Copyright 2025 Vercel Inc. Licensed under the Apache License, Version 2.0.
// Modified by the vexdom authors: the nodes are built in process from the content-script wire
// (serde, the string-or-int id readers and the never-read `description`, `ignored` and
// `AXValue.type` removed; node ids are indexes); `from_wire` added.

use std::collections::HashMap;

use serde_json::Value;

use crate::refs::RefKey;
use crate::snapshot::{CursorElementInfo, cursor_info_from_wire};
use crate::wire::{States, WireNode, WireSnapshot, WireValue};

/// Child frames nested deeper than this are not rendered.
const MAX_IFRAME_SNAPSHOT_DEPTH: usize = 4;

/// One node of the tree the vendored pipeline reads. Its id is its index in the node list.
pub(crate) struct AXNode {
    pub role: Option<AXValue>,
    pub name: Option<AXValue>,
    pub value: Option<AXValue>,
    pub properties: Option<Vec<AXProperty>>,
    /// Indexes of the children in the node list.
    pub child_ids: Vec<usize>,
    /// Key of the node in the pipeline's side maps (cursor info, hints, refs): its index, present
    /// only when the content script gave the element an id.
    pub backend_d_o_m_node_id: Option<i64>,
}

pub(crate) struct AXValue {
    pub value: Option<Value>,
}

pub(crate) struct AXProperty {
    pub name: &'static str,
    pub value: AXValue,
}

/// A document of a stitched snapshot.
struct Document<'wire> {
    frame_id: i64,
    doc_key: &'wire str,
}

/// The wire node behind an AX node.
struct Source<'wire> {
    node: &'wire WireNode,
    /// Index of the node's document in `WirePage::docs`.
    doc: usize,
}

/// A stitched snapshot: the top document followed by every child frame's document.
pub(crate) struct WirePage<'wire> {
    pub nodes: Vec<AXNode>,
    /// The wire node behind each entry of `nodes`.
    sources: Vec<Source<'wire>>,
    /// Top document first.
    docs: Vec<Document<'wire>>,
    /// Display-name fallback (frame `title`, then `name`) of each `Iframe` node with a frame slot,
    /// by node index.
    pub frame_names: Vec<(usize, &'wire str)>,
}

/// Expands the content script's compact tree into AX nodes (docs/research/content-script-ax.md
/// §6, "Daemon expansion"), with each child frame's tree under its `Iframe` node.
///
/// A node keeps at most one parent: a child index that is out of range, names the document root
/// or was already claimed by an earlier parent is dropped, so the tree has no cycles.
pub(crate) fn from_wire(snapshot: &WireSnapshot, frame_id: i64) -> WirePage<'_> {
    let mut page = WirePage {
        nodes: Vec::with_capacity(snapshot.nodes.len()),
        sources: Vec::with_capacity(snapshot.nodes.len()),
        docs: Vec::new(),
        frame_names: Vec::new(),
    };
    page.push_document(snapshot, frame_id, 0);
    page
}

impl<'wire> WirePage<'wire> {
    /// Appends a document's nodes and its child frames' documents, and returns the index of its
    /// root (`None` for a document without nodes).
    fn push_document(
        &mut self,
        snapshot: &'wire WireSnapshot,
        frame_id: i64,
        depth: usize,
    ) -> Option<usize> {
        let base = self.nodes.len();
        let len = snapshot.nodes.len();
        let doc = self.docs.len();
        self.docs.push(Document {
            frame_id,
            doc_key: &snapshot.doc_key,
        });
        let mut claimed = vec![false; len];
        for (index, node) in snapshot.nodes.iter().enumerate() {
            let children = node.c.as_deref().unwrap_or_default();
            let mut child_ids = Vec::with_capacity(children.len());
            for &child in children {
                if child != 0 && child < len && !claimed[child] {
                    claimed[child] = true;
                    child_ids.push(base + child);
                }
            }
            self.nodes.push(ax_node(node, child_ids, base + index));
            self.sources.push(Source { node, doc });
        }
        for slot in &snapshot.frames {
            if slot.node >= len {
                continue;
            }
            let host = base + slot.node;
            let fallback = [&slot.title, &slot.name]
                .into_iter()
                .find_map(|text| text.as_deref().map(str::trim).filter(|t| !t.is_empty()));
            if let Some(name) = fallback {
                self.frame_names.push((host, name));
            }
            if depth >= MAX_IFRAME_SNAPSHOT_DEPTH {
                continue;
            }
            // A child tree without its frame id could not be acted on by ref, so it is left out.
            let (Some(child), Some(child_frame)) = (slot.snapshot.as_deref(), slot.frame_id) else {
                continue;
            };
            if let Some(root) = self.push_document(child, child_frame, depth + 1) {
                self.nodes[host].child_ids.push(root);
            }
        }
        (len > 0).then_some(base)
    }

    /// Cursor-interactive elements by pipeline id.
    pub fn cursor_elements(&self) -> HashMap<i64, CursorElementInfo> {
        self.sources
            .iter()
            .enumerate()
            .filter_map(|(index, source)| {
                let info = source
                    .node
                    .k
                    .as_ref()
                    .filter(|_| source.node.id.is_some())?;
                Some((index as i64, cursor_info_from_wire(info)))
            })
            .collect()
    }

    /// Wire node behind a pipeline id.
    pub fn source(&self, backend_node_id: i64) -> &'wire WireNode {
        self.sources[backend_node_id as usize].node
    }

    /// Frame of a pipeline id.
    pub fn frame_of(&self, backend_node_id: i64) -> i64 {
        self.docs[self.sources[backend_node_id as usize].doc].frame_id
    }

    /// Durable identity of a pipeline id; `None` when the element has no content-script id.
    pub fn ref_key(&self, backend_node_id: i64) -> Option<RefKey> {
        let source = &self.sources[backend_node_id as usize];
        let document = &self.docs[source.doc];
        Some(RefKey {
            frame_id: document.frame_id,
            doc_key: document.doc_key.to_string(),
            id: source.node.id?,
        })
    }

    /// Keys of the documents in this snapshot.
    pub fn doc_keys(&self) -> impl Iterator<Item = &'wire str> + '_ {
        self.docs.iter().map(|document| document.doc_key)
    }
}

/// One wire node as an AX node: role, name and value as AX strings, states as properties (`level`
/// an integer, `checked` a string, the rest booleans). Like Firefox's mapping, a link carries no
/// value.
fn ax_node(node: &WireNode, child_ids: Vec<usize>, index: usize) -> AXNode {
    let text = |text: &str| AXValue {
        value: Some(Value::String(text.to_string())),
    };
    let value = node
        .v
        .as_ref()
        .filter(|_| node.r != "link")
        .map(|v| AXValue {
            value: Some(match v {
                WireValue::Text(text) => Value::String(text.clone()),
                WireValue::Number(number) => Value::Number(number.clone()),
            }),
        });
    AXNode {
        role: Some(text(&node.r)),
        name: node.n.as_deref().map(text),
        value,
        properties: node.s.as_ref().map(properties),
        child_ids,
        backend_d_o_m_node_id: node.id.map(|_| index as i64),
    }
}

fn properties(states: &States) -> Vec<AXProperty> {
    let property = |name, value| AXProperty {
        name,
        value: AXValue { value: Some(value) },
    };
    let flags = [
        ("expanded", states.expanded),
        ("selected", states.selected),
        ("disabled", states.disabled),
        ("required", states.required),
    ];
    let mut properties = Vec::new();
    if let Some(level) = states.level {
        properties.push(property("level", Value::from(level)));
    }
    if let Some(checked) = &states.checked {
        properties.push(property("checked", Value::String(checked.clone())));
    }
    for (name, flag) in flags {
        if let Some(flag) = flag {
            properties.push(property(name, Value::Bool(flag)));
        }
    }
    properties
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn wire(value: Value) -> WireSnapshot {
        serde_json::from_value(value).expect("a valid wire snapshot")
    }

    fn document(doc_key: &str, nodes: Value, frames: Value) -> Value {
        json!({"v": 1, "docKey": doc_key, "url": "https://x.test/", "title": "", "nodes": nodes, "frames": frames})
    }

    fn values(nodes: &[AXNode]) -> Vec<(Value, Vec<usize>, Option<i64>)> {
        nodes
            .iter()
            .map(|node| {
                let role = node.role.as_ref().and_then(|role| role.value.clone());
                (
                    role.unwrap_or(Value::Null),
                    node.child_ids.clone(),
                    node.backend_d_o_m_node_id,
                )
            })
            .collect()
    }

    #[test]
    fn expands_nodes_values_and_states() {
        let snapshot = wire(document(
            "doc",
            json!([
                {"id": 1, "r": "RootWebArea", "n": "T", "c": [1, 2, 3]},
                {"id": 4, "r": "slider", "n": "Volume", "v": 0.5,
                 "s": {"level": 2, "checked": true, "expanded": false, "disabled": true, "focused": true}},
                {"r": "StaticText", "n": "hi"},
                {"id": 9, "r": "link", "n": "Docs", "v": "https://x.test/docs"}
            ]),
            json!([]),
        ));
        let page = from_wire(&snapshot, 0);
        assert_eq!(
            values(&page.nodes),
            [
                (json!("RootWebArea"), vec![1, 2, 3], Some(0)),
                (json!("slider"), vec![], Some(1)),
                (json!("StaticText"), vec![], None),
                (json!("link"), vec![], Some(3)),
            ]
        );
        let slider = &page.nodes[1];
        assert_eq!(slider.value.as_ref().unwrap().value, Some(json!(0.5)));
        let properties: Vec<(&str, Option<Value>)> = slider
            .properties
            .iter()
            .flatten()
            .map(|p| (p.name, p.value.value.clone()))
            .collect();
        assert_eq!(
            properties,
            [
                ("level", Some(json!(2))),
                ("checked", Some(json!("true"))),
                ("expanded", Some(json!(false))),
                ("disabled", Some(json!(true))),
            ],
            "booleans stay booleans, checked becomes text, focused is not carried"
        );
        assert!(page.nodes[3].value.is_none(), "a link never shows a value");
        assert_eq!(
            page.ref_key(1),
            Some(RefKey {
                frame_id: 0,
                doc_key: "doc".into(),
                id: 4
            })
        );
    }

    #[test]
    fn a_node_keeps_one_parent_and_the_root_keeps_none() {
        let snapshot = wire(document(
            "doc",
            json!([
                {"r": "RootWebArea", "c": [1, 2, 2, 7]},
                {"r": "list", "c": [0, 1, 2, 3]},
                {"r": "listitem", "c": [1]},
                {"r": "listitem"}
            ]),
            json!([]),
        ));
        let page = from_wire(&snapshot, 0);
        let children: Vec<&[usize]> = page.nodes.iter().map(|n| n.child_ids.as_slice()).collect();
        assert_eq!(children, [&[1, 2][..], &[3], &[], &[]]);
    }

    #[test]
    fn stitches_child_frames_under_their_iframe_nodes() {
        let grandchild = document(
            "grand",
            json!([{"r": "RootWebArea", "c": [1]}, {"id": 3, "r": "button", "n": "Deep"}]),
            json!([]),
        );
        let child = document(
            "child",
            json!([{"id": 1, "r": "RootWebArea", "c": [1, 2]}, {"id": 2, "r": "button", "n": "In"}, {"id": 5, "r": "Iframe"}]),
            json!([{"node": 2, "frameId": 8, "sameOrigin": true, "snapshot": grandchild}]),
        );
        let snapshot = wire(document(
            "top",
            json!([{"id": 1, "r": "RootWebArea", "c": [1, 2]}, {"id": 2, "r": "Iframe"}, {"id": 3, "r": "Iframe"}]),
            json!([
                {"node": 1, "frameId": 7, "title": "Pay", "sameOrigin": false, "snapshot": child},
                // No frame id: the tree is left out, the display name is kept.
                {"node": 2, "name": "ads", "sameOrigin": false, "snapshot": document("lost", json!([{"r": "RootWebArea"}]), json!([]))},
                {"node": 9, "frameId": 3, "sameOrigin": false}
            ]),
        ));
        let page = from_wire(&snapshot, 0);
        assert_eq!(page.nodes.len(), 8);
        assert_eq!(
            page.nodes[1].child_ids,
            [3],
            "child root under the Iframe node"
        );
        assert_eq!(
            page.nodes[5].child_ids,
            [6],
            "grandchild root under the child's Iframe"
        );
        assert_eq!(page.nodes[3].child_ids, [4, 5]);
        assert_eq!(page.frame_names, [(1, "Pay"), (2, "ads")]);
        assert_eq!(
            page.doc_keys().collect::<Vec<_>>(),
            ["top", "child", "grand"]
        );
        assert_eq!(
            (page.frame_of(1), page.frame_of(4), page.frame_of(7)),
            (0, 7, 8)
        );
        assert_eq!(
            page.ref_key(7).map(|key| (key.doc_key, key.id)),
            Some(("grand".to_string(), 3))
        );
    }

    #[test]
    fn frames_nest_at_most_four_deep() {
        let mut frame = document("doc5", json!([{"r": "RootWebArea"}]), json!([]));
        for level in (0..5).rev() {
            frame = document(
                &format!("doc{level}"),
                json!([{"r": "RootWebArea", "c": [1]}, {"id": 1, "r": "Iframe"}]),
                json!([{"node": 1, "frameId": level + 1, "sameOrigin": true, "snapshot": frame}]),
            );
        }
        let snapshot = wire(frame);
        let page = from_wire(&snapshot, 0);
        assert_eq!(
            page.doc_keys().collect::<Vec<_>>(),
            ["doc0", "doc1", "doc2", "doc3", "doc4"]
        );
    }
}
