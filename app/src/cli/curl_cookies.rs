//! `vexdom cookies set --curl <file>`: the cookies of a request copied from DevTools, a bare
//! `Cookie:` header or a JSON list. Ported from agent-browser (`cli/src/commands.rs` at f0a9835,
//! Apache-2.0, see NOTICE). Error text never echoes a cookie value.

use serde_json::{Value, json};

/// Accepts one of three formats:
/// 1. A JSON array of `{name, value}`
/// 2. A cURL command (`-H 'cookie: …'`, `-b '…'`/`--cookie '…'`)
/// 3. Bare cookie header — `name=value; name2=value2`
pub fn parse_curl_cookies(raw: &str) -> Result<Vec<Value>, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("cookies file is empty".to_string());
    }

    if trimmed.starts_with('[') {
        let arr: Vec<Value> = serde_json::from_str(trimmed)
            .map_err(|e| format!("cookies JSON parse error: {}", e))?;
        let mut out = Vec::with_capacity(arr.len());
        for (i, c) in arr.into_iter().enumerate() {
            let name = c
                .get("name")
                .and_then(|v| v.as_str())
                .ok_or_else(|| format!("cookies[{}] missing string name", i))?;
            let value = c
                .get("value")
                .and_then(|v| v.as_str())
                .ok_or_else(|| format!("cookies[{}] missing string value", i))?;
            out.push(json!({ "name": name, "value": value }));
        }
        return Ok(out);
    }

    // Heuristic: cURL commands start with `curl` followed by space/quote.
    let looks_like_curl = {
        let head: String = trimmed.chars().take(5).collect::<String>().to_lowercase();
        head.starts_with("curl")
            && head
                .chars()
                .nth(4)
                .is_some_and(|c| c.is_whitespace() || c == '\'' || c == '"')
    };

    let header = if looks_like_curl {
        extract_cookie_header_from_curl(trimmed).ok_or_else(|| {
            "no Cookie header found in this cURL - right-click an authenticated request in DevTools → Network → Copy → Copy as cURL".to_string()
        })?
    } else {
        trimmed.to_string()
    };

    parse_cookie_header(&header)
}

fn extract_cookie_header_from_curl(curl: &str) -> Option<String> {
    // Strip bash (`\`) and cmd (`^`) line continuations so -H is on one line.
    let joined = curl
        .replace("\\\r\n", " ")
        .replace("\\\n", " ")
        .replace("^\r\n", " ")
        .replace("^\n", " ");
    if let Some(v) = match_quoted_arg(&joined, "-H", Some("cookie")) {
        return Some(v);
    }
    if let Some(v) = match_quoted_arg(&joined, "-b", None) {
        return Some(v);
    }
    if let Some(v) = match_quoted_arg(&joined, "--cookie", None) {
        return Some(v);
    }
    None
}

/// Find `flag <quote>[header:]value<quote>` in haystack and return the value.
/// When `expect_header` is set, the quoted value must start with that header
/// name followed by a colon (case-insensitive) and the prefix is stripped.
fn match_quoted_arg(haystack: &str, flag: &str, expect_header: Option<&str>) -> Option<String> {
    let bytes = haystack.as_bytes();
    let flag_b = flag.as_bytes();
    let mut i = 0;
    while i + flag_b.len() < bytes.len() {
        if &bytes[i..i + flag_b.len()] != flag_b {
            i += 1;
            continue;
        }
        // Must be at a word boundary on the left (start of string or whitespace).
        if i > 0 && !bytes[i - 1].is_ascii_whitespace() {
            i += 1;
            continue;
        }
        let mut j = i + flag_b.len();
        // Require a whitespace separator after the flag.
        if j >= bytes.len() || !bytes[j].is_ascii_whitespace() {
            i += 1;
            continue;
        }
        while j < bytes.len() && bytes[j].is_ascii_whitespace() {
            j += 1;
        }
        if j >= bytes.len() {
            return None;
        }
        let quote = bytes[j];
        if quote != b'\'' && quote != b'"' {
            i = j;
            continue;
        }
        let start = j + 1;
        let mut k = start;
        while k < bytes.len() && bytes[k] != quote {
            k += 1;
        }
        if k >= bytes.len() {
            return None;
        }
        let value = String::from_utf8_lossy(&bytes[start..k]).into_owned();
        if let Some(header) = expect_header {
            let lower = value.to_lowercase();
            let prefix = format!("{}:", header.to_lowercase());
            if let Some(stripped) = lower.strip_prefix(&prefix) {
                let _ = stripped;
                return Some(value[prefix.len()..].trim().to_string());
            }
            i = k + 1;
            continue;
        }
        return Some(value);
    }
    None
}

fn parse_cookie_header(header: &str) -> Result<Vec<Value>, String> {
    let mut out = Vec::new();
    for piece in header.split(';') {
        let piece = piece.trim();
        let Some(eq) = piece.find('=') else { continue };
        let name = piece[..eq].trim();
        let value = piece[eq + 1..].trim();
        if !name.is_empty() {
            out.push(json!({ "name": name, "value": value }));
        }
    }
    if out.is_empty() {
        return Err("no cookies found in input".to_string());
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_curl_headers_and_json() {
        let curl = "curl 'http://127.0.0.1:8765/' -H 'cookie: a=1; b=2'";
        assert_eq!(
            parse_curl_cookies(curl).unwrap(),
            vec![
                json!({"name": "a", "value": "1"}),
                json!({"name": "b", "value": "2"})
            ]
        );
        assert_eq!(parse_curl_cookies("x=1").unwrap().len(), 1);
        assert_eq!(
            parse_curl_cookies(r#"[{"name":"n","value":"v"}]"#).unwrap()[0]["name"],
            "n"
        );
        assert!(parse_curl_cookies("curl 'http://x/'").is_err());
        assert_eq!(
            parse_curl_cookies("   ").unwrap_err(),
            "cookies file is empty"
        );
    }
}
