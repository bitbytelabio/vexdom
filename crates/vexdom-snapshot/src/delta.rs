// Derived from agent-browser v0.45.0 (f0a9835), cli/src/native/actions.rs (SnapshotRevision,
// snapshot_response, snapshot_delta_response).
// Copyright 2025 Vercel Inc. Licensed under the Apache License, Version 2.0.
// Modified by the vexdom authors: the per-session revisions of `DaemonState` became `Baselines`,
// keyed by tab with a caller-chosen cap, and every reply stores its tree as the baseline; replies
// are text instead of JSON and carry no `changes`/`refs` maps; options compare as
// `SnapshotOptions`; eviction ties go to the lowest tab id; a delta lists one `@` hunk per run of
// changed lines (Myers, via `similar`) instead of one splice between common prefix and suffix.

use std::collections::HashMap;
use std::fmt::Write;
use std::time::{Duration, Instant};

use similar::{Algorithm, DiffTag};

use crate::snapshot::SnapshotOptions;

/// How long a delta may spend diffing; past it the diff gets coarser, never wrong.
const DIFF_DEADLINE: Duration = Duration::from_millis(50);

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
    /// - `snapshot r{rev} (delta from r{base}):` followed by one or more hunks, each on its own
    ///   line: `@ lines {a}-{b} replaced by:\n{lines}`, `@ inserted after line {n}:\n{lines}` or
    ///   `@ lines {a}-{b} removed`, with 1-based line numbers of the baseline tree.
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

    // Line hunks reproduce the new tree exactly, including changes no ref can describe (a
    // checkbox gaining `[checked]`). One hunk per run of changed lines keeps two distant edits
    // from resending everything between them.
    let before_lines: Vec<&str> = previous.tree.split('\n').collect();
    let after_lines: Vec<&str> = tree.split('\n').collect();
    let deadline = Instant::now() + DIFF_DEADLINE;
    let ops = similar::capture_diff_slices_deadline(
        Algorithm::Myers,
        &before_lines,
        &after_lines,
        Some(deadline),
    );

    let mut delta = format!("snapshot r{revision} (delta from r{}):", previous.revision);
    for group in similar::group_diff_ops(ops, 0) {
        // A group is one run of changed ops in order, so its first and last span the hunk.
        let mut changed = group.iter().filter(|op| op.tag() != DiffTag::Equal);
        let Some(head) = changed.next() else {
            continue;
        };
        let tail = changed.next_back().unwrap_or(head);
        let old = head.old_range().start..tail.old_range().end;
        let new = head.new_range().start..tail.new_range().end;
        let (first, last) = (old.start + 1, old.end);
        // Writing to a String cannot fail.
        let _ = if new.is_empty() {
            write!(delta, "\n@ lines {first}-{last} removed")
        } else if old.is_empty() {
            write!(delta, "\n@ inserted after line {}:", old.start)
        } else {
            write!(delta, "\n@ lines {first}-{last} replaced by:")
        };
        for line in &after_lines[new] {
            delta.push('\n');
            delta.push_str(line);
        }
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
        let mut lines = reply.split('\n');
        let head = lines.next().expect("a header");
        assert!(head.contains("(delta from r"), "{reply}");
        // Each hunk: its instruction and the new lines that follow it, before the next `@`.
        let mut hunks: Vec<(&str, Vec<&str>)> = Vec::new();
        for line in lines {
            match line.strip_prefix("@ ") {
                Some(instruction) => hunks.push((instruction, Vec::new())),
                None => hunks.last_mut().expect("a hunk").1.push(line),
            }
        }
        let range = |spec: &str| {
            let (a, b) = spec.split_once('-').expect("a line range");
            let (a, b): (usize, usize) = (
                a.parse().expect("a line number"),
                b.parse().expect("a line number"),
            );
            a - 1..b
        };
        let mut tree: Vec<&str> = before.split('\n').collect();
        // Hunks number the baseline's lines, so applying them bottom up keeps numbers valid.
        for (instruction, new_lines) in hunks.into_iter().rev() {
            if let Some(spec) = instruction.strip_prefix("inserted after line ") {
                let at: usize = spec.trim_end_matches(':').parse().expect("a line number");
                tree.splice(at..at, new_lines);
            } else if let Some(spec) = instruction.strip_suffix(" replaced by:") {
                tree.splice(range(spec.trim_start_matches("lines ")), new_lines);
            } else {
                let spec = instruction.strip_suffix(" removed").expect("a removal");
                tree.splice(range(spec.trim_start_matches("lines ")), []);
            }
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
                "snapshot r2 (delta from r1):\n@ lines 41-42 replaced by:\n- button \"Saved\" [ref=e1]\n- checkbox \"Agree\" [ref=e2] [checked]",
            ),
            (
                &checked_only,
                "snapshot r2 (delta from r1):\n@ lines 42-42 replaced by:\n- checkbox \"Agree\" [ref=e2] [checked]",
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
            "snapshot r2 (delta from r1):\n@ lines 2-2 replaced by:\nnew"
        );

        let removed = snapshot_delta_response(&previous, 2, &"x".repeat(1000));
        assert_eq!(removed, "snapshot r2 (delta from r1):\n@ lines 2-2 removed");
        assert_eq!(apply(&before, &removed), "x".repeat(1000));

        let added = format!("{}\nold\nmore", "x".repeat(1000));
        let inserted = snapshot_delta_response(&previous, 2, &added);
        assert_eq!(
            inserted,
            "snapshot r2 (delta from r1):\n@ inserted after line 2:\nmore"
        );
        assert_eq!(apply(&before, &inserted), added);

        let top = format!("top\n{before}");
        let at_top = snapshot_delta_response(&previous, 2, &top);
        assert_eq!(
            at_top,
            "snapshot r2 (delta from r1):\n@ inserted after line 0:\ntop"
        );
        assert_eq!(apply(&before, &at_top), top);
    }

    #[test]
    fn two_distant_changes_make_two_hunks() {
        let before: Vec<String> = (1..=200)
            .map(|i| format!("- link \"Item {i}\" [ref=e{i}]"))
            .collect();
        let mut after = before.clone();
        after[2] = "- link \"Item 3 (new)\" [ref=e3]".to_string();
        after[189] = "- link \"Item 190 (new)\" [ref=e190]".to_string();
        let (before, after) = (before.join("\n"), after.join("\n"));

        let reply = snapshot_delta_response(&revision(1, &before), 2, &after);

        assert_eq!(
            reply,
            "snapshot r2 (delta from r1):\n@ lines 3-3 replaced by:\n- link \"Item 3 (new)\" [ref=e3]\n@ lines 190-190 replaced by:\n- link \"Item 190 (new)\" [ref=e190]"
        );
        // The old single splice resent lines 3-190, everything between the two changes.
        let splice: usize = after
            .split('\n')
            .skip(2)
            .take(188)
            .map(|l| l.len() + 1)
            .sum();
        assert!(reply.len() < splice, "{} >= {splice}", reply.len());
        assert_eq!(apply(&before, &reply), after);
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
            "snapshot r3 (delta from r2):\n@ lines 21-21 replaced by:\n- button \"Saved\" [ref=e1]"
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
