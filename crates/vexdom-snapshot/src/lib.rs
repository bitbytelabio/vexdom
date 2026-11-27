//! Accessibility snapshots for vexdom: the content script's compact page tree ([`WireSnapshot`],
//! JSON) rendered as agent-browser-style text with element refs (`[ref=e5]`), the refs of each tab
//! ([`RefMap`]), and the last tree shown per tab ([`Baselines`]) so the next one can be sent as a
//! text delta.
//!
//! The tree, ref and render pipeline is vendored from agent-browser v0.45.0 (commit f0a9835,
//! Apache-2.0, Copyright 2025 Vercel Inc.); every derived file names its source and what changed.
//! Ref numbers follow docs/internals/decisions.md ADR-021: an element's number is keyed by its frame, its
//! document and its content-script id, and numbers are never given out twice in a tab.
//!
//! # Examples
//!
//! ```
//! use vexdom_snapshot::{Baselines, RefMap, SnapshotOptions, WireSnapshot, parse_ref, render};
//!
//! let snapshot: WireSnapshot = serde_json::from_str(
//!     r#"{"v": 1, "docKey": "doc-1", "url": "https://x.test/", "title": "X",
//!         "nodes": [{"id": 1, "r": "RootWebArea", "n": "X", "c": [1]},
//!                   {"id": 7, "r": "button", "n": "Save"}],
//!         "frames": []}"#,
//! )
//! .unwrap();
//! let (mut refs, mut baselines) = (RefMap::new(), Baselines::new(32));
//! let opts = SnapshotOptions { interactive: true, ..SnapshotOptions::default() };
//!
//! let tree = render(&snapshot, 0, &opts, &mut refs);
//! assert_eq!(tree, r#"- button "Save" [ref=e1]"#);
//! let reply = baselines.reply(5, &snapshot.url, &opts, &tree, false);
//! assert_eq!(reply, format!("snapshot r1\n{tree}"));
//!
//! // An action names the element by ref; the key says which frame and document hold it.
//! let entry = refs.get(&parse_ref("@e1").unwrap()).unwrap();
//! assert_eq!((entry.key.frame_id, entry.key.doc_key.as_str(), entry.key.id), (0, "doc-1", 7));
//! ```

mod ax;
mod delta;
pub mod diff;
mod output;
mod refs;
mod snapshot;
pub mod webmcp_text;
mod wire;

pub use delta::Baselines;
pub use output::{from_offset, page, truncate};
pub use refs::{RefEntry, RefKey, RefMap, parse_ref};
pub use snapshot::{SnapshotOptions, render};
pub use wire::{CursorInfo, FrameSlot, States, WireNode, WireSnapshot, WireValue};
