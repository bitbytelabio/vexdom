//! The tail of the Bridge's log (`~/.vexdom/daemon.log`) for Settings → Logs: the last
//! [`KEEP`] lines, each poll reading only what was appended since the last one. A file that shrank
//! (cleared, rotated) is read again from its start. Kept apart from GPUI so it is tested without a
//! window.

use std::collections::VecDeque;
use std::fs::File;
use std::io::{Read as _, Seek as _, SeekFrom};
use std::path::Path;

/// Lines kept.
pub const KEEP: usize = 400;

/// The most a first read takes from the end of a long log: room for [`KEEP`] long lines.
const FIRST_READ: u64 = 256 * 1024;

/// How close to its end a scrolled list counts as at the end, in pixels.
const BOTTOM_SLACK: f32 = 4.0;

#[derive(Debug, Default)]
pub struct LogTail {
    /// Bytes of the file consumed: always just past a newline.
    read_to: u64,
    lines: VecDeque<String>,
}

impl LogTail {
    pub fn lines(&self) -> impl Iterator<Item = &str> {
        self.lines.iter().map(String::as_str)
    }

    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    /// Reads what `path` gained since the last poll; `true` when the lines changed. A missing file
    /// reads as empty.
    pub fn poll(&mut self, path: &Path) -> bool {
        let Ok(mut file) = File::open(path) else {
            return self.clear();
        };
        let Ok(len) = file.metadata().map(|meta| meta.len()) else {
            return false;
        };
        let mut changed = false;
        if len < self.read_to {
            changed = self.clear();
        }
        if len == self.read_to {
            return changed;
        }
        let start = if self.read_to == 0 {
            len.saturating_sub(FIRST_READ)
        } else {
            self.read_to
        };
        let mut bytes = Vec::with_capacity(usize::try_from(len - start).unwrap_or(0));
        if file.seek(SeekFrom::Start(start)).is_err()
            || file.take(len - start).read_to_end(&mut bytes).is_err()
        {
            return changed;
        }
        // A line still being written waits for its newline, so no line (or character) is split.
        let Some(end) = bytes.iter().rposition(|byte| *byte == b'\n') else {
            return changed;
        };
        let mut complete = &bytes[..end];
        if start > 0 && self.read_to == 0 {
            // Started mid-file: the first line is cut, drop it.
            complete = match complete.iter().position(|byte| *byte == b'\n') {
                Some(cut) => &complete[cut + 1..],
                None => &[],
            };
        }
        self.read_to = start + end as u64 + 1;
        for line in complete.split(|byte| *byte == b'\n') {
            if self.lines.len() == KEEP {
                self.lines.pop_front();
            }
            self.lines
                .push_back(String::from_utf8_lossy(line).into_owned());
        }
        true
    }

    fn clear(&mut self) -> bool {
        let had = self.read_to > 0 || !self.lines.is_empty();
        *self = Self::default();
        had
    }
}

/// Whether a list scrolled down by `scrolled` of at most `max` pixels shows its last line, so
/// new lines keep it there; scrolled up, it stays where the reader left it.
pub fn at_bottom(scrolled: f32, max: f32) -> bool {
    max - scrolled <= BOTTOM_SLACK
}

#[cfg(test)]
mod tests {
    use std::io::Write as _;
    use std::path::PathBuf;

    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("vexdom-desktop-log-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("a scratch folder");
        dir.join("daemon.log")
    }

    fn append(path: &Path, text: &str) {
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .and_then(|mut file| file.write_all(text.as_bytes()))
            .expect("appended");
    }

    fn shown(tail: &LogTail) -> Vec<&str> {
        tail.lines().collect()
    }

    #[test]
    fn appended_lines_arrive_and_a_half_line_waits() {
        let log = scratch("append");
        let mut tail = LogTail::default();
        assert!(!tail.poll(&log));
        append(&log, "one\ntwo\nthr");
        assert!(tail.poll(&log));
        assert_eq!(shown(&tail), ["one", "two"]);
        assert!(!tail.poll(&log));
        append(&log, "ee\n");
        assert!(tail.poll(&log));
        assert_eq!(shown(&tail), ["one", "two", "three"]);
        let _ = std::fs::remove_dir_all(log.parent().expect("a folder"));
    }

    #[test]
    fn a_file_that_shrank_is_read_again_from_its_start() {
        let log = scratch("shrink");
        let mut tail = LogTail::default();
        append(&log, "old one\nold two\nold three\n");
        assert!(tail.poll(&log));
        std::fs::write(&log, "new\n").expect("truncated");
        assert!(tail.poll(&log));
        assert_eq!(shown(&tail), ["new"]);
        std::fs::remove_file(&log).expect("removed");
        assert!(tail.poll(&log));
        assert!(tail.is_empty());
        let _ = std::fs::remove_dir_all(log.parent().expect("a folder"));
    }

    #[test]
    fn only_the_last_lines_are_kept_and_a_long_log_is_read_from_near_its_end() {
        let log = scratch("keep");
        let mut tail = LogTail::default();
        let text: String = (0..1000).map(|n| format!("line {n}\n")).collect();
        append(&log, &text);
        assert!(tail.poll(&log));
        assert_eq!(tail.lines.len(), KEEP);
        assert_eq!(tail.lines().next(), Some("line 600"));
        assert_eq!(tail.lines().last(), Some("line 999"));

        let long = scratch("long");
        let line = format!("{}\n", "x".repeat(99));
        append(&long, &line.repeat(5000));
        let mut far = LogTail::default();
        assert!(far.poll(&long));
        // Every line read is whole: the cut one at the window's start was dropped.
        assert!(far.lines().all(|shown| shown.len() == 99));
        assert_eq!(far.lines.len(), KEEP);
        let _ = std::fs::remove_dir_all(log.parent().expect("a folder"));
        let _ = std::fs::remove_dir_all(long.parent().expect("a folder"));
    }

    #[test]
    fn the_list_follows_new_lines_only_from_its_end() {
        assert!(at_bottom(600.0, 600.0));
        assert!(at_bottom(597.0, 600.0));
        assert!(!at_bottom(400.0, 600.0));
        assert!(at_bottom(0.0, 0.0));
    }
}
