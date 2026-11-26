// Derived from agent-browser v0.45.0 (f0a9835), cli/src/native/element.rs (RefEntry, RefMap,
// parse_ref, normalize_ax_role).
// Copyright 2025 Vercel Inc. Licensed under the Apache License, Version 2.0.
// Modified by the vexdom authors: a ref's number is keyed by `RefKey` (frame, content-script
// document key, element id; docs/internals/decisions.md ADR-021) instead of CDP sessions, loaders and
// backend node ids, and the numbers of documents missing from a snapshot are forgotten;
// `RefEntry` carries that key instead of backend node, selector, frame and document fields;
// selector refs, exact-node refs and the document-binding API removed; clippy fixes.

use std::collections::HashMap;

/// Durable identity of an element: the browser frame, the content-script instance that walked the
/// document (one per document) and the element's id in that instance.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RefKey {
    pub frame_id: i64,
    pub doc_key: String,
    pub id: u64,
}

/// The element a ref of the latest snapshot points at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefEntry {
    pub key: RefKey,
    pub role: String,
    /// Accessible name as the page reported it (the printed name has NBSP and zero-width
    /// characters normalized).
    pub name: String,
    /// Position among the refs with the same role and name in the same frame, set only when there
    /// are several: such a ref must not be recovered by role and name.
    pub nth: Option<usize>,
}

/// The refs of one tab: the numbers given out so far and the refs of the latest snapshot.
#[derive(Debug, Clone)]
pub struct RefMap {
    map: HashMap<String, RefEntry>,
    keys: HashMap<RefKey, u64>,
    next_ref: u64,
}

impl Default for RefMap {
    fn default() -> Self {
        Self::new()
    }
}

impl RefMap {
    pub fn new() -> Self {
        Self {
            map: HashMap::new(),
            keys: HashMap::new(),
            next_ref: 1,
        }
    }

    pub(crate) fn add(&mut self, ref_id: String, entry: RefEntry) {
        self.map.insert(ref_id, entry);
    }

    /// The element behind `ref_id` (`"e5"`), if the latest snapshot showed that ref.
    #[must_use]
    pub fn get(&self, ref_id: &str) -> Option<&RefEntry> {
        self.map.get(ref_id)
    }

    /// The refs of the latest snapshot in numeric order (`e2` before `e10`).
    #[must_use]
    pub fn entries_sorted(&self) -> Vec<(String, RefEntry)> {
        let mut entries = self
            .map
            .iter()
            .map(|(ref_id, entry)| (ref_id.clone(), entry.clone()))
            .collect::<Vec<_>>();

        entries.sort_by_key(|(ref_id, _)| {
            ref_id
                .strip_prefix('e')
                .and_then(|n| n.parse::<u64>().ok())
                .unwrap_or(u64::MAX)
        });

        entries
    }

    /// Starts a snapshot of the documents `doc_keys`: drops the refs of the previous snapshot and
    /// the numbers of elements in any other document (it was navigated away or reloaded).
    pub(crate) fn begin_snapshot(&mut self, doc_keys: &[&str]) {
        self.map.clear();
        self.keys
            .retain(|key, _| doc_keys.contains(&key.doc_key.as_str()));
    }

    /// The number of an element's ref: the one it already has, else the next one never given out.
    pub(crate) fn number(&mut self, key: &RefKey) -> u64 {
        if let Some(&number) = self.keys.get(key) {
            return number;
        }
        let number = self.next_ref;
        self.next_ref += 1;
        self.keys.insert(key.clone(), number);
        number
    }
}

/// Reads a ref written as `@e5`, `ref=e5` or `e5`.
///
/// As in agent-browser, `@e` and `ref=e` read as `e`, and leading zeros are kept (`e007` is not
/// `e7`).
#[must_use]
pub fn parse_ref(input: &str) -> Option<String> {
    let trimmed = input.trim();

    if let Some(stripped) = trimmed.strip_prefix('@')
        && stripped.starts_with('e')
        && stripped[1..].chars().all(|c| c.is_ascii_digit())
    {
        return Some(stripped.to_string());
    }

    if let Some(stripped) = trimmed.strip_prefix("ref=")
        && stripped.starts_with('e')
        && stripped[1..].chars().all(|c| c.is_ascii_digit())
    {
        return Some(stripped.to_string());
    }

    if trimmed.starts_with('e')
        && trimmed.len() > 1
        && trimmed[1..].chars().all(|c| c.is_ascii_digit())
    {
        return Some(trimmed.to_string());
    }

    None
}

/// Use the same iframe role for snapshot refs and stale-ref recovery.
pub(crate) fn normalize_ax_role(role: String) -> String {
    if role == "IframePresentational" {
        "Iframe".to_string()
    } else {
        role
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(frame_id: i64, doc_key: &str, id: u64) -> RefKey {
        RefKey {
            frame_id,
            doc_key: doc_key.to_string(),
            id,
        }
    }

    fn entry(key: RefKey, role: &str, name: &str) -> RefEntry {
        RefEntry {
            key,
            role: role.to_string(),
            name: name.to_string(),
            nth: None,
        }
    }

    #[test]
    fn test_parse_ref_at_prefix() {
        assert_eq!(parse_ref("@e1"), Some("e1".to_string()));
        assert_eq!(parse_ref("@e123"), Some("e123".to_string()));
    }

    #[test]
    fn test_parse_ref_equals_prefix() {
        assert_eq!(parse_ref("ref=e1"), Some("e1".to_string()));
    }

    #[test]
    fn test_parse_ref_bare() {
        assert_eq!(parse_ref("e1"), Some("e1".to_string()));
        assert_eq!(parse_ref("e42"), Some("e42".to_string()));
    }

    #[test]
    fn test_parse_ref_invalid() {
        assert_eq!(parse_ref("button"), None);
        assert_eq!(parse_ref("e"), None);
        assert_eq!(parse_ref("1"), None);
        assert_eq!(parse_ref(""), None);
    }

    #[test]
    fn test_ref_map_basic() {
        let mut map = RefMap::new();
        map.add(
            "e1".to_string(),
            entry(key(0, "doc", 42), "button", "Submit"),
        );
        assert!(map.get("e1").is_some());
        assert_eq!(map.get("e1").unwrap().role, "button");
        assert!(map.get("e2").is_none());
    }

    #[test]
    fn test_ref_map_clear_preserves_monotonic_numbering() {
        let mut map = RefMap::new();
        assert_eq!(map.number(&key(0, "doc", 42)), 1);
        map.add(
            "e1".to_string(),
            entry(key(0, "doc", 42), "button", "Submit"),
        );

        map.begin_snapshot(&["doc"]);

        assert!(map.get("e1").is_none());
        assert_eq!(
            map.number(&key(0, "doc", 42)),
            1,
            "same element, same number"
        );
        assert_eq!(map.number(&key(0, "doc", 43)), 2);
    }

    #[test]
    fn numbers_are_reused_per_key_and_never_recycled() {
        let mut map = RefMap::new();
        let first = [key(0, "a", 1), key(0, "a", 2), key(7, "b", 1)];
        let numbers: Vec<u64> = first.iter().map(|k| map.number(k)).collect();
        assert_eq!(
            numbers,
            [1, 2, 3],
            "same id in another frame is another element"
        );

        // Document "a" is gone: its numbers are forgotten, but not given out again.
        map.begin_snapshot(&["b", "c"]);
        assert_eq!(map.number(&key(7, "b", 1)), 3);
        assert_eq!(map.number(&key(0, "a", 1)), 4);
        assert_eq!(map.number(&key(0, "c", 1)), 5);
    }
}
