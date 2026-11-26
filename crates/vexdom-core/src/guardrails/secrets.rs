//! The daemon's half of the sanitizer: replaces the credentials `vexdom_pure::secrets` finds with
//! handles `⟦kind:id#tag⟧` through `vexdom_pure::seal`, the same grammar and walk the extension seals
//! with. There is no vault: this half only seals (non-strict, its own counter mint), so the daemon
//! never holds a credential it could leak.

use std::sync::LazyLock;
use std::sync::atomic::{AtomicU64, Ordering};

use regex::Regex;
use serde_json::Value;
use vexdom_pure::seal;
use vexdom_pure::secrets::WS;

pub use vexdom_pure::seal::{CLOSE, Handle, OPEN};

pub fn handles_in(text: &str) -> Vec<Handle> {
    seal::handles_in(text, None)
}

/// Every handle anywhere in a JSON value.
pub fn sealed_handles(value: &Value) -> Vec<Handle> {
    seal::sealed_handles(value, None)
}

/// Per daemon process: a page cannot author a handle with a tag it never sees.
static TAG: LazyLock<String> = LazyLock::new(|| crate::rand::hex(8));
static SEQ: AtomicU64 = AtomicU64::new(0);

fn to_base36(mut n: u64) -> String {
    const DIGITS: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    if n == 0 {
        return "0".into();
    }
    let mut out = Vec::new();
    while n > 0 {
        out.push(DIGITS[(n % 36) as usize]);
        n /= 36;
    }
    out.reverse();
    String::from_utf8(out).expect("base36 digits are ascii")
}

fn mint(_value: &str, kind: &str, _shape: &str) -> String {
    let id = to_base36(SEQ.fetch_add(1, Ordering::Relaxed) + 1);
    seal::handle_for(kind, &id, None, &TAG)
}

/// Seal a string on its way to a model, a client, or the user. Handles already in the text
/// (from the extension) are immune.
pub fn seal_secrets(text: &str) -> String {
    seal::seal_text(text, None, &mut mint)
}

/// Seal every string in a JSON value; a string hanging off a credential-named key is sealed
/// whole. `selector`, `ref` and `doc` values are round-trip tokens and left alone.
pub fn seal_secrets_in(value: &Value) -> Value {
    seal::seal_value(value, None, &mut mint)
}

const HOLD: usize = 256;
const MAX_HOLD: usize = 2_048;
static LABEL_TAIL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(r#"[A-Za-z0-9_-]{{2,32}}["']?[{WS}]*[:=][{WS}]*$"#))
        .expect("label tail compiles")
});

/// Seals text that arrives in deltas, holding back what might be half a secret.
#[derive(Default)]
pub struct StreamSealer {
    held: String,
}

impl StreamSealer {
    pub fn push(&mut self, delta: &str) -> String {
        self.held.push_str(delta);
        let cut = cut_point(&self.held);
        if cut == 0 {
            return String::new();
        }
        let rest = self.held.split_off(cut);
        let out = std::mem::replace(&mut self.held, rest);
        seal_secrets(&out)
    }

    pub fn flush(&mut self) -> String {
        if self.held.is_empty() {
            return String::new();
        }
        seal_secrets(&std::mem::take(&mut self.held))
    }
}

fn floor_boundary(text: &str, mut at: usize) -> usize {
    while at > 0 && !text.is_char_boundary(at) {
        at -= 1;
    }
    at
}

fn cut_point(held: &str) -> usize {
    if held.len() <= HOLD {
        return 0;
    }
    let limit = floor_boundary(held, held.len() - HOLD);
    let mut cut = match held[..=limit.min(held.len() - 1)].rfind('\n') {
        Some(at) => at,
        None => {
            if held.len() <= MAX_HOLD {
                return 0;
            }
            match held[..limit]
                .char_indices()
                .rev()
                .find(|(at, c)| *at > 0 && c.is_whitespace())
            {
                Some((at, _)) => at,
                None => limit,
            }
        }
    };
    let window = floor_boundary(held, cut.saturating_sub(48));
    if let Some(label) = LABEL_TAIL.find(&held[window..cut]) {
        cut = window + label.start();
    }
    if let Some(begin) = held.rfind("-----BEGIN")
        && begin < cut
        && !held[begin..].contains("-----END")
    {
        cut = begin;
    }
    cut
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seals_vendor_keys_with_their_marker() {
        let sealed = seal_secrets("key: sk-ant-abcdefghijklmnopqrstuvwxyz0123");
        assert!(sealed.starts_with("key: sk-ant-…⟦api-key:"), "{sealed}");
        assert!(!sealed.contains("abcdefghij"));
    }

    #[test]
    fn labelled_and_prose() {
        assert!(seal_secrets("password: hunter2!").contains("⟦password:"));
        assert_eq!(seal_secrets("password: ********"), "password: ********");
        assert!(seal_secrets("Your temporary password is Xk9#mmq2").contains("⟦password:"));
        assert_eq!(
            seal_secrets("The password is required"),
            "The password is required"
        );
        assert!(!seal_secrets("xpassword: hunter22").contains('⟦'));
    }

    #[test]
    fn entropy_gate() {
        let token = "aZ3kQ9pL2mX7vB4nR8tY1wE6uI5oP0sDfGhJk";
        assert!(seal_secrets(token).contains("⟦secret:"));
        assert_eq!(
            seal_secrets("ContinueReadingTheFullArticleNowPlease2"),
            "ContinueReadingTheFullArticleNowPlease2"
        );
        assert_eq!(
            seal_secrets("0123456789abcdef0123456789abcdef01"),
            "0123456789abcdef0123456789abcdef01"
        );
    }

    #[test]
    fn cards_keep_the_last_four() {
        let sealed = seal_secrets("card 4242 4242 4242 4242 ok");
        assert!(
            sealed.contains("⟦card:") && sealed.ends_with("…4242 ok"),
            "{sealed}"
        );
        assert_eq!(seal_secrets("1234 5678 9012 3456"), "1234 5678 9012 3456");
    }

    #[test]
    fn existing_handles_are_immune_and_found() {
        let text = "use ⟦password:1@example.com#abcdef12⟧ here";
        assert_eq!(seal_secrets(text), text);
        let handles = sealed_handles(&serde_json::json!({"value": text}));
        assert_eq!(handles[0].origin.as_deref(), Some("example.com"));
    }

    #[test]
    fn keyed_values() {
        let sealed = seal_secrets_in(
            &serde_json::json!({"password": "hunter2", "selector": "#password", "n": 1}),
        );
        assert!(
            sealed["password"]
                .as_str()
                .unwrap()
                .starts_with("⟦password:")
        );
        assert_eq!(sealed["selector"], "#password");
        assert_eq!(
            vexdom_pure::secrets::kind_for_key("newPassword"),
            Some("password")
        );
        assert_eq!(
            vexdom_pure::secrets::kind_for_key("API_KEYS"),
            Some("token")
        );
        assert_eq!(vexdom_pure::secrets::kind_for_key("title"), None);
    }

    #[test]
    fn stream_holds_back_the_edge() {
        let mut sealer = StreamSealer::default();
        let mut out = sealer.push("intro ");
        out += &sealer.push("sk-ant-abcdefgh");
        out += &sealer.push("ijklmnopqrstuvwxyz");
        out += &sealer.flush();
        assert!(out.contains("⟦api-key:"), "{out}");
        let long = format!("{}more", "line\n".repeat(100));
        let mut sealer = StreamSealer::default();
        let first = sealer.push(&long);
        assert!(!first.is_empty());
        assert_eq!(first + &sealer.flush(), long);
    }
}
