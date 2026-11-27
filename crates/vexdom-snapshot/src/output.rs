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

/// `content` from its `offset`-th char on. Past the end: `Err` with a line saying how long it is.
pub fn from_offset(content: &str, offset: usize) -> Result<&str, String> {
    if offset == 0 {
        return Ok(content);
    }
    match content.char_indices().nth(offset) {
        Some((at, _)) => Ok(&content[at..]),
        None => Err(format!(
            "[offset {offset} is past the end: the text is {} chars]",
            content.chars().count()
        )),
    }
}

/// `content` from char `offset`, cut to `max_chars` with a final line naming where to read on:
/// `\n[truncated: chars {offset}–{end} of {total}; read on with --offset {end}]`.
#[must_use]
pub fn page(content: &str, offset: usize, max_chars: Option<usize>) -> String {
    let rest = match from_offset(content, offset) {
        Ok(rest) => rest,
        Err(note) => return note,
    };
    let Some(max) = max_chars else {
        return rest.to_owned();
    };
    match rest.char_indices().nth(max) {
        None => rest.to_owned(),
        Some((at, _)) => {
            let end = offset + max;
            let total = offset + rest.chars().count();
            format!(
                "{}\n[truncated: chars {offset}–{end} of {total}; read on with --offset {end}]",
                &rest[..at]
            )
        }
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

    #[test]
    fn page_names_the_next_offset() {
        assert_eq!(
            page("abcdef", 0, Some(4)),
            "abcd\n[truncated: chars 0–4 of 6; read on with --offset 4]"
        );
        assert_eq!(page("abcdef", 4, Some(4)), "ef");
        assert_eq!(
            page("😀😀😀😀", 1, Some(2)),
            "😀😀\n[truncated: chars 1–3 of 4; read on with --offset 3]",
            "counts chars, not bytes"
        );
    }

    #[test]
    fn from_offset_past_the_end_says_how_long() {
        assert_eq!(
            from_offset("abc", 3),
            Err("[offset 3 is past the end: the text is 3 chars]".into())
        );
        assert_eq!(from_offset("", 0), Ok(""));
    }
}
