// Derived from agent-browser v0.45.0 (f0a9835), cli/src/output.rs (`truncate_if_needed`).
// Copyright 2025 Vercel Inc. Licensed under the Apache License, Version 2.0.
// Modified by the vexdom authors: public as `truncate`; the truncation notice is reworded for
// vexdom's `--max-output`.

/// Cuts `content` to `max_chars` Unicode scalar values and says so on a final line:
/// `\n[truncated: {shown} of {total} chars; raise --max-output]`. `None` keeps everything.
#[must_use]
pub fn truncate(content: &str, max_chars: Option<usize>) -> String {
    let Some(limit) = max_chars else {
        return content.to_string();
    };
    // Fast path: byte length is a lower bound on char count, so if the
    // byte length is within the limit the char count must be too.
    if content.len() <= limit {
        return content.to_string();
    }
    // Find the byte offset of the limit-th character.
    match content.char_indices().nth(limit).map(|(i, _)| i) {
        Some(byte_offset) => {
            let total_chars = content.chars().count();
            format!(
                "{}\n[truncated: {limit} of {total_chars} chars; raise --max-output]",
                &content[..byte_offset]
            )
        }
        // Content has fewer than `limit` chars despite more bytes
        None => content.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_with_boundaries_applies_max_output() {
        let rendered = truncate("abcdef", Some(5));

        assert_eq!(
            rendered,
            "abcde\n[truncated: 5 of 6 chars; raise --max-output]"
        );
    }

    #[test]
    fn truncation_counts_chars_not_bytes() {
        assert_eq!(
            truncate("héllo", Some(5)),
            "héllo",
            "6 bytes, 5 chars: no cut"
        );
        assert_eq!(
            truncate("😀😀😀", Some(2)),
            "😀😀\n[truncated: 2 of 3 chars; raise --max-output]"
        );
        assert_eq!(truncate("abcdef", None), "abcdef");
        assert_eq!(truncate("abc", Some(3)), "abc");
    }
}
