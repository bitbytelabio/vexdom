// Derived from agent-browser v0.45.0 (f0a9835), cli/src/native/actions.rs (SnapshotRevision,
// snapshot_response, snapshot_delta_response).
// Copyright 2025 Vercel Inc. Licensed under the Apache License, Version 2.0.
// Modified by the vexdom authors: the per-session revisions of `DaemonState` became `Baselines`,
// keyed by tab with a caller-chosen cap, and every reply stores its tree as the baseline; replies
// are text instead of JSON and carry no `changes`/`refs` maps; options compare as
// `SnapshotOptions`; eviction ties go to the lowest tab id.

use std::collections::HashMap;
use std::fmt::Write;

use crate::snapshot::SnapshotOptions;

struct SnapshotRevision {
    revision: u64,
    url: String,
    options: SnapshotOptions,
    tree: String,
}

/// The last snapshot tree shown for each tab, so the next one can be sent as a change.
pub struct Baselines {
    cap: usize,
    tabs: HashMap<i64, SnapshotRevision>,
}

impl Baselines {
    /// Keeps baselines for at most `cap` tabs (at least one); a new tab past the cap evicts the
    /// tab whose baseline has the lowest revision.
    pub fn new(cap: usize) -> Self {
        Self {
            cap,
            tabs: HashMap::new(),
        }
    }

    /// The text to send for showing `tree` on `tab`, which then becomes the tab's baseline.
    ///
    /// Revisions count the trees shown per tab, from 1. The reply is one of
    /// - `snapshot r{rev}: unchanged`;
    /// - `snapshot r{rev}\n{tree}` when the tab has no baseline, its URL or options changed,
    ///   `force_full` is set, or the change would take at least 70 % of the tree's bytes;
    /// - `snapshot r{rev} (delta from r{base}): ` followed by `lines {a}-{b} replaced by:\n{lines}`,
    ///   `inserted after line {n}:\n{lines}` or `lines {a}-{b} removed`, with 1-based line numbers
    ///   of the baseline tree.
    #[must_use]
    pub fn reply(
        &mut self,
        tab: i64,
        url: &str,
        opts: &SnapshotOptions,
        tree: &str,
        force_full: bool,
    ) -> String {
        let previous = self.tabs.get(&tab);
        let revision = previous.map_or(1, |entry| entry.revision + 1);
        let response = match previous {
            Some(previous) if !force_full && previous.url == url && previous.options == *opts => {
                snapshot_delta_response(previous, revision, tree)
            }
            _ => full_response(revision, tree),
        };

        if let Some(entry) = self.tabs.get_mut(&tab) {
            entry.revision = revision;
            entry.url.clear();
            entry.url.push_str(url);
            entry.options.clone_from(opts);
            entry.tree.clear();
            entry.tree.push_str(tree);
            return response;
        }
        if self.tabs.len() >= self.cap
            && let Some(oldest_key) = self
                .tabs
                .iter()
                .min_by_key(|&(&tab, entry)| (entry.revision, tab))
                .map(|(&tab, _)| tab)
        {
            self.tabs.remove(&oldest_key);
        }
        self.tabs.insert(
            tab,
            SnapshotRevision {
                revision,
                url: url.to_string(),
                options: opts.clone(),
                tree: tree.to_string(),
            },
        );
        response
    }

    /// Drops the baseline of a tab (closed, or its refs were reset), so its next reply is full.
    pub fn forget(&mut self, tab: i64) {
        self.tabs.remove(&tab);
    }
}

fn full_response(revision: u64, tree: &str) -> String {
    format!("snapshot r{revision}\n{tree}")
}

fn snapshot_delta_response(previous: &SnapshotRevision, revision: u64, tree: &str) -> String {
    if previous.tree == tree {
        return format!("snapshot r{revision}: unchanged");
    }

    // One splice between the common prefix and the common suffix reproduces the new tree
    // exactly, including changes no ref can describe (a checkbox gaining `[checked]`).
    let before_lines: Vec<&str> = previous.tree.split('\n').collect();
    let after_lines: Vec<&str> = tree.split('\n').collect();
    let prefix = before_lines
        .iter()
        .zip(&after_lines)
        .take_while(|(a, b)| a == b)
        .count();
    let suffix = before_lines[prefix..]
        .iter()
        .rev()
        .zip(after_lines[prefix..].iter().rev())
        .take_while(|(a, b)| a == b)
        .count();
    let start_line = prefix;
    let delete_count = before_lines.len() - prefix - suffix;
    let lines = &after_lines[prefix..after_lines.len() - suffix];

    let mut delta = format!("snapshot r{revision} (delta from r{}): ", previous.revision);
    let (first, last) = (start_line + 1, start_line + delete_count);
    // Writing to a String cannot fail.
    let _ = if lines.is_empty() {
        write!(delta, "lines {first}-{last} removed")
    } else if delete_count == 0 {
        write!(delta, "inserted after line {start_line}:")
    } else {
        write!(delta, "lines {first}-{last} replaced by:")
    };
    for line in lines {
        delta.push('\n');
        delta.push_str(line);
    }

    if delta.len() >= tree.len().saturating_mul(7) / 10 {
        full_response(revision, tree)
    } else {
        delta
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn revision(revision: u64, tree: &str) -> SnapshotRevision {
        SnapshotRevision {
            revision,
            url: "https://example.com".to_string(),
            options: SnapshotOptions::default(),
            tree: tree.to_string(),
        }
    }

    /// Applies a reply to the tree it was computed against, the way a reader of the text would.
    fn apply(before: &str, reply: &str) -> String {
        let (head, rest) = reply.split_once("): ").expect("a delta reply");
        assert!(head.contains("(delta from r"), "{reply}");
        let (instruction, lines) = rest.split_once('\n').unwrap_or((rest, ""));
        let mut tree: Vec<&str> = before.split('\n').collect();
        let range = |spec: &str| {
            let (a, b) = spec.split_once('-').expect("a line range");
            let (a, b): (usize, usize) = (a.parse().unwrap(), b.parse().unwrap());
            a - 1..b
        };
        let new_lines = lines.split('\n');
        if let Some(spec) = instruction.strip_prefix("inserted after line ") {
            let at: usize = spec.trim_end_matches(':').parse().unwrap();
            tree.splice(at..at, new_lines);
        } else if let Some(spec) = instruction.strip_suffix(" replaced by:") {
            tree.splice(range(spec.trim_start_matches("lines ")), new_lines);
        } else {
            let spec = instruction.strip_suffix(" removed").expect("a removal");
            tree.splice(range(spec.trim_start_matches("lines ")), []);
        }
        tree.join("\n")
    }

    #[test]
    fn snapshot_delta_tree_splice_preserves_changes_missing_from_ref_metadata() {
        let padding = "- button \"Unchanged\" [ref=e99]\n".repeat(40);
        let before = format!("{padding}- button \"Save\" [ref=e1]\n- checkbox \"Agree\" [ref=e2]");
        let after = format!(
            "{padding}- button \"Saved\" [ref=e1]\n- checkbox \"Agree\" [ref=e2] [checked]"
        );
        let previous = revision(1, &before);
        let checked_only = before.replace("[ref=e2]", "[ref=e2] [checked]");
        for (current, expected) in [
            (
                &after,
                "snapshot r2 (delta from r1): lines 41-42 replaced by:\n- button \"Saved\" [ref=e1]\n- checkbox \"Agree\" [ref=e2] [checked]",
            ),
            (
                &checked_only,
                "snapshot r2 (delta from r1): lines 42-42 replaced by:\n- checkbox \"Agree\" [ref=e2] [checked]",
            ),
        ] {
            let reply = snapshot_delta_response(&previous, 2, current);
            assert_eq!(reply, expected);
            assert_eq!(&apply(&before, &reply), current);
        }
    }

    #[test]
    fn test_snapshot_delta_unchanged_is_tiny() {
        let previous = revision(4, "- button \"Save\" [ref=e1]");
        let reply = snapshot_delta_response(&previous, 5, &previous.tree);
        assert_eq!(reply, "snapshot r5: unchanged");
    }

    #[test]
    fn test_snapshot_delta_reports_replacements_and_removals() {
        let before = format!("{}\nold", "x".repeat(1000));
        let previous = revision(1, &before);
        let replaced = snapshot_delta_response(&previous, 2, &format!("{}\nnew", "x".repeat(1000)));
        assert_eq!(
            replaced,
            "snapshot r2 (delta from r1): lines 2-2 replaced by:\nnew"
        );

        let removed = snapshot_delta_response(&previous, 2, &"x".repeat(1000));
        assert_eq!(removed, "snapshot r2 (delta from r1): lines 2-2 removed");
        assert_eq!(apply(&before, &removed), "x".repeat(1000));

        let added = format!("{}\nold\nmore", "x".repeat(1000));
        let inserted = snapshot_delta_response(&previous, 2, &added);
        assert_eq!(
            inserted,
            "snapshot r2 (delta from r1): inserted after line 2:\nmore"
        );
        assert_eq!(apply(&before, &inserted), added);

        let top = format!("top\n{before}");
        let at_top = snapshot_delta_response(&previous, 2, &top);
        assert_eq!(
            at_top,
            "snapshot r2 (delta from r1): inserted after line 0:\ntop"
        );
        assert_eq!(apply(&before, &at_top), top);
    }

    #[test]
    fn large_changes_are_sent_in_full() {
        let previous = revision(1, "- button \"Save\" [ref=e1]");
        let tree = "- button \"Saved\" [ref=e1]";
        assert_eq!(
            snapshot_delta_response(&previous, 2, tree),
            format!("snapshot r2\n{tree}")
        );
    }

    #[test]
    fn baselines_send_full_then_changes() {
        let mut baselines = Baselines::new(32);
        let opts = SnapshotOptions::default();
        let url = "https://example.com/";
        let tree = format!(
            "{}- button \"Save\" [ref=e1]",
            "- link \"x\" [ref=e2]\n".repeat(20)
        );
        assert_eq!(
            baselines.reply(1, url, &opts, &tree, false),
            format!("snapshot r1\n{tree}")
        );
        assert_eq!(
            baselines.reply(1, url, &opts, &tree, false),
            "snapshot r2: unchanged"
        );
        let saved = tree.replace("\"Save\"", "\"Saved\"");
        assert_eq!(
            baselines.reply(1, url, &opts, &saved, false),
            "snapshot r3 (delta from r2): lines 21-21 replaced by:\n- button \"Saved\" [ref=e1]"
        );
        // Every reply is the new baseline, a full one included.
        assert_eq!(
            baselines.reply(1, url, &opts, &tree, true),
            format!("snapshot r4\n{tree}")
        );
        assert_eq!(
            baselines.reply(1, url, &opts, &tree, false),
            "snapshot r5: unchanged"
        );
    }

    #[test]
    fn url_or_option_changes_force_full() {
        let mut baselines = Baselines::new(32);
        let opts = SnapshotOptions::default();
        let tree = "- button \"Save\" [ref=e1]";
        let _ = baselines.reply(1, "https://a.test/", &opts, tree, false);
        assert_eq!(
            baselines.reply(1, "https://a.test/#x", &opts, tree, false),
            format!("snapshot r2\n{tree}")
        );
        let interactive = SnapshotOptions {
            interactive: true,
            ..SnapshotOptions::default()
        };
        assert_eq!(
            baselines.reply(1, "https://a.test/#x", &interactive, tree, false),
            format!("snapshot r3\n{tree}")
        );
        assert_eq!(
            baselines.reply(1, "https://a.test/#x", &interactive, tree, false),
            "snapshot r4: unchanged"
        );
    }

    #[test]
    fn eviction_drops_the_lowest_revision_and_forget_resets() {
        let mut baselines = Baselines::new(2);
        let opts = SnapshotOptions::default();
        let tree = "- button \"Save\" [ref=e1]";
        for _ in 0..3 {
            let _ = baselines.reply(10, "u", &opts, tree, false);
        }
        let _ = baselines.reply(20, "u", &opts, tree, false);
        // Tab 30 evicts tab 20 (revision 1), not tab 10 (revision 3), though 10 is older.
        let _ = baselines.reply(30, "u", &opts, tree, false);
        assert_eq!(
            baselines.reply(10, "u", &opts, tree, false),
            "snapshot r4: unchanged"
        );
        assert_eq!(
            baselines.reply(20, "u", &opts, tree, false),
            format!("snapshot r1\n{tree}")
        );

        baselines.forget(10);
        assert_eq!(
            baselines.reply(10, "u", &opts, tree, false),
            format!("snapshot r1\n{tree}")
        );
    }
}
