//! What a credential looks like (`src/lib/secrets/{shapes,detect}.ts`): the shapes, the label
//! vocabulary and the entropy gate, shared by the daemon's sealing (`vexdom_core::guardrails::secrets`)
//! and the extension's (through ext-core). Offsets are UTF-8 bytes. Regex spellings follow
//! `docs/research/js-regex-and-utf16.md` §2 (JS `\s`, ASCII `\b`, ASCII case folding).

use std::collections::HashMap;
use std::sync::LazyLock;

use regex::Regex;

/// JS `\s` (WhiteSpace + LineTerminator), for character classes.
pub const WS: &str = r"\t\n\x0B\x0C\r \x{A0}\x{1680}\x{2000}-\x{200A}\x{2028}\x{2029}\x{202F}\x{205F}\x{3000}\x{FEFF}";
/// JS `.` without `s`.
const DOT: &str = r"[^\n\r\x{2028}\x{2029}]";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Reveal {
    pub head: usize,
    pub tail: usize,
}

pub const NOTHING: Reveal = Reveal { head: 0, tail: 0 };

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Span {
    pub start: usize,
    pub end: usize,
    pub value: String,
    pub kind: &'static str,
    pub shape: &'static str,
    pub reveal: Reveal,
}

enum Pattern {
    Plain(Regex),
    Fancy(fancy_regex::Regex),
}

enum Validate {
    None,
    NotAPlaceholder,
    LooksLikeCredential,
    Card,
}

struct Shape {
    id: &'static str,
    kind: &'static str,
    pattern: Pattern,
    guard: Option<&'static str>,
    reveal: Reveal,
    validate: Validate,
}

type Word = &'static [&'static str];

const PASSWORD_WORDS: &[Word] = &[
    &["pass", "word"],
    &["pass", "wd"],
    &["pass", "phrase"],
    &["pass", "code"],
    &["pwd"],
    &["otp"],
    &["one", "time", "code"],
];

const TOKEN_WORDS: &[Word] = &[
    &["secret"],
    &["token"],
    &["api", "key"],
    &["access", "key"],
    &["access", "token"],
    &["secret", "key"],
    &["client", "secret"],
    &["refresh", "token"],
    &["auth", "token"],
    &["authorization"],
    &["bearer"],
    &["credential"],
    &["credentials"],
    &["signing", "key"],
    &["private", "key"],
    &["connection", "string"],
];

const COOKIE_WORDS: &[Word] = &[
    &["cookie"],
    &["session", "id"],
    &["session", "key"],
    &["session", "token"],
    &["csrf", "token"],
    &["xsrf", "token"],
];

/// ASCII-only case-insensitive spelling of a literal (JS `i` without `u`).
fn ci(word: &str) -> String {
    word.chars()
        .map(|c| {
            if c.is_ascii_alphabetic() {
                format!("[{}{}]", c.to_ascii_lowercase(), c.to_ascii_uppercase())
            } else {
                regex::escape(&c.to_string())
            }
        })
        .collect()
}

fn inline(words: &[Word]) -> String {
    let separator = format!("[_\\-{WS}]?");
    words
        .iter()
        .map(|word| {
            word.iter()
                .map(|part| ci(part))
                .collect::<Vec<_>>()
                .join(&separator)
        })
        .collect::<Vec<_>>()
        .join("|")
}

fn labelled(label: &str) -> String {
    let value = format!(
        r#"(?:{b}[{WS}]+|{ba}[{WS}]+|{t}[{WS}]+)?(?:"([^"\r\n]{{4,400}})"|'([^'\r\n]{{4,400}})'|([^{WS},;&"'<>{{}}\[\]]{{4,400}}))"#,
        b = ci("bearer"),
        ba = ci("basic"),
        t = ci("token"),
    );
    format!(r#"(?<![A-Za-z0-9])(?:{label})["']?[{WS}]*[:=][{WS}]*{value}"#)
}

fn prose(label: &str) -> String {
    let value = format!(
        r#"(?:"([^"\r\n]{{4,400}})"|'([^'\r\n]{{4,400}})'|([^{WS}"'<>]{{3,399}}[^{WS}"'<>.,;:!?]))"#
    );
    format!(
        r#"(?<![A-Za-z0-9])(?:{label})[{WS}]+(?:{is}|{are}|{was}|{will}[{WS}]+{be})[{WS}]*:?[{WS}]+{value}"#,
        is = ci("is"),
        are = ci("are"),
        was = ci("was"),
        will = ci("will"),
        be = ci("be"),
    )
}

fn plain(pattern: &str) -> Pattern {
    Pattern::Plain(Regex::new(pattern).expect("secret shape compiles"))
}

fn fancy(pattern: &str) -> Pattern {
    Pattern::Fancy(fancy_regex::Regex::new(pattern).expect("secret shape compiles"))
}

fn shape(
    id: &'static str,
    kind: &'static str,
    guard: Option<&'static str>,
    pattern: Pattern,
    head: usize,
    tail: usize,
    validate: Validate,
) -> Shape {
    Shape {
        id,
        kind,
        pattern,
        guard,
        reveal: Reveal { head, tail },
        validate,
    }
}

static SHAPES: LazyLock<Vec<Shape>> = LazyLock::new(|| {
    use Validate::{Card, LooksLikeCredential, None as Any, NotAPlaceholder};
    let password = inline(PASSWORD_WORDS);
    let token = inline(TOKEN_WORDS);
    let cookie = inline(COOKIE_WORDS);
    vec![
        shape(
            "private-key",
            "private-key",
            Some("-----begin"),
            plain(&format!(
                r"-----BEGIN(?:[A-Z ]{{0,32}})PRIVATE KEY-----[A-Za-z0-9+/={WS}]{{0,8000}}-----END(?:[A-Z ]{{0,32}})PRIVATE KEY-----"
            )),
            0,
            0,
            Any,
        ),
        shape(
            "jwt",
            "jwt",
            Some("eyj"),
            plain(r"(?-u:\b)eyJ[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{6,}\.[A-Za-z0-9_-]{6,}"),
            0,
            0,
            Any,
        ),
        shape(
            "anthropic-key",
            "api-key",
            Some("sk-ant-"),
            plain(r"(?-u:\b)sk-ant-[A-Za-z0-9_-]{16,}"),
            7,
            0,
            Any,
        ),
        shape(
            "openai-key",
            "api-key",
            Some("sk-"),
            plain(r"(?-u:\b)sk-(?:proj-|svcacct-|admin-)?[A-Za-z0-9_-]{20,}"),
            3,
            0,
            Any,
        ),
        shape(
            "google-key",
            "api-key",
            Some("aiza"),
            plain(r"(?-u:\b)AIza[0-9A-Za-z_-]{35}(?-u:\b)"),
            4,
            0,
            Any,
        ),
        shape(
            "aws-access-key",
            "api-key",
            None,
            plain(r"(?-u:\b)(?:AKIA|ASIA|AIDA|AROA|AGPA|ANPA)[0-9A-Z]{16}(?-u:\b)"),
            4,
            0,
            Any,
        ),
        shape(
            "github-pat",
            "token",
            Some("github_pat_"),
            plain(r"(?-u:\b)github_pat_[A-Za-z0-9_]{40,}"),
            11,
            0,
            Any,
        ),
        shape(
            "github-token",
            "token",
            Some("gh"),
            plain(r"(?-u:\b)gh[pousr]_[A-Za-z0-9]{30,}"),
            4,
            0,
            Any,
        ),
        shape(
            "slack-token",
            "token",
            Some("xox"),
            plain(r"(?-u:\b)xox[abposr]-[A-Za-z0-9-]{10,}"),
            4,
            0,
            Any,
        ),
        shape(
            "stripe-key",
            "api-key",
            Some("k_"),
            plain(r"(?-u:\b)[rs]k_(?:live|test)_[A-Za-z0-9]{16,}"),
            8,
            0,
            Any,
        ),
        shape(
            "npm-token",
            "token",
            Some("npm_"),
            plain(r"(?-u:\b)npm_[A-Za-z0-9]{36}(?-u:\b)"),
            4,
            0,
            Any,
        ),
        shape(
            "gitlab-token",
            "token",
            Some("glpat-"),
            plain(r"(?-u:\b)glpat-[A-Za-z0-9_-]{20,}"),
            6,
            0,
            Any,
        ),
        shape(
            "sendgrid-key",
            "api-key",
            Some("sg."),
            plain(r"(?-u:\b)SG\.[A-Za-z0-9_-]{16,}\.[A-Za-z0-9_-]{16,}"),
            3,
            0,
            Any,
        ),
        shape(
            "basic-auth",
            "password",
            Some("@"),
            plain(&format!(
                r"(?-u:\b)https?://[^{WS}/:@]{{1,64}}:([^{WS}/@]{{3,128}})@"
            )),
            0,
            0,
            Any,
        ),
        shape(
            "cookie-header",
            "cookie",
            Some("cookie"),
            plain(r"(?:^|\n)[ \t]*(?:(?i-u:set-))?(?i-u:cookie)[ \t]*:[ \t]*([^\r\n]{4,4000})"),
            0,
            0,
            Any,
        ),
        shape(
            "labelled-password",
            "password",
            None,
            fancy(&labelled(&password)),
            0,
            0,
            NotAPlaceholder,
        ),
        shape(
            "labelled-token",
            "token",
            None,
            fancy(&labelled(&token)),
            0,
            0,
            NotAPlaceholder,
        ),
        shape(
            "labelled-cookie",
            "cookie",
            None,
            fancy(&labelled(&cookie)),
            0,
            0,
            NotAPlaceholder,
        ),
        shape(
            "prose-password",
            "password",
            None,
            fancy(&prose(&password)),
            0,
            0,
            LooksLikeCredential,
        ),
        shape(
            "prose-token",
            "token",
            None,
            fancy(&prose(&token)),
            0,
            0,
            LooksLikeCredential,
        ),
        shape(
            "card",
            "card",
            None,
            plain(r"(?-u:\b)[0-9](?:[ \-]?[0-9]){12,18}(?-u:\b)"),
            0,
            4,
            Card,
        ),
    ]
});

static CANDIDATE: LazyLock<fancy_regex::Regex> = LazyLock::new(|| {
    fancy_regex::Regex::new(
        r"(?<![A-Za-z0-9+/_=\-])[A-Za-z0-9+/_\-]{32,4096}={0,2}(?![A-Za-z0-9+/_\-])",
    )
    .expect("candidate compiles")
});
static UUID: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^[0-9A-Fa-f]{8}-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{12}$")
        .expect("uuid compiles")
});
static DATA_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(
        r"(?-u:\b)data:[^{WS};,]{{0,80}};base64,[A-Za-z0-9+/=]+"
    ))
    .expect("data url compiles")
});
static CREDENTIAL_SIGNAL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"[0-9]|[!@#$%^&*()_+=\[\]{}|\\<>~/]|[a-z][A-Z]").expect("signal compiles")
});
static PLACEHOLDER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(r"^(?:(?i-u:null|nil|none|true|false|undefined|n/?a|empty|blank|test|demo|example|sample|changeme|hidden|redacted)|(?i-u:your)[\-_{WS}]{DOT}*|(?i-u:my)[\-_{WS}]{DOT}*|(?i-u:x){{3,}}|\*+|•+|\.{{3,}}|…+|-+|_+|\[[^\]]*\]|<[^>]*>|\{{\{{{DOT}*\}}\}}|\$\{{{DOT}*\}})$"))
        .expect("placeholder compiles")
});

/// `/^(.)\1*$/`: one UTF-16 unit, not a line terminator, repeated over the whole string.
fn same_unit(value: &str) -> bool {
    let mut units = value.encode_utf16();
    match units.next() {
        Some(first) if ![0x0A, 0x0D, 0x2028, 0x2029].contains(&first) => {
            units.all(|unit| unit == first)
        }
        _ => false,
    }
}

pub fn not_a_placeholder(value: &str) -> bool {
    !PLACEHOLDER.is_match(value) && !same_unit(value) && !value.contains('…')
}

pub fn looks_like_credential(value: &str) -> bool {
    value.encode_utf16().count() >= 6
        && not_a_placeholder(value)
        && CREDENTIAL_SIGNAL.is_match(value)
}

/// `looksLikeCardNumber` (`src/lib/recordings/events.ts`): 13–19 digits passing Luhn.
pub fn looks_like_card_number(value: &str) -> bool {
    let digits: Vec<u8> = value
        .chars()
        .filter(|c| !(c.is_whitespace() || *c == '-' || *c == '\u{FEFF}'))
        .map(|c| {
            if c.is_ascii_digit() {
                c as u8 - b'0'
            } else {
                255
            }
        })
        .collect();
    if !(13..=19).contains(&digits.len()) || digits.contains(&255) {
        return false;
    }
    let mut sum = 0u32;
    let mut double = false;
    for digit in digits.iter().rev() {
        let mut digit = u32::from(*digit);
        if double {
            digit *= 2;
            if digit > 9 {
                digit -= 9;
            }
        }
        sum += digit;
        double = !double;
    }
    sum.is_multiple_of(10)
}

/// The kind an object key names (`kindForKey`), separators ignored.
pub fn kind_for_key(key: &str) -> Option<&'static str> {
    let normalized: String = key
        .to_lowercase()
        .chars()
        .filter(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        .collect();
    if normalized.is_empty() {
        return None;
    }
    let mut forms = vec![normalized.clone()];
    if let Some(single) = normalized.strip_suffix('s') {
        forms.push(single.to_owned());
    }
    for (kind, words) in [
        ("password", PASSWORD_WORDS),
        ("token", TOKEN_WORDS),
        ("cookie", COOKIE_WORDS),
    ] {
        if words
            .iter()
            .any(|word| forms.iter().any(|form| form.ends_with(&word.concat())))
        {
            return Some(kind);
        }
    }
    None
}

#[derive(Debug, Clone, Copy)]
struct Range {
    start: usize,
    end: usize,
}

fn overlaps(claimed: &[Range], start: usize, end: usize) -> bool {
    claimed
        .iter()
        .any(|range| start < range.end && range.start < end)
}

/// Every capture of a match as `(start, end)`, `None` for a group that did not take part.
fn matches(pattern: &Pattern, text: &str) -> Vec<Vec<Option<(usize, usize)>>> {
    match pattern {
        Pattern::Plain(regex) => regex
            .captures_iter(text)
            .map(|caps| {
                caps.iter()
                    .map(|group| group.map(|m| (m.start(), m.end())))
                    .collect()
            })
            .collect(),
        Pattern::Fancy(regex) => regex
            .captures_iter(text)
            .filter_map(Result::ok)
            .map(|caps| {
                caps.iter()
                    .map(|group| group.map(|m| (m.start(), m.end())))
                    .collect()
            })
            .collect(),
    }
}

/// The first defined capture group is the secret; with no groups, the whole match is.
fn secret_in(groups: &[Option<(usize, usize)>]) -> Option<(usize, usize)> {
    let whole = groups[0]?;
    match groups[1..].iter().flatten().next() {
        None if groups.len() == 1 || groups[1..].iter().all(Option::is_none) => Some(whole),
        Some(&(start, end)) if start < end => Some((start, end)),
        _ => None,
    }
}

fn looks_high_entropy(value: &str) -> bool {
    if value.len() < 32 || UUID.is_match(value) {
        return false;
    }
    if value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return false;
    }
    let bytes = value.as_bytes();
    if !bytes.iter().any(u8::is_ascii_lowercase)
        || !bytes.iter().any(u8::is_ascii_uppercase)
        || !bytes.iter().any(u8::is_ascii_digit)
    {
        return false;
    }
    entropy(value) >= 4.3 && case_flips(value) >= 0.5
}

pub fn case_flips(value: &str) -> f64 {
    let letters: Vec<u8> = value.bytes().filter(u8::is_ascii_alphabetic).collect();
    if letters.len() < 2 {
        return 0.0;
    }
    let flips = letters
        .windows(2)
        .filter(|pair| pair[0].is_ascii_uppercase() != pair[1].is_ascii_uppercase())
        .count();
    flips as f64 / (letters.len() - 1) as f64
}

pub fn entropy(value: &str) -> f64 {
    let mut counts: HashMap<char, usize> = HashMap::new();
    for c in value.chars() {
        *counts.entry(c).or_default() += 1;
    }
    let length = value.encode_utf16().count() as f64;
    // Subtracted from 0.0 as the TS does, so one repeated character is +0, not -0.
    counts.values().fold(0.0, |bits, &count| {
        let p = count as f64 / length;
        bits - p * p.log2()
    })
}

/// Finds the secrets in a string; later shapes that overlap an earlier find are dropped.
pub fn find_secrets(text: &str, immune: &[(usize, usize)]) -> Vec<Span> {
    if text.is_empty() {
        return Vec::new();
    }
    let mut claimed: Vec<Range> = immune
        .iter()
        .map(|&(start, end)| Range { start, end })
        .chain(DATA_URL.find_iter(text).map(|m| Range {
            start: m.start(),
            end: m.end(),
        }))
        .collect();
    let lower = text.to_lowercase();
    let mut found = Vec::new();
    for shape in SHAPES.iter() {
        if shape.guard.is_some_and(|guard| !lower.contains(guard)) {
            continue;
        }
        for groups in matches(&shape.pattern, text) {
            let Some((start, end)) = secret_in(&groups) else {
                continue;
            };
            let value = &text[start..end];
            let valid = match shape.validate {
                Validate::None => true,
                Validate::NotAPlaceholder => not_a_placeholder(value),
                Validate::LooksLikeCredential => looks_like_credential(value),
                Validate::Card => looks_like_card_number(value),
            };
            if !valid || overlaps(&claimed, start, end) {
                continue;
            }
            claimed.push(Range { start, end });
            found.push(Span {
                start,
                end,
                value: value.to_owned(),
                kind: shape.kind,
                shape: shape.id,
                reveal: shape.reveal,
            });
        }
    }
    for candidate in CANDIDATE.find_iter(text).filter_map(Result::ok) {
        let value = candidate.as_str();
        if !looks_high_entropy(value) || overlaps(&claimed, candidate.start(), candidate.end()) {
            continue;
        }
        claimed.push(Range {
            start: candidate.start(),
            end: candidate.end(),
        });
        found.push(Span {
            start: candidate.start(),
            end: candidate.end(),
            value: value.to_owned(),
            kind: "secret",
            shape: "high-entropy",
            reveal: NOTHING,
        });
    }
    found.sort_by_key(|span| span.start);
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_vendor_keys_with_their_marker() {
        let text = "key: sk-ant-abcdefghijklmnopqrstuvwxyz0123";
        let spans = find_secrets(text, &[]);
        assert_eq!(spans.len(), 1);
        assert_eq!(
            (spans[0].kind, spans[0].shape),
            ("api-key", "anthropic-key")
        );
        assert_eq!(
            &text[spans[0].start..spans[0].end],
            "sk-ant-abcdefghijklmnopqrstuvwxyz0123"
        );
        assert_eq!(spans[0].reveal, Reveal { head: 7, tail: 0 });
    }

    #[test]
    fn immune_ranges_and_placeholders() {
        let text = "password: hunter2!";
        assert_eq!(find_secrets(text, &[(0, text.len())]), Vec::new());
        assert!(find_secrets("password: ********", &[]).is_empty());
        assert!(!not_a_placeholder("your-token"));
        assert!(looks_like_credential("Xk9#mmq2"));
        assert_eq!(kind_for_key("API_KEYS"), Some("token"));
    }

    // The detection table from `src/lib/secrets/secrets.test.ts` (ported with P9).
    #[test]
    fn detects_every_shape() {
        for (text, shape) in [
            (
                "sk-ant-api03-AbCdEfGhIjKlMnOpQrStUvWxYz012345",
                "anthropic-key",
            ),
            ("ghp_AbCdEf0123456789AbCdEf0123456789abcd", "github-token"),
            ("AKIAIOSFODNN7EXAMPLE", "aws-access-key"),
            ("AIzaSyA1B2c3D4e5F6g7H8i9J0k1L2m3N4o5P6q", "google-key"),
            ("sk_live_AbCdEf0123456789xyz", "stripe-key"),
            (
                "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.dQw4w9WgXcQabcdef",
                "jwt",
            ),
            ("password: hunter2Nowaythis", "labelled-password"),
            ("api_key = \"AbCdEf0123456789\"", "labelled-token"),
            ("Authorization: Bearer abc123def456ghi", "labelled-token"),
            ("Cookie: session=abc123def456; theme=dark", "cookie-header"),
            ("https://user:s3cretPassw0rd@example.com/x", "basic-auth"),
            ("4242 4242 4242 4242", "card"),
            (
                "Your temporary password is Tr0ub4dor&3xK9",
                "prose-password",
            ),
            (
                "Your new password will be: Hunter2Kestrel",
                "prose-password",
            ),
            ("The API key is AbCdEf0123456789xyz", "prose-token"),
            ("Xk9mPq2LvRt7Yn4WzB8sJd3HgF6cA1eU", "high-entropy"),
            (
                "-----BEGIN RSA PRIVATE KEY-----\nMIIEowIBAAKCAQEA1234\n-----END RSA PRIVATE KEY-----",
                "private-key",
            ),
        ] {
            let shapes: Vec<_> = find_secrets(text, &[])
                .iter()
                .map(|span| span.shape)
                .collect();
            assert!(shapes.contains(&shape), "{shape} in {text:?}: {shapes:?}");
        }
    }

    #[test]
    fn leaves_page_text_alone() {
        for text in [
            "Sign in to your account to continue reading the article",
            "see https://cdn.site.com/assets/index-a1b2c3d4e5f6a7b8.js for details",
            "commit 5f2a9c8e1b3d7f0a4c6e8b2d5a7f9c1e3b6d8a0f",
            "id 550e8400-e29b-41d4-a716-446655440000",
            "ThisIsALongCamelCaseIdentifier12",
            "GetUserProfileByAccountIdV2Handler",
            "ContinueReadingTheFullArticleHere",
            "password: ********",
            "password: your-password-here",
            "password: null",
            "A password is required to continue",
            "Your password is incorrect. Please try again.",
            "The password is case-sensitive",
            "This session is expired",
            "order 1234567890123456789012345678",
        ] {
            assert_eq!(find_secrets(text, &[]), Vec::new(), "{text:?}");
        }
    }

    /// The inline labels and the key matcher come from one word list; every word must read both ways.
    #[test]
    fn every_secret_word_reads_inline_and_as_a_key() {
        for word in PASSWORD_WORDS.iter().chain(TOKEN_WORDS).chain(COOKIE_WORDS) {
            let snake = word.join("_");
            let text = format!("{snake}: hunter2Nowaythis");
            assert!(
                find_secrets(&text, &[])
                    .iter()
                    .any(|span| span.value.contains("hunter2")),
                "{text:?}"
            );
            let camel: String = word
                .iter()
                .enumerate()
                .map(|(i, part)| {
                    if i == 0 {
                        (*part).to_owned()
                    } else {
                        part[..1].to_uppercase() + &part[1..]
                    }
                })
                .collect();
            for key in [snake.as_str(), camel.as_str()] {
                assert!(kind_for_key(key).is_some(), "{key}");
            }
        }
        for key in [
            "username",
            "email",
            "title",
            "href",
            "summary",
            "passenger",
            "sessionCount",
            "tokenizer",
        ] {
            assert_eq!(kind_for_key(key), None, "{key}");
        }
    }
}
