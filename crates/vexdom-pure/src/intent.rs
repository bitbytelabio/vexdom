//! The local intent grammar (`src/lib/intent/{normalize,grammar,route}.ts`, ported with P9): routes a
//! plain instruction typed into the side panel to one page action when it is unambiguous, and
//! escalates everything else to the agent. Pure: the same text always routes the same way.
//!
//! The rules run on normalized text, which is ASCII (lowercase letters, digits, spaces and
//! `./:&=@_+#-`), so the regex crate's semantics match the JavaScript ones the rules were written in.

use std::sync::LazyLock;

use regex::{Captures, Regex};
use serde_json::{Value, json};

use crate::secrets::WS;
use crate::text::{encode_uri_component, js_trim};

/// Below this score a match escalates rather than acts.
pub const ACT_THRESHOLD: f64 = 0.75;

/// A normalized instruction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Utterance {
    pub text: String,
    pub raw: String,
    pub question: bool,
}

fn re(pattern: &str) -> Regex {
    Regex::new(pattern).expect("intent pattern compiles")
}

static SPOKEN_TLD: LazyLock<Regex> = LazyLock::new(|| {
    re(&format!(
        r"(?-u:\b)([a-z0-9][a-z0-9-]*)[{WS}]+dot[{WS}]+(com|net|org|io|dev|co|ai|app|edu|gov|me|uk|xyz)(?-u:\b)"
    ))
});
static OPENER: LazyLock<Regex> = LazyLock::new(|| {
    re(
        r"^(?:hey|hi|ok|okay|yo|um|uh|please|vexdom|assistant|can you|could you|would you|will you|i want you to|i would like you to|id like you to|lets|just|now|go ahead and)\s+",
    )
});
static TRAILER: LazyLock<Regex> = LazyLock::new(|| {
    re(
        r"\s+(?:please|thanks|thank you|for me|now|a bit|a little|a lot|some more|more|slightly|real quick|quickly)$",
    )
});
static PUNCTUATION: LazyLock<Regex> = LazyLock::new(|| re(&format!(r"[^a-z0-9{WS}./:&=@_+#-]")));
static SPACES: LazyLock<Regex> = LazyLock::new(|| re(&format!(r"[{WS}]+")));
static QUESTION_MARK: LazyLock<Regex> = LazyLock::new(|| re(&format!(r"\?[{WS}]*$")));

/// Lowercases, spells out spoken TLDs (`github dot com`), drops punctuation, and peels polite
/// openers and trailers until none is left.
pub fn normalize(input: &str) -> Utterance {
    let raw = js_trim(input).to_owned();
    let lowered: String = raw
        .to_lowercase()
        .chars()
        .filter(|c| !matches!(c, '\'' | '‘' | '’'))
        .collect();
    let text = SPOKEN_TLD.replace_all(&lowered, "$1.$2");
    let text = PUNCTUATION.replace_all(&text, " ");
    let mut text = js_trim(&SPACES.replace_all(&text, " ")).to_owned();

    let mut softened = false;
    loop {
        let previous = text.clone();
        let peeled = OPENER.replace(&text, "");
        let peeled = TRAILER.replace(&peeled, "");
        text = js_trim(&peeled).to_owned();
        softened |= text != previous;
        if text == previous {
            break;
        }
    }
    let text = text.strip_suffix('.').unwrap_or(&text).to_owned();
    let question = QUESTION_MARK.is_match(&raw) && !softened;
    Utterance {
        text,
        raw,
        question,
    }
}

/// What a rule's builder makes of its match.
struct Built {
    input: Value,
    label: String,
    confidence: f64,
    risky: bool,
}

struct Rule {
    id: &'static str,
    action: &'static str,
    certainty: f64,
    pattern: Regex,
    build: fn(&Captures) -> Option<Built>,
}

const NAVIGATE: &str = "page.navigate";
const OPEN_TAB: &str = "page.openTab";
const SEARCH_SITE: &str = "page.searchSite";
const SCROLL_TO: &str = "page.scrollTo";
const PRESS_KEY: &str = "page.pressKey";
const STOP_MONITOR: &str = "page.stopMonitor";
const CLICK_ELEMENT: &str = "page.clickElement";
const START_RECORDING: &str = "vexdom.startRecording";
const STOP_RECORDING: &str = "vexdom.stopRecording";

const SITES: &[(&str, &str)] = &[
    ("google", "google.com"),
    ("google maps", "maps.google.com"),
    ("maps", "maps.google.com"),
    ("google drive", "drive.google.com"),
    ("drive", "drive.google.com"),
    ("google calendar", "calendar.google.com"),
    ("calendar", "calendar.google.com"),
    ("gmail", "mail.google.com"),
    ("youtube", "youtube.com"),
    ("github", "github.com"),
    ("reddit", "reddit.com"),
    ("wikipedia", "wikipedia.org"),
    ("twitter", "twitter.com"),
    ("linkedin", "linkedin.com"),
    ("facebook", "facebook.com"),
    ("instagram", "instagram.com"),
    ("amazon", "amazon.com"),
    ("netflix", "netflix.com"),
    ("stack overflow", "stackoverflow.com"),
    ("stackoverflow", "stackoverflow.com"),
    ("hacker news", "news.ycombinator.com"),
    ("chatgpt", "chatgpt.com"),
    ("claude", "claude.ai"),
];

const SUBMITTING_KEYS: &[&str] = &["enter", "return"];

const KEYS: &[(&str, &str)] = &[
    ("escape", "Escape"),
    ("esc", "Escape"),
    ("tab", "Tab"),
    ("space", " "),
    ("spacebar", " "),
    ("backspace", "Backspace"),
    ("delete", "Delete"),
    ("page up", "PageUp"),
    ("page down", "PageDown"),
    ("home", "Home"),
    ("end", "End"),
    ("up arrow", "ArrowUp"),
    ("down arrow", "ArrowDown"),
    ("left arrow", "ArrowLeft"),
    ("right arrow", "ArrowRight"),
    ("arrow up", "ArrowUp"),
    ("arrow down", "ArrowDown"),
    ("arrow left", "ArrowLeft"),
    ("arrow right", "ArrowRight"),
];

fn lookup(table: &'static [(&str, &str)], key: &str) -> Option<&'static str> {
    table
        .iter()
        .find(|(name, _)| *name == key)
        .map(|(_, value)| *value)
}

static CONSEQUENTIAL: LazyLock<Regex> = LazyLock::new(|| {
    re(
        r"\b(?:buy|buy now|purchase|pay|payment|checkout|place order|order now|delete|remove|send|submit|transfer|withdraw|deposit|unsubscribe|subscribe|donate|book now|confirm)\b",
    )
});
static DEICTIC: LazyLock<Regex> =
    LazyLock::new(|| re(r"^(?:it|that|this|there|them|those|these|him|her|thing|one)$"));
static HTTP_URL: LazyLock<Regex> = LazyLock::new(|| re(r"^https?://\S+$"));
static HTTP_PREFIX: LazyLock<Regex> = LazyLock::new(|| re(r"^https?://"));
static LOCALHOST: LazyLock<Regex> = LazyLock::new(|| re(r"^localhost(?::\d+)?(?:/\S*)?$"));
static DOMAIN: LazyLock<Regex> =
    LazyLock::new(|| re(r"^(?:[a-z0-9](?:[a-z0-9-]*[a-z0-9])?\.)+[a-z]{2,}(?::\d+)?(?:/\S*)?$"));

const MAX_LABEL_TOKENS: usize = 8;

fn built(input: Value, label: String, confidence: f64) -> Option<Built> {
    Some(Built {
        input,
        label,
        confidence,
        risky: false,
    })
}

fn group<'a>(caps: &'a Captures, name: &str) -> &'a str {
    caps.name(name).map_or("", |m| m.as_str())
}

struct Slot {
    value: String,
    confidence: f64,
}

/// A free-text label: at most eight words; long ones and deictic ones (`it`, `that`) score lower.
fn slot(raw: &str) -> Option<Slot> {
    let value = raw.trim();
    if value.is_empty() {
        return None;
    }
    let tokens = value.split_whitespace().count();
    if tokens > MAX_LABEL_TOKENS {
        return None;
    }
    if DEICTIC.is_match(value) {
        return Some(Slot {
            value: value.to_owned(),
            confidence: 0.55,
        });
    }
    let mut confidence = 1.0;
    if tokens > 4 {
        confidence -= 0.12 * (tokens - 4) as f64;
    }
    if value.len() < 2 {
        confidence -= 0.4;
    }
    Some(Slot {
        value: value.to_owned(),
        confidence: f64::max(0.0, confidence),
    })
}

fn to_url(dest: &str) -> Option<String> {
    if HTTP_URL.is_match(dest) {
        return Some(dest.to_owned());
    }
    if LOCALHOST.is_match(dest) {
        return Some(format!("http://{dest}"));
    }
    if DOMAIN.is_match(dest) {
        return Some(format!("https://{dest}"));
    }
    None
}

/// A destination: a known site name, or something that reads as a URL.
fn destination(dest: &str) -> Option<(String, String, f64)> {
    if let Some(site) = lookup(SITES, dest) {
        return Some((format!("https://{site}"), site.to_owned(), 1.0));
    }
    let url = to_url(dest)?;
    let confidence = if HTTP_PREFIX.is_match(dest) {
        1.0
    } else {
        0.97
    };
    Some((url, dest.to_owned(), confidence))
}

const GO_TO: &str = r"(?:go\s+to|goto|open|visit|navigate\s+to|take\s+me\s+to|load|pull\s+up)";

static RULES: LazyLock<Vec<Rule>> = LazyLock::new(|| {
    let key_names = KEYS
        .iter()
        .map(|(name, _)| *name)
        .collect::<Vec<_>>()
        .join("|");
    vec![
        Rule {
            id: "history.back",
            action: NAVIGATE,
            certainty: 0.97,
            pattern: re(r"^(?:go\s+)?back(?:\s+(?:a|one)\s+page)?$"),
            build: |_| built(json!({"action": "back"}), "Go back".into(), 1.0),
        },
        Rule {
            id: "history.forward",
            action: NAVIGATE,
            certainty: 0.97,
            pattern: re(r"^(?:go\s+)?forwards?(?:\s+(?:a|one)\s+page)?$"),
            build: |_| built(json!({"action": "forward"}), "Go forward".into(), 1.0),
        },
        Rule {
            id: "history.reload",
            action: NAVIGATE,
            certainty: 0.97,
            pattern: re(r"^(?:reload|refresh)(?:\s+(?:the|this)\s+page)?$"),
            build: |_| built(json!({"action": "reload"}), "Reload the page".into(), 1.0),
        },
        Rule {
            id: "tab.open",
            action: OPEN_TAB,
            certainty: 0.95,
            pattern: re(&format!(
                r"^{GO_TO}\s+(?:the\s+)?(?<dest>\S+(?:\s+\S+)?)\s+in\s+(?:an?\s+)?(?:new|another|separate)\s+tab$"
            )),
            build: |caps| {
                let (url, name, confidence) = destination(group(caps, "dest"))?;
                built(
                    json!({"url": url}),
                    format!("Open {name} in a new tab"),
                    confidence,
                )
            },
        },
        Rule {
            id: "navigate.url",
            action: NAVIGATE,
            certainty: 0.95,
            pattern: re(&format!(r"^{GO_TO}\s+(?:the\s+)?(?<dest>\S+(?:\s+\S+)?)$")),
            build: |caps| {
                let (url, name, confidence) = destination(group(caps, "dest"))?;
                built(json!({"url": url}), format!("Open {name}"), confidence)
            },
        },
        Rule {
            id: "navigate.search",
            action: NAVIGATE,
            certainty: 0.9,
            pattern: re(
                r"^(?<verb>google|search\s+(?:the\s+web|google)\s+for|web\s+search\s+for)\s+(?<q>.+)$",
            ),
            build: |caps| {
                let verb = group(caps, "verb");
                let query = group(caps, "q").trim();
                if verb == "google" && query.split(' ').count() < 2 {
                    return None;
                }
                built(
                    json!({"url": format!("https://www.google.com/search?q={}", encode_uri_component(query))}),
                    format!("Search the web for \"{query}\""),
                    if verb == "google" { 0.95 } else { 1.0 },
                )
            },
        },
        Rule {
            id: "site.search",
            action: SEARCH_SITE,
            certainty: 0.95,
            pattern: re(
                r"^search\s+(?:(?:on|in)\s+)?(?:(?:this|the)\s+(?:site|website)|here)\s+for\s+(?<q>.+)$",
            ),
            build: |caps| {
                let query = group(caps, "q").trim();
                if query.is_empty() {
                    return None;
                }
                built(
                    json!({"query": query}),
                    format!("Search this site for \"{query}\""),
                    1.0,
                )
            },
        },
        Rule {
            id: "scroll.direction",
            action: SCROLL_TO,
            certainty: 0.95,
            pattern: re(
                r"^(?:scroll|jump|move|go)(?:\s+(?:to|down\s+to|back\s+to|all\s+the\s+way\s+to))?\s+(?:the\s+)?(?<dir>up|down|top|bottom)(?:\s+of\s+(?:the\s+)?page)?$",
            ),
            build: |caps| {
                let dir = group(caps, "dir");
                built(json!({"direction": dir}), format!("Scroll {dir}"), 1.0)
            },
        },
        Rule {
            id: "scroll.page",
            action: SCROLL_TO,
            certainty: 0.95,
            pattern: re(r"^page\s+(?<dir>up|down)$"),
            build: |caps| {
                let dir = group(caps, "dir");
                built(json!({"direction": dir}), format!("Scroll {dir}"), 1.0)
            },
        },
        Rule {
            id: "scroll.element",
            action: SCROLL_TO,
            certainty: 0.85,
            pattern: re(r"^scroll\s+(?:down\s+)?to\s+(?:the\s+)?(?<label>.+)$"),
            build: |caps| {
                let text = slot(group(caps, "label"))?;
                built(
                    json!({"target": {"text": text.value}}),
                    format!("Scroll to {}", text.value),
                    text.confidence,
                )
            },
        },
        Rule {
            id: "key.press",
            action: PRESS_KEY,
            certainty: 0.97,
            pattern: re(&format!(
                r"^(?:press|hit|tap|push)(?:\s+the)?\s+(?<key>{key_names})(?:\s+key)?$"
            )),
            build: |caps| {
                let key = group(caps, "key");
                let dom = lookup(KEYS, key)?;
                built(json!({"key": dom}), format!("Press {key}"), 1.0)
            },
        },
        Rule {
            id: "recording.start",
            action: START_RECORDING,
            certainty: 0.95,
            pattern: re(
                r"^(?:start\s+record(?:ing)?(?:\s+(?:my|this|the))?(?:\s+(?:browsing|browser))?(?:\s+session)?|record(?:ing)?(?:\s+(?:my|this|the))?(?:\s+(?:browsing|browser))?\s+session)$",
            ),
            build: |_| {
                built(
                    json!({"captureValues": false}),
                    "Start recording this session".into(),
                    1.0,
                )
            },
        },
        Rule {
            id: "recording.stop",
            action: STOP_RECORDING,
            certainty: 0.95,
            pattern: re(r"^(?:stop|end|finish)\s+(?:the\s+)?recording$"),
            build: |_| built(json!({}), "Stop recording".into(), 1.0),
        },
        Rule {
            id: "monitor.stop",
            action: STOP_MONITOR,
            certainty: 0.95,
            pattern: re(
                r"^(?:stop|end|quit|cancel)\s+(?:the\s+)?(?:monitor(?:ing)?|watching)(?:\s+(?:the|this|that)\s+\S{1,20})?$",
            ),
            build: |_| built(json!({}), "Stop monitoring".into(), 1.0),
        },
        Rule {
            id: "click.text",
            action: CLICK_ELEMENT,
            certainty: 0.9,
            pattern: re(
                r"^(?:click|tap|press|hit|push)(?:\s+on)?(?:\s+the)?\s+(?<label>.+?)(?:\s+(?<role>button|link|tab|checkbox))?$",
            ),
            build: |caps| {
                let text = slot(group(caps, "label"))?;
                let role = caps.name("role").map(|m| m.as_str());
                if role.is_none() && SUBMITTING_KEYS.contains(&text.value.to_lowercase().as_str()) {
                    return None;
                }
                let target = match role {
                    Some(role) => json!({"text": text.value, "role": role}),
                    None => json!({"text": text.value}),
                };
                Some(Built {
                    input: json!({"target": target}),
                    label: format!("Click {}", text.value),
                    confidence: text.confidence,
                    risky: CONSEQUENTIAL.is_match(&text.value),
                })
            },
        },
    ]
});

static QUESTION: LazyLock<Regex> = LazyLock::new(|| {
    re(
        r"^(?:what|whats|why|how|who|when|where|which|is|are|was|were|do|does|did|can|could|should|would|tell|explain|describe|summarize|summarise|read|find|show|check|look|see|list|compare|help|write|draft|fill)\b",
    )
});
static MULTI_STEP: LazyLock<Regex> =
    LazyLock::new(|| re(r"\b(?:and then|then|after that|and also|followed by)\b"));
static HEDGE: LazyLock<Regex> = LazyLock::new(|| {
    re(r"\b(?:if|unless|whenever|maybe|might|try to|see if|as soon as|in case|otherwise)\b")
});

fn round(n: f64) -> f64 {
    (n * 1000.0).round() / 1000.0
}

fn escalate(reason: &str, score: f64, rule_id: Option<&str>) -> Value {
    let mut out = json!({"decision": "escalate", "reason": reason, "score": score});
    if let Some(rule_id) = rule_id {
        out["ruleId"] = json!(rule_id);
    }
    out
}

/// The TypeScript `Routing`: `{"decision":"act","intent":{ruleId,action,input,label,score}}` or
/// `{"decision":"escalate","reason",score,"ruleId"?}`.
pub fn route_intent(input: &str) -> Value {
    let utterance = normalize(input);
    if utterance.raw.starts_with('@') {
        return escalate("skill-prefix", 0.0, None);
    }
    if utterance.text.is_empty() {
        return escalate("no-match", 0.0, None);
    }
    if utterance.question || QUESTION.is_match(&utterance.text) {
        return escalate("question", 0.0, None);
    }
    if MULTI_STEP.is_match(&utterance.text) {
        return escalate("multi-step", 0.0, None);
    }
    if HEDGE.is_match(&utterance.text) {
        return escalate("conditional", 0.0, None);
    }

    let mut best: Option<(&Rule, f64, Built)> = None;
    for rule in RULES.iter() {
        let Some(caps) = rule.pattern.captures(&utterance.text) else {
            continue;
        };
        let Some(slot) = (rule.build)(&caps) else {
            continue;
        };
        let score = round(rule.certainty * slot.confidence);
        if best.as_ref().is_none_or(|(_, best, _)| score > *best) {
            best = Some((rule, score, slot));
        }
    }

    let Some((rule, score, built)) = best else {
        return escalate("no-match", 0.0, None);
    };
    if built.risky {
        return escalate("consequential", score, Some(rule.id));
    }
    if score < ACT_THRESHOLD {
        return escalate("below-threshold", score, Some(rule.id));
    }
    json!({
        "decision": "act",
        "intent": {
            "ruleId": rule.id,
            "action": rule.action,
            "input": built.input,
            "label": built.label,
            "score": score,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_openers_trailers_and_spoken_domains() {
        let u = normalize("Hey Vexdom, could you go back please?");
        assert_eq!((u.text.as_str(), u.question), ("go back", false));
        assert_eq!(normalize("open github dot com").text, "open github.com");
        assert_eq!(normalize("reload this page.").text, "reload this page");
        assert!(normalize("what is this?").question);
    }

    #[test]
    fn routes_and_escalates() {
        let act = route_intent("go to github");
        assert_eq!(act["decision"], "act");
        assert_eq!(act["intent"]["input"]["url"], "https://github.com");
        assert_eq!(act["intent"]["score"], 0.95);
        let search = route_intent("google best noise cancelling headphones");
        assert_eq!(
            search["intent"]["input"]["url"],
            "https://www.google.com/search?q=best%20noise%20cancelling%20headphones"
        );
        assert_eq!(route_intent("click buy now")["reason"], "consequential");
        assert_eq!(route_intent("@skill do it")["reason"], "skill-prefix");
        assert_eq!(route_intent("open a new tab")["reason"], "no-match");
        assert!(route_intent("open a new tab").get("ruleId").is_none());
    }
}
