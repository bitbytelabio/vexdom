// Derived from agent-browser v0.45.0 (f0a9835), cli/src/read.rs (html_to_markdownish and its
// helpers). Copyright 2025 Vercel Inc. Licensed under the Apache License, Version 2.0.
// Modified by the vexdom authors: public entry point `html_to_markdown`; fetch code removed;
// links as `[text](url)` against a base, table rows as `| a | b |`, a bullet whose text starts on
// its own line joined to it; entities decoded per text run, so a URL is never decoded twice.

//! HTML to readable markdown-ish text for `Control.read`: headings, paragraphs, lists, table
//! rows, `pre` fences, and link targets when asked; scripts, styles, `svg` and `head` dropped;
//! entities decoded.

use url::Url;

/// The page as markdown. With `links`, each link with a target prints as `[text](absolute url)`,
/// resolved against that base; without, as its text.
pub fn html_to_markdown(html: &str, links: Option<&Url>) -> String {
    html_to_markdownish(html, links)
}

/// The text written so far and what the open tags make of the next.
struct Converter<'base> {
    out: String,
    in_pre: bool,
    /// Base for link targets; `None` prints links as their text.
    links: Option<&'base Url>,
    /// Open `a` tags: where their text starts in `out` and their absolute target, or `None` for
    /// an anchor that prints as text.
    anchors: Vec<Option<(usize, String)>>,
    /// Open table cells: a block inside one is a space, so the row stays on one line.
    cell_depth: usize,
    /// `cell_depth` outside each open table. A row or cell resets the depth from it, so a cell
    /// whose `</td>` the HTML left out (which it may) does not stay open past its row or table.
    tables: Vec<usize>,
}

fn html_to_markdownish(html: &str, links: Option<&Url>) -> String {
    let stripped = strip_ignored_html_blocks(html);
    let mut converter = Converter {
        out: String::new(),
        in_pre: false,
        links,
        anchors: Vec::new(),
        cell_depth: 0,
        tables: Vec::new(),
    };
    let mut text = String::new();
    let mut chars = stripped.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch != '<' {
            text.push(ch);
            continue;
        }
        converter.out.push_str(&decode_html_entities(&text));
        text.clear();

        let mut tag = String::new();
        for next in chars.by_ref() {
            if next == '>' {
                break;
            }
            tag.push(next);
        }
        converter.tag(&tag);
    }
    converter.out.push_str(&decode_html_entities(&text));

    if converter.in_pre {
        converter.out.push_str("\n```");
    }
    normalize_markdownish(&converter.out)
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

/// Tags that break the text into blocks; inside a table cell each is one space instead.
const BLOCKS: &[&str] = &[
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "p",
    "div",
    "section",
    "article",
    "main",
    "header",
    "footer",
    "nav",
    "blockquote",
    "li",
    "ul",
    "ol",
    "br",
];

impl Converter<'_> {
    fn tag(&mut self, raw: &str) {
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
        if name == "a" {
            self.anchor(tag, closing);
            return;
        }
        let out = &mut self.out;
        if self.cell_depth > 0 && BLOCKS.contains(&name.as_str()) {
            out.push(' ');
            return;
        }
        // The open cells outside the innermost table.
        let outside = self.tables.last().copied().unwrap_or(0);

        if closing {
            match name.as_str() {
                "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "p" | "div" | "section" | "article"
                | "main" | "header" | "footer" | "nav" | "blockquote" | "li" | "ul" | "ol" => {
                    out.push_str("\n\n");
                }
                "table" => {
                    out.push_str("\n\n");
                    self.cell_depth = self.tables.pop().unwrap_or(0);
                }
                "tr" => {
                    out.push_str(" |\n");
                    self.cell_depth = outside;
                }
                "td" | "th" => self.cell_depth = outside,
                "pre" => {
                    out.push_str("\n```\n\n");
                    self.in_pre = false;
                }
                _ => {}
            }
            return;
        }

        match name.as_str() {
            "br" => out.push('\n'),
            "p" | "div" | "section" | "article" | "main" | "header" | "footer" | "nav"
            | "blockquote" | "ul" | "ol" => out.push_str("\n\n"),
            "table" => {
                out.push_str("\n\n");
                self.tables.push(self.cell_depth);
            }
            "tr" => {
                out.push('\n');
                self.cell_depth = outside;
            }
            "td" | "th" => {
                out.push_str(" | ");
                self.cell_depth = outside + 1;
            }
            "li" => out.push_str("\n- "),
            "h1" => out.push_str("\n\n# "),
            "h2" => out.push_str("\n\n## "),
            "h3" => out.push_str("\n\n### "),
            "h4" => out.push_str("\n\n#### "),
            "h5" => out.push_str("\n\n##### "),
            "h6" => out.push_str("\n\n###### "),
            "pre" => {
                out.push_str("\n\n```\n");
                self.in_pre = true;
            }
            _ => {}
        }
    }

    /// Opens or closes a link: `[text](target)` when targets print and the `href` goes
    /// somewhere, else nothing but its text. A link with no text prints nothing.
    fn anchor(&mut self, tag: &str, closing: bool) {
        let Some(base) = self.links.filter(|_| !self.in_pre) else {
            return;
        };
        if closing {
            if let Some(Some((start, target))) = self.anchors.pop() {
                if self.out[start..].trim().is_empty() {
                    self.out.truncate(start - 1);
                } else {
                    self.out.push_str("](");
                    self.out.push_str(&target);
                    self.out.push(')');
                }
            }
            return;
        }
        let target = parse_attr(tag, "href")
            .map(|href| decode_html_entities(&href))
            .filter(|href| {
                let href = href.trim();
                !href.is_empty()
                    && !href.starts_with('#')
                    && !href.to_ascii_lowercase().starts_with("javascript:")
            })
            .and_then(|href| base.join(href.trim()).ok());
        match target {
            Some(target) => {
                self.out.push('[');
                self.anchors.push(Some((self.out.len(), target.into())));
            }
            None => self.anchors.push(None),
        }
    }
}

/// The value of attribute `name` in a tag's source (`a href="…" …`): double-quoted,
/// single-quoted or bare. Attributes are read one by one, so text inside another attribute's
/// value (`data-x=" href='…'"`) is never taken for this one.
fn parse_attr(tag: &str, name: &str) -> Option<String> {
    let is_space = |c: char| c.is_whitespace() || c == '/';
    // Past the tag name.
    let mut rest = tag.trim_start_matches('/');
    rest = &rest[rest.find(is_space).unwrap_or(rest.len())..];
    loop {
        rest = rest.trim_start_matches(is_space);
        if rest.is_empty() {
            return None;
        }
        let end = rest
            .find(|c: char| is_space(c) || c == '=')
            .unwrap_or(rest.len());
        let attr = &rest[..end];
        rest = rest[end..].trim_start();
        let value = match rest.strip_prefix('=').map(str::trim_start) {
            Some(after) => match after.chars().next() {
                Some(quote @ ('"' | '\'')) => {
                    let body = &after[1..];
                    let close = body.find(quote).unwrap_or(body.len());
                    rest = body.get(close + 1..).unwrap_or("");
                    &body[..close]
                }
                _ => {
                    let close = after.find(char::is_whitespace).unwrap_or(after.len());
                    rest = &after[close..];
                    &after[..close]
                }
            },
            // Neither a name nor a value: nothing left to read.
            None if attr.is_empty() => return None,
            None => "",
        };
        if attr.eq_ignore_ascii_case(name) {
            return Some(value.to_string());
        }
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
    // A bullet whose text opened a block of its own (`<li><p>One`) waits for that text.
    let mut bullet = false;

    for raw in input.lines() {
        let mut line = if in_fence {
            raw.trim_end().to_string()
        } else {
            raw.split_whitespace().collect::<Vec<_>>().join(" ")
        };

        if line.trim() == "```" {
            in_fence = !in_fence;
        }

        if !in_fence && line == "-" {
            // A second bare bullet means the first had no text: it is dropped.
            bullet = true;
            continue;
        }
        if bullet {
            if line.trim().is_empty() {
                continue;
            }
            bullet = false;
            // Only plain text belongs to the bullet; a heading, list item, table row or fence
            // after an empty item stays what it is, and the empty item is dropped.
            let structural = ["#", "- ", "|", "```"]
                .iter()
                .any(|prefix| line.starts_with(prefix));
            if !structural {
                line = format!("- {line}");
            }
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
        let text = html_to_markdownish(html, None);
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
        let text = html_to_markdownish(html, None);
        assert!(text.contains("Home"));
        assert!(text.contains("# agent-browser"));
        assert!(text.contains("Browser automation CLI."));
    }

    #[test]
    fn links_print_their_absolute_target_only_with_a_base() {
        let html = r#"<p><a href="/x?a=1&amp;b=2">Go</a> <a href='#top'>Top</a> <a href=javascript:void(0)>JS</a> <a href="/y"> </a></p>"#;
        let base = Url::parse("https://s.test/p/").expect("a base URL");
        let linked = html_to_markdownish(html, Some(&base));
        assert_eq!(linked, "[Go](https://s.test/x?a=1&b=2) Top JS");
        assert_eq!(html_to_markdownish(html, None), "Go Top JS");
    }

    #[test]
    fn a_link_target_is_read_from_its_own_attribute_only() {
        let html = r#"<a data-x=" href='https://legit.test/'" href="https://evil.test/">E</a> <a class=x href=/bare title='a b'>B</a>"#;
        let base = Url::parse("https://s.test/").expect("a base URL");
        assert_eq!(
            html_to_markdownish(html, Some(&base)),
            "[E](https://evil.test/) [B](https://s.test/bare)"
        );
    }

    #[test]
    fn table_rows_stay_on_one_line() {
        let html =
            "<table><tr><th>A</th><th>B</th></tr><tr><td><p>1</p></td><td>2</td></tr></table>";
        let text = html_to_markdownish(html, None);
        assert!(text.contains("| A | B |"), "{text}");
        assert!(text.contains("| 1 | 2 |"), "{text}");
    }

    #[test]
    fn a_bullet_joins_text_that_opened_its_own_block() {
        assert_eq!(
            html_to_markdownish("<ul><li><p>One</p></li><li>Two</li></ul>", None),
            "- One\n\n- Two"
        );
    }

    #[test]
    fn an_empty_item_leaves_the_next_heading_alone() {
        let html = r#"<ul><li><a href="/cart"><img alt=""></a></li></ul><h1>Product</h1>"#;
        assert_eq!(html_to_markdownish(html, None), "# Product");
    }

    #[test]
    fn cells_left_open_close_with_their_row_and_table() {
        let html = "<table><tr><td>a<td>b<tr><td>c</table><h1>T</h1><ul><li>x</li></ul>";
        let text = html_to_markdownish(html, None);
        assert!(text.contains("| a | b"), "{text}");
        assert!(text.contains("# T"), "{text}");
        assert!(text.contains("- x"), "{text}");
    }
}
