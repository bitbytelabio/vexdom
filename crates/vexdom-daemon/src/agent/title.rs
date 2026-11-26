//! A short name for a panel conversation, asked of a one-shot of the configured agent
//! (`title.ts`).

use serde_json::json;
use vexdom_core::config::AgentConfig;
use vexdom_core::log;
use vexdom_core::result::{ActionResult, failure};

use super::abort::{Abort, abort_after};
use super::runner::run_agent_json;
use super::runners::drive::JsonOptions;
use vexdom_pure::text::{flatten, js_trim, utf16_prefix};

const MAX_TITLE_CHARS: usize = 60;

const MAX_MESSAGES: usize = 12;
const MAX_MESSAGE_CHARS: usize = 400;

const NAME_TIMEOUT_MS: u64 = 30_000;

const EMPTY_NAME: &str = "The agent returned an empty name.";

/// `nameSession`: `{title}` for the requests made in one sitting, on `host` when there is one.
pub async fn name_session(
    host: Option<String>,
    messages: Vec<String>,
    config: &AgentConfig,
) -> ActionResult {
    let said: Vec<&str> = messages
        .iter()
        .map(|message| js_trim(message))
        .filter(|message| !message.is_empty())
        .collect();
    let recent: Vec<&str> = said[said.len().saturating_sub(MAX_MESSAGES)..]
        .iter()
        .map(|message| utf16_prefix(message, MAX_MESSAGE_CHARS))
        .collect();
    if recent.is_empty() {
        return failure(
            "INVALID_INPUT",
            "Nothing was said in that conversation yet.",
        );
    }
    let host = host.as_deref().filter(|host| !host.is_empty());

    let signal = Abort::new();
    let options = JsonOptions {
        timed_out: "Naming the conversation took too long.",
        empty: EMPTY_NAME,
        accept: None,
    };
    let asked = run_agent_json(prompt_for(&recent, host), config, &signal, false, &options);
    match abort_after(asked, &signal, NAME_TIMEOUT_MS).await {
        Ok(output) => {
            let title = clamp(&output);
            if title.is_empty() {
                return failure("AGENT_FAILED", EMPTY_NAME);
            }
            log::log(&format!("named session: {title}"));
            Ok(json!({"title": title}))
        }
        Err(error) => {
            log::log(&format!(
                "naming a session failed: {}: {}",
                error.code, error.message
            ));
            Err(error)
        }
    }
}

fn prompt_for(messages: &[&str], host: Option<&str>) -> String {
    let on = host.map_or_else(String::new, |host| format!(" while on {host}"));
    let numbered: Vec<String> = messages
        .iter()
        .enumerate()
        .map(|(index, message)| format!("{}. {message}", index + 1))
        .collect();
    let site = host.map_or_else(
        || "Do not invent a site name.".to_owned(),
        |host| {
            format!(
                "End it with \" — {host}\" so the site is visible at a glance, and count that in the limit."
            )
        },
    );
    format!(
        "Below are the requests someone made to a browser assistant, in order, in one sitting{on}.\n\n\
         {}\n\nReply with a title for this conversation and nothing else — no quotes, no punctuation at \
         the end, no preamble. At most {MAX_TITLE_CHARS} characters, in the style of a short headline \
         (\"Refund a Stripe charge\"). Name what the person was trying to do, not what the assistant did. \
         {site}",
        numbered.join("\n")
    )
}

/// One line, no quotes around it, at most sixty characters.
fn clamp(output: &str) -> String {
    let quotes: &[char] = &['"', '\'', '“', '”'];
    let unquoted = flatten(output);
    let unquoted = js_trim(unquoted.trim_matches(quotes));
    js_trim(utf16_prefix(unquoted, MAX_TITLE_CHARS)).to_owned()
}

#[cfg(all(test, unix))]
mod tests {
    use serde_json::json;
    use vexdom_core::result::ActionError;

    use super::*;
    use crate::agent::test_support::{StandIn, local, scratch};

    fn named(stand_in: &StandIn, messages: &[&str], host: Option<&str>) -> ActionResult {
        let messages: Vec<String> = messages.iter().map(|&message| message.to_owned()).collect();
        let host = host.map(str::to_owned);
        let mut result = None;
        local(async {
            result = Some(name_session(host, messages, &stand_in.config).await);
        });
        result.expect("local runs the naming to its end")
    }

    fn failed(code: &str, message: &str) -> ActionResult {
        Err(ActionError {
            code: code.to_owned(),
            message: message.to_owned(),
        })
    }

    fn agent(name: &str) -> (crate::agent::test_support::Scratch, StandIn) {
        let home = scratch(name);
        let stand_in = StandIn::new(home.path(), "");
        stand_in.answer("Refund a Stripe charge");
        (home, stand_in)
    }

    #[test]
    fn the_agents_title_is_the_conversations_name() {
        let (_home, stand_in) = agent("title-name");
        assert_eq!(
            named(&stand_in, &["refund the last charge"], None),
            Ok(json!({"title": "Refund a Stripe charge"}))
        );
    }

    #[test]
    fn a_conversation_with_nothing_said_yet_is_not_sent_to_the_agent_at_all() {
        let (_home, stand_in) = agent("title-nothing");
        assert_eq!(
            named(&stand_in, &["", "   ", "\n\t"], None),
            failed(
                "INVALID_INPUT",
                "Nothing was said in that conversation yet."
            )
        );
        assert_eq!(stand_in.calls(), 0);
    }

    #[test]
    fn only_the_last_twelve_messages_are_sent_numbered_each_cut_to_400_characters() {
        let (_home, stand_in) = agent("title-twelve");
        let mut messages: Vec<String> = (1..=14).map(|i| format!("message {i}")).collect();
        messages[13] = format!("  {}  ", "x".repeat(500));
        let messages: Vec<&str> = messages.iter().map(String::as_str).collect();
        named(&stand_in, &messages, None).expect("the agent names it");
        let prompt = stand_in.prompt();
        assert_eq!(
            [
                prompt.contains("1. message 3\n"),
                prompt.contains("message 2\n"),
                prompt.contains(&format!("12. {}\n", "x".repeat(400))),
                prompt.contains(&"x".repeat(401)),
            ],
            [true, false, true, false]
        );
    }

    #[test]
    fn on_a_site_the_title_is_asked_to_end_with_it() {
        let (_home, stand_in) = agent("title-site");
        named(&stand_in, &["refund it"], Some("dashboard.stripe.com")).expect("the agent names it");
        let prompt = stand_in.prompt();
        assert!(
            prompt.contains("while on dashboard.stripe.com."),
            "{prompt}"
        );
        assert!(
            prompt.contains("End it with \" — dashboard.stripe.com\""),
            "{prompt}"
        );
    }

    #[test]
    fn off_any_site_the_agent_is_told_not_to_invent_one() {
        let (_home, stand_in) = agent("title-no-site");
        named(&stand_in, &["what is a refund"], None).expect("the agent names it");
        let prompt = stand_in.prompt();
        assert!(prompt.contains("Do not invent a site name."), "{prompt}");
    }

    #[test]
    fn the_whole_prompt_reads_as_one_request() {
        assert_eq!(
            prompt_for(&["refund it", "the last one"], Some("dashboard.stripe.com")),
            "Below are the requests someone made to a browser assistant, in order, in one sitting while on dashboard.stripe.com.\n\n\
             1. refund it\n2. the last one\n\n\
             Reply with a title for this conversation and nothing else — no quotes, no punctuation at the end, no preamble. \
             At most 60 characters, in the style of a short headline (\"Refund a Stripe charge\"). \
             Name what the person was trying to do, not what the assistant did. \
             End it with \" — dashboard.stripe.com\" so the site is visible at a glance, and count that in the limit."
        );
    }

    #[test]
    fn the_answer_is_tidied_quotes_and_extra_whitespace_dropped_and_cut_to_sixty_characters() {
        let (_home, stand_in) = agent("title-tidy");
        stand_in.answer(&format!(
            "  “Compare   three\nlaptop deals {}”  ",
            "and more ".repeat(10)
        ));
        assert_eq!(
            named(&stand_in, &["compare laptops"], None),
            Ok(json!({"title": "Compare three laptop deals and more and more and more and mo"}))
        );
    }

    #[test]
    fn an_answer_that_is_only_quotes_is_no_name() {
        let (_home, stand_in) = agent("title-quotes");
        stand_in.answer("\"\"");
        assert_eq!(
            named(&stand_in, &["hi"], None),
            failed("AGENT_FAILED", "The agent returned an empty name.")
        );
    }

    #[test]
    fn the_agents_failure_is_passed_on_as_it_was_reported() {
        let (home, mut stand_in) = agent("title-missing");
        stand_in.config.claude.bin = home.path().join("no-claude").display().to_string();
        let Err(error) = named(&stand_in, &["hi"], None) else {
            panic!("a missing agent names nothing");
        };
        assert_eq!(error.code, "AGENT_MISSING");
        assert!(error.message.starts_with("Could not run \""), "{error}");
        let logged = std::fs::read_to_string(vexdom_core::paths::log_path()).unwrap();
        assert!(logged.contains("naming a session failed: AGENT_MISSING: Could not run"));
    }

    #[test]
    fn an_agent_that_takes_longer_than_thirty_seconds_is_stopped() {
        let home = scratch("title-slow");
        let stand_in = StandIn::new(home.path(), "sleep 60");
        local(async {
            tokio::time::pause();
            assert_eq!(
                name_session(None, vec!["hi".to_owned()], &stand_in.config).await,
                failed("TIMEOUT", "Naming the conversation took too long.")
            );
        });
    }
}
