//! The result of the extension action `page.snapshot` (docs/research/content-script-ax.md §6): a
//! compact page tree the content script builds for one document, with the trees of child frames
//! filled in by the extension background.

use serde::{Deserialize, Deserializer, Serialize};

/// One document's tree as the content script returned it. `nodes[0]` is the root.
///
/// `viewport` and `stats` are accepted and ignored.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WireSnapshot {
    /// Schema version (1).
    pub v: u32,
    /// Random key of the content-script instance, one per document: refs issued for another key
    /// belong to a document that is gone.
    pub doc_key: String,
    pub url: String,
    #[serde(default)]
    pub title: String,
    pub nodes: Vec<WireNode>,
    /// Child frames that are not walked inline, each attached to an `Iframe` node of `nodes`.
    #[serde(default)]
    pub frames: Vec<FrameSlot>,
}

/// One accessibility node. Field names are short because a page has thousands of them.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct WireNode {
    /// Durable per-document element id. Only an element with an id can get a ref; text nodes
    /// have none.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<u64>,
    /// Role: an ARIA computed role, or `RootWebArea`, `StaticText` or `Iframe`.
    pub r: String,
    /// Accessible name; for `StaticText`, the text.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub n: Option<String>,
    /// Value: textbox text, selected option text, range value. Never sent for links or passwords.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub v: Option<WireValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub s: Option<States>,
    /// Child indexes into the same `nodes`, in flat-tree order.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub c: Option<Vec<usize>>,
    /// Present only for cursor-interactive elements.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub k: Option<CursorInfo>,
    /// Link target (`a.href`), printed with [`SnapshotOptions::urls`](crate::SnapshotOptions::urls).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub u: Option<String>,
    /// Hint for a control whose role and label are empty or shared with another control.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub h: Option<String>,
    /// The element lives in a shadow tree (the content script refuses role+name recovery for it).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sh: Option<bool>,
}

/// A node value: text, or a number such as a range's `valuenow`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum WireValue {
    Text(String),
    Number(serde_json::Number),
}

/// States the snapshot prints. `level`, `checked` and `expanded` print whenever present, so an
/// explicit `false` shows (`expanded=false`); `selected`, `disabled` and `required` print only when
/// true.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct States {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub level: Option<i64>,
    /// `"true"`, `"false"` or `"mixed"`; a JSON boolean is accepted too.
    #[serde(
        default,
        deserialize_with = "string_or_bool",
        skip_serializing_if = "Option::is_none"
    )]
    pub checked: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expanded: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub selected: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub required: Option<bool>,
    /// Carried for the page's benefit; the snapshot does not print it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub focused: Option<bool>,
}

/// Why an element without an interactive role behaves like a control. Field names are those of
/// agent-browser's cursor scan, so the extension can port it unchanged.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CursorInfo {
    pub has_cursor_pointer: bool,
    pub has_on_click: bool,
    pub has_tab_index: bool,
    pub is_editable: bool,
    /// The element's text, printed as its label when the accessible name is empty.
    pub text: String,
    /// A native tag (`a`, `summary`, …): ignored when the node already has an interactive role.
    pub is_native: bool,
    /// `"radio"` or `"checkbox"` when the element is a label wrapping a hidden input of that type.
    pub hidden_input_type: Option<String>,
    /// That hidden input's state: `"true"`, `"false"` or `"mixed"`.
    pub hidden_input_checked: Option<String>,
}

/// A child frame of a document, attached under the `Iframe` node `nodes[node]`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FrameSlot {
    /// Index of the frame's `Iframe` node in the parent's `nodes`.
    pub node: usize,
    /// Browser frame id; refs in the child tree are keyed by it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub frame_id: Option<i64>,
    /// Nonce the background uses to find the frame on Chrome.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub probe: Option<String>,
    /// The frame element's `title`, shown when the `Iframe` node has no accessible name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// The frame element's `name`, the fallback after `title`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default)]
    pub same_origin: bool,
    /// The child frame's own result, filled in by the extension background (nesting of at most 4
    /// frames is rendered).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snapshot: Option<Box<WireSnapshot>>,
}

/// Reads `checked` as agent-browser's AX reader does: a string as is, a boolean as its text.
fn string_or_bool<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Checked {
        Text(String),
        Flag(bool),
    }
    Ok(
        Option::<Checked>::deserialize(deserializer)?.map(|checked| match checked {
            Checked::Text(text) => text,
            Checked::Flag(flag) => flag.to_string(),
        }),
    )
}
