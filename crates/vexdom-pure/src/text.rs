//! JavaScript string semantics the ported texts depend on: `\s`, `trim()`, `length` and `slice`
//! count UTF-16 units, and numbers print without a trailing `.0`.

use base64::Engine as _;
use serde_json::Value;

/// `\s` in a JavaScript regex.
pub fn is_js_space(c: char) -> bool {
    matches!(
        c,
        '\t' | '\n' | '\u{0B}' | '\u{0C}' | '\r' | ' ' | '\u{A0}' | '\u{1680}' | '\u{2000}'
            ..='\u{200A}'
                | '\u{2028}'
                | '\u{2029}'
                | '\u{202F}'
                | '\u{205F}'
                | '\u{3000}'
                | '\u{FEFF}'
    )
}

/// `text.trim()`.
pub fn js_trim(text: &str) -> &str {
    text.trim_matches(is_js_space)
}

/// `text.replace(/\s+/g, ' ')`.
pub fn collapse_spaces(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut in_space = false;
    for c in text.chars() {
        if is_js_space(c) {
            if !in_space {
                out.push(' ');
            }
            in_space = true;
        } else {
            out.push(c);
            in_space = false;
        }
    }
    out
}

/// `text.replace(/\s+/g, ' ').trim()`.
pub fn flatten(text: &str) -> String {
    js_trim(&collapse_spaces(text)).to_owned()
}

/// `encodeURIComponent(text)`: keeps `A-Za-z0-9-_.!~*'()`, percent-encodes the rest as UTF-8.
pub fn encode_uri_component(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || b"-_.!~*'()".contains(&byte) {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

/// `text.length`.
pub fn utf16_len(text: &str) -> usize {
    text.encode_utf16().count()
}

/// `text.slice(0, units)`; a surrogate pair the cut would split is left out.
pub fn utf16_prefix(text: &str, units: usize) -> &str {
    let mut count = 0;
    for (at, c) in text.char_indices() {
        count += c.len_utf16();
        if count > units {
            return &text[..at];
        }
    }
    text
}

/// `text.slice(-units)`.
pub fn utf16_suffix(text: &str, units: usize) -> &str {
    let mut count = 0;
    for (at, c) in text.char_indices().rev() {
        count += c.len_utf16();
        if count > units {
            return &text[at + c.len_utf8()..];
        }
    }
    text
}

/// `a.localeCompare(b)` for the names Vexdom sorts: ICU's root order for ASCII (spaces, then
/// punctuation and symbols in CLDR's order, digits, then letters whatever their case), lowercase
/// first on a tie; anything else after, by code point.
pub fn locale_compare(a: &str, b: &str) -> std::cmp::Ordering {
    const MARKS: &str = "_-,;:!?.'\"()[]{}@*/\\&#%`^+<=>|~$";
    fn weight(c: char) -> (u8, u32) {
        if c.is_whitespace() {
            (0, u32::from(c))
        } else if let Some(at) = MARKS.find(c) {
            (1, u32::try_from(at).unwrap_or(u32::MAX))
        } else if c.is_ascii_digit() {
            (2, u32::from(c))
        } else if c.is_ascii_alphabetic() {
            (3, u32::from(c.to_ascii_lowercase()))
        } else {
            (4, u32::from(c.to_lowercase().next().unwrap_or(c)))
        }
    }
    a.chars()
        .map(weight)
        .cmp(b.chars().map(weight))
        .then_with(|| b.cmp(a))
}

/// A JSON value as a template literal prints it: numbers without a trailing `.0`.
pub fn js_text(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Number(number) => match number.as_f64() {
            Some(float) if number.is_f64() && float.fract() == 0.0 && float.abs() < 1e21 => {
                format!("{float:.0}")
            }
            _ => number.to_string(),
        },
        Value::Bool(flag) => flag.to_string(),
        Value::Null => "null".into(),
        Value::Array(_) | Value::Object(_) => value.to_string(),
    }
}

/// `Number.isSafeInteger(value)` for a JSON number, as an `i64`.
pub fn safe_integer(value: &Value) -> Option<i64> {
    const MAX_SAFE: f64 = 9_007_199_254_740_991.0;
    let float = value.as_f64()?;
    (float.fract() == 0.0 && float.abs() <= MAX_SAFE).then_some(float as i64)
}

/// `Buffer.from(text, 'base64')`: url-safe letters accepted, anything else skipped.
pub fn decode_base64(text: &str) -> Vec<u8> {
    let mut clean: String = text
        .chars()
        .filter_map(|c| match c {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '+' | '/' => Some(c),
            '-' => Some('+'),
            '_' => Some('/'),
            _ => None,
        })
        .collect();
    if clean.len() % 4 == 1 {
        clean.pop();
    }
    base64::engine::general_purpose::STANDARD_NO_PAD
        .decode(clean)
        .unwrap_or_default()
}

/// `{a: 1, ...}` key lookup for an optional string field.
pub fn str_field<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value.get(key).and_then(Value::as_str)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn whitespace_is_the_regex_class() {
        assert_eq!(flatten("\u{FEFF} a \n\t b\u{3000}"), "a b");
        assert_eq!(js_trim("\u{85}x"), "\u{85}x");
    }

    #[test]
    fn lengths_and_slices_count_utf16_units() {
        assert_eq!(utf16_len("🙂a"), 3);
        assert_eq!(utf16_prefix("🙂a", 1), "");
        assert_eq!(utf16_prefix("🙂a", 2), "🙂");
        assert_eq!(utf16_suffix("ab🙂", 2), "🙂");
        assert_eq!(utf16_suffix("ab", 5), "ab");
    }

    #[test]
    fn names_sort_as_locale_compare_does() {
        let mut names = vec![
            "review",
            "Deploy-app",
            "deploy",
            "_draft",
            "2fa",
            "deploy_app",
            "a",
        ];
        names.sort_by(|a, b| locale_compare(a, b));
        assert_eq!(
            names,
            [
                "_draft",
                "2fa",
                "a",
                "deploy",
                "deploy_app",
                "Deploy-app",
                "review"
            ]
        );
        assert_eq!(locale_compare("a", "A"), std::cmp::Ordering::Less);
    }

    #[test]
    fn numbers_print_as_javascript_does() {
        assert_eq!(js_text(&json!(1280.0)), "1280");
        assert_eq!(js_text(&json!(12.5)), "12.5");
        assert_eq!(js_text(&json!(7)), "7");
        assert_eq!(safe_integer(&json!(3.0)), Some(3));
        assert_eq!(safe_integer(&json!(3.5)), None);
    }

    #[test]
    fn base64_is_lenient() {
        assert_eq!(decode_base64("aGk="), b"hi");
        assert_eq!(decode_base64("aG\nk"), b"hi");
    }
}
