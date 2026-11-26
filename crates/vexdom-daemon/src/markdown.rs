// Derived from agent-browser v0.45.0 (f0a9835), cli/src/read.rs (html_to_markdownish and its
// helpers). Copyright 2025 Vercel Inc. Licensed under the Apache License, Version 2.0.
// Modified by the vexdom authors: public entry point `html_to_markdown`; fetch code removed.

//! HTML to readable markdown-ish text for `Control.read`: headings, paragraphs, lists, `pre`
//! fences; scripts, styles, `svg` and `head` dropped; entities decoded.

pub fn html_to_markdown(html: &str) -> String {
    html_to_markdownish(html)
}

fn html_to_markdownish(html: &str) -> String {
    let stripped = strip_ignored_html_blocks(html);
    let mut out = String::new();
    let mut chars = stripped.chars().peekable();
    let mut in_pre = false;

    while let Some(ch) = chars.next() {
        if ch != '<' {
            out.push(ch);
            continue;
        }

        let mut tag = String::new();
        for next in chars.by_ref() {
            if next == '>' {
                break;
            }
            tag.push(next);
        }
        handle_html_tag(&tag, &mut out, &mut in_pre);
    }

    let decoded = decode_html_entities(&out);
    if in_pre {
        normalize_markdownish(&format!("{}\n```", decoded))
    } else {
        normalize_markdownish(&decoded)
    }
}

fn strip_ignored_html_blocks(html: &str) -> String {
    let mut remaining = html.to_string();
    for tag in ["script", "style", "noscript", "svg", "head"] {
        remaining = strip_tag_block(&remaining, tag);
    }
    remaining
}

fn strip_tag_block(input: &str, tag: &str) -> String {
    let mut out = String::new();
    let mut cursor = 0;
    let lower = input.to_ascii_lowercase();
    let close = format!("</{}>", tag);

    while let Some(start) = find_open_tag(&lower, tag, cursor) {
        out.push_str(&input[cursor..start]);
        let after_start = lower[start..]
            .find('>')
            .map(|idx| start + idx + 1)
            .unwrap_or(input.len());
        if let Some(close_rel) = lower[after_start..].find(&close) {
            cursor = after_start + close_rel + close.len();
        } else {
            cursor = input.len();
        }
    }
    out.push_str(&input[cursor..]);
    out
}

fn find_open_tag(lower: &str, tag: &str, cursor: usize) -> Option<usize> {
    let needle = format!("<{}", tag);
    let mut search = cursor;
    while let Some(rel) = lower[search..].find(&needle) {
        let start = search + rel;
        let after_name = start + needle.len();
        let is_boundary = lower
            .as_bytes()
            .get(after_name)
            .map(|byte| matches!(byte, b' ' | b'\t' | b'\n' | b'\r' | b'\x0c' | b'/' | b'>'))
            .unwrap_or(true);
        if is_boundary {
            return Some(start);
        }
        search = after_name;
    }
    None
}

fn handle_html_tag(raw: &str, out: &mut String, in_pre: &mut bool) {
    let tag = raw.trim();
    if tag.is_empty() || tag.starts_with('!') || tag.starts_with('?') {
        return;
    }
    let closing = tag.starts_with('/');
    let name = tag
        .trim_start_matches('/')
        .split_whitespace()
        .next()
        .unwrap_or("")
        .trim_end_matches('/')
        .to_ascii_lowercase();

    if closing {
        match name.as_str() {
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "p" | "div" | "section" | "article"
            | "main" | "header" | "footer" | "nav" | "blockquote" | "li" | "tr" | "table"
            | "ul" | "ol" => out.push_str("\n\n"),
            "pre" => {
                out.push_str("\n```\n\n");
                *in_pre = false;
            }
            _ => {}
        }
        return;
    }

    match name.as_str() {
        "br" => out.push('\n'),
        "p" | "div" | "section" | "article" | "main" | "header" | "footer" | "nav"
        | "blockquote" | "table" | "tr" | "ul" | "ol" => out.push_str("\n\n"),
        "li" => out.push_str("\n- "),
        "h1" => out.push_str("\n\n# "),
        "h2" => out.push_str("\n\n## "),
        "h3" => out.push_str("\n\n### "),
        "h4" => out.push_str("\n\n#### "),
        "h5" => out.push_str("\n\n##### "),
        "h6" => out.push_str("\n\n###### "),
        "pre" => {
            out.push_str("\n\n```\n");
            *in_pre = true;
        }
        _ => {}
    }
}

fn decode_html_entities(input: &str) -> String {
    let mut out = String::new();
    let mut chars = input.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch != '&' {
            out.push(ch);
            continue;
        }

        let mut entity = String::new();
        while let Some(&next) = chars.peek() {
            if next == ';' {
                chars.next();
                break;
            }
            if entity.len() >= 16 || next.is_whitespace() || next == '&' {
                break;
            }
            entity.push(next);
            chars.next();
        }

        match decode_entity(&entity) {
            Some(decoded) => out.push(decoded),
            None => {
                out.push('&');
                out.push_str(&entity);
            }
        }
    }
    out
}

fn decode_entity(entity: &str) -> Option<char> {
    match entity {
        "amp" => Some('&'),
        "lt" => Some('<'),
        "gt" => Some('>'),
        "quot" => Some('"'),
        "apos" => Some('\''),
        "nbsp" => Some(' '),
        _ if entity.starts_with("#x") || entity.starts_with("#X") => {
            u32::from_str_radix(&entity[2..], 16)
                .ok()
                .and_then(char::from_u32)
        }
        _ if entity.starts_with('#') => entity[1..].parse::<u32>().ok().and_then(char::from_u32),
        _ => None,
    }
}

fn normalize_markdownish(input: &str) -> String {
    let mut lines = Vec::new();
    let mut blank_count = 0;
    let mut in_fence = false;

    for raw in input.lines() {
        let line = if in_fence {
            raw.trim_end().to_string()
        } else {
            raw.split_whitespace().collect::<Vec<_>>().join(" ")
        };

        if line.trim() == "```" {
            in_fence = !in_fence;
        }

        if line.trim().is_empty() {
            blank_count += 1;
            if blank_count <= 1 && !lines.is_empty() {
                lines.push(String::new());
            }
        } else {
            blank_count = 0;
            lines.push(line);
        }
    }

    lines.join("\n").trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn html_to_markdownish_extracts_readable_text() {
        let html = r#"
          <html><head><title>Skip</title><style>.x{}</style></head>
          <body><main><h1>Docs &amp; API</h1><p>Hello <strong>world</strong>.</p><ul><li>One</li><li>Two</li></ul></main></body></html>
        "#;
        let text = html_to_markdownish(html);
        assert!(text.contains("# Docs & API"));
        assert!(text.contains("Hello world."));
        assert!(text.contains("- One"));
        assert!(!text.contains(".x{}"));
    }

    #[test]
    fn html_to_markdownish_keeps_header_after_head() {
        let html = r#"
          <html><head><title>Skip</title></head><body>
          <header><a href="/">Home</a></header>
          <article><h1>agent-browser</h1><p>Browser automation CLI.</p></article>
          </body></html>
        "#;
        let text = html_to_markdownish(html);
        assert!(text.contains("Home"));
        assert!(text.contains("# agent-browser"));
        assert!(text.contains("Browser automation CLI."));
    }
}
