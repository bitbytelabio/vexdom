//! `page.solveCaptcha` with an analyst in the loop (`captcha-solver.ts`). The extension does the
//! clicking and the photographing; each time it comes back with an image grid still open, a
//! one-shot analyst session looks at that single picture and is stopped the moment it has
//! answered, and the answer goes back to the extension. The caller sees the outcome or, when no
//! analyst could finish the job, the open grid itself, so it can answer the rest.

use std::rc::Rc;
use std::time::{Duration, Instant};

use futures::future::LocalBoxFuture;
use serde_json::{Map, Value, json};
use vexdom_core::config::AgentConfig;
use vexdom_core::log;
use vexdom_core::result::ActionResult;

use super::abort::{Abort, abort_after};
use super::captcha_analyst::{ChallengeAnswer, analyst_sees, answer_challenge};
use crate::link::Reply;

const MAX_ROUNDS: usize = 10;
const ROUND_FLOOR_MS: f64 = 12_000.0;
const CLICK_RESERVE_MS: f64 = 8_000.0;

/// `SOLVE_CAPTCHA_TIMEOUT_MS` (`src/lib/actions/page/solve-captcha.ts`): the whole solve's budget
/// when the caller names none.
const SOLVE_CAPTCHA_TIMEOUT_MS: u64 = 120_000;

/// One `page.solveCaptcha` call to the extension.
pub type Invoke = Rc<dyn Fn(Value) -> LocalBoxFuture<'static, Reply>>;

/// `solveCaptchaWithAnalyst`, its analyst the configured agent.
pub async fn solve_captcha_with_analyst(
    invoke: Invoke,
    input: Value,
    config: &AgentConfig,
) -> Reply {
    solve_with(&*invoke, input, config, answer_challenge).await
}

/// The solve loop, whoever answers the rounds.
async fn solve_with(
    invoke: &dyn Fn(Value) -> LocalBoxFuture<'static, Reply>,
    input: Value,
    config: &AgentConfig,
    answer: impl AsyncFn(&Value, &AgentConfig, &Abort) -> Option<ChallengeAnswer>,
) -> Reply {
    let asked = match input {
        Value::Object(asked) => asked,
        _ => Map::new(),
    };
    let budget = asked
        .get("timeoutMs")
        .filter(|budget| budget.is_number())
        .cloned()
        .unwrap_or_else(|| json!(SOLVE_CAPTCHA_TIMEOUT_MS));
    let started = Instant::now();
    // What is left of the one budget, in milliseconds; below zero once it is spent.
    let left = || budget.as_f64().unwrap_or_default() - millis(started.elapsed());

    let mut first = asked.clone();
    first.insert("timeoutMs".into(), budget.clone());
    let mut reply = invoke(Value::Object(first)).await;
    if asked.contains_key("tiles") || asked.contains_key("reload") || !analyst_sees(config) {
        return reply;
    }

    let mut rounds = 0;
    let mut stopped = None;
    loop {
        let given = {
            let Some(challenge) = open_challenge(&reply.result) else {
                break;
            };
            if rounds >= MAX_ROUNDS {
                stopped = Some("reached its round limit");
                break;
            }
            let remaining = left();
            if remaining < ROUND_FLOOR_MS {
                stopped = Some("ran out of time");
                break;
            }
            let signal = Abort::new();
            let within = whole_millis(remaining - CLICK_RESERVE_MS);
            abort_after(answer(challenge, config, &signal), &signal, within).await
        };
        let Some(given) = given else {
            stopped = Some("could not answer this round");
            break;
        };
        rounds += 1;
        log::log(&format!(
            "captcha analyst round {rounds}: {}",
            describe_answer(&given)
        ));
        let mut next = match given.to_json() {
            Value::Object(next) => next,
            _ => Map::new(),
        };
        if let Some(wait) = asked.get("waitMs").filter(|wait| wait.is_number()) {
            next.insert("waitMs".into(), wait.clone());
        }
        let timeout = whole_millis(left().max(CLICK_RESERVE_MS));
        next.insert("timeoutMs".into(), json!(timeout));
        reply = invoke(Value::Object(next)).await;
    }
    annotate(reply, rounds, stopped)
}

fn millis(elapsed: Duration) -> f64 {
    elapsed.as_secs_f64() * 1000.0
}

/// Milliseconds as a whole number, as `Date.now()` arithmetic gives them.
fn whole_millis(ms: f64) -> u64 {
    // Saturating: a budget past `u64::MAX` milliseconds is no budget at all.
    ms.max(0.0) as u64
}

fn describe_answer(given: &ChallengeAnswer) -> String {
    match given {
        ChallengeAnswer::Reload => "asked for another challenge".into(),
        ChallengeAnswer::Tiles(tiles) if tiles.is_empty() => "tiles none".into(),
        ChallengeAnswer::Tiles(tiles) => {
            let tiles: Vec<String> = tiles.iter().map(u64::to_string).collect();
            format!("tiles {}", tiles.join(","))
        }
        ChallengeAnswer::Points(points) => format!(
            "{} point{}",
            points.len(),
            if points.len() == 1 { "" } else { "s" }
        ),
    }
}

/// The challenge the extension came back with, while its grid is still open.
fn open_challenge(result: &ActionResult) -> Option<&Value> {
    let data = result.as_ref().ok()?;
    let challenge = data.get("challenge")?;
    let open = data.get("state").and_then(Value::as_str) == Some("challenge")
        && challenge
            .get("image")
            .and_then(Value::as_str)
            .is_some_and(|image| !image.is_empty());
    open.then_some(challenge)
}

fn annotate(mut reply: Reply, rounds: usize, stopped: Option<&str>) -> Reply {
    let annotated = match &reply.result {
        Err(_) => return reply,
        Ok(data) if open_challenge(&reply.result).is_some() => {
            hand_back(data, rounds, stopped.unwrap_or("stopped"))
        }
        Ok(data) if rounds > 0 => with_rounds(data, rounds),
        Ok(_) => return reply,
    };
    reply.result = Ok(annotated);
    reply
}

fn fields(data: &Value) -> Map<String, Value> {
    data.as_object().cloned().unwrap_or_default()
}

fn with_rounds(data: &Value, rounds: usize) -> Value {
    let mut data = fields(data);
    data.insert("analystRounds".into(), json!(rounds));
    Value::Object(data)
}

/// The open challenge, for the caller to answer the rest of.
fn hand_back(data: &Value, rounds: usize, why: &str) -> Value {
    let mut data = fields(data);
    let after = match rounds {
        0 => String::new(),
        1 => " after 1 round".into(),
        rounds => format!(" after {rounds} rounds"),
    };
    let handed = format!(
        "Vexdom’s captcha analyst {why}{after}. The challenge is yours now: look at the image and call page.solveCaptcha with your answer."
    );
    let note = match data.get("note").and_then(Value::as_str) {
        Some(note) if !note.is_empty() => format!("{note} {handed}"),
        _ => handed,
    };
    data.insert("analystRounds".into(), json!(rounds));
    data.insert("note".into(), json!(note));
    Value::Object(data)
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::collections::VecDeque;

    use vexdom_core::config::read_agent_config;
    use vexdom_core::result::failure;

    use super::*;
    use crate::agent::test_support::{local, scratch};

    const IMAGE: &str = "data:image/jpeg;base64,AAAA";

    fn open(prompt: &str) -> ActionResult {
        Ok(json!({
            "vendor": "recaptcha-v2",
            "state": "challenge",
            "challenge": {"kind": "tiles", "prompt": prompt, "tiles": 9, "image": IMAGE},
        }))
    }

    fn open_grid() -> ActionResult {
        open("Select all images with a bus")
    }

    fn solved() -> ActionResult {
        Ok(json!({"vendor": "recaptcha-v2", "state": "solved", "solved": true}))
    }

    /// The extension: each call takes the next result, then reports the widget gone.
    struct Extension {
        results: RefCell<VecDeque<ActionResult>>,
        asked: RefCell<Vec<Value>>,
    }

    fn extension(results: Vec<ActionResult>) -> Rc<Extension> {
        Rc::new(Extension {
            results: RefCell::new(results.into()),
            asked: RefCell::default(),
        })
    }

    fn invoker(extension: &Rc<Extension>) -> impl Fn(Value) -> LocalBoxFuture<'static, Reply> {
        let extension = Rc::clone(extension);
        move |input| {
            extension.asked.borrow_mut().push(input);
            let result = extension
                .results
                .borrow_mut()
                .pop_front()
                .unwrap_or_else(|| failure("CAPTCHA_NOT_FOUND", "gone"));
            Box::pin(std::future::ready(Reply::from(result)))
        }
    }

    /// An analyst that gives the next of `answers` each round, then none.
    struct Analyst {
        answers: RefCell<VecDeque<Option<ChallengeAnswer>>>,
        calls: RefCell<usize>,
    }

    fn analyst(answers: Vec<Option<ChallengeAnswer>>) -> Analyst {
        Analyst {
            answers: RefCell::new(answers.into()),
            calls: RefCell::new(0),
        }
    }

    fn solve(extension: &Rc<Extension>, input: Value, analyst: &Analyst) -> Reply {
        let _home = scratch("captcha-solver");
        let config = read_agent_config();
        let invoke = invoker(extension);
        let mut reply = None;
        local(async {
            let answer = async |_: &Value, _: &AgentConfig, _: &Abort| {
                *analyst.calls.borrow_mut() += 1;
                analyst.answers.borrow_mut().pop_front().flatten()
            };
            reply = Some(solve_with(&invoke, input, &config, answer).await);
        });
        reply.expect("local runs the solve to its end")
    }

    fn tiles(tiles: &[u64]) -> Option<ChallengeAnswer> {
        Some(ChallengeAnswer::Tiles(tiles.to_vec()))
    }

    #[test]
    fn a_checkbox_that_is_accepted_outright_never_wakes_the_analyst() {
        let extension = extension(vec![solved()]);
        let analyst = analyst(Vec::new());
        assert_eq!(solve(&extension, json!({}), &analyst).result, solved());
        assert_eq!(*analyst.calls.borrow(), 0);
    }

    #[test]
    fn answers_each_round_it_is_shown_until_the_widget_is_satisfied() {
        let extension = extension(vec![open_grid(), open_grid(), solved()]);
        let analyst = analyst(vec![tiles(&[2, 5]), tiles(&[])]);
        let data = solve(&extension, json!({}), &analyst)
            .result
            .expect("the widget is satisfied");
        assert_eq!(
            [&data["state"], &data["analystRounds"]],
            [&json!("solved"), &json!(2)]
        );
        let answers: Vec<Value> = extension
            .asked
            .borrow()
            .iter()
            .map(|asked| asked.get("tiles").cloned().unwrap_or(Value::Null))
            .collect();
        assert_eq!(answers, [Value::Null, json!([2, 5]), json!([])]);
    }

    #[test]
    fn each_call_to_the_extension_gets_what_is_left_of_the_one_budget() {
        let extension = extension(vec![open_grid(), solved()]);
        solve(
            &extension,
            json!({"timeoutMs": 60_000}),
            &analyst(vec![tiles(&[1])]),
        );
        let budgets: Vec<u64> = extension
            .asked
            .borrow()
            .iter()
            .map(|asked| asked["timeoutMs"].as_u64().expect("a whole number of ms"))
            .collect();
        let [first, second] = budgets[..] else {
            panic!("two calls, not {budgets:?}");
        };
        assert_eq!(
            [first == 60_000, second <= first, second > 50_000],
            [true, true, true],
            "{budgets:?}"
        );
    }

    #[test]
    fn when_the_analyst_cannot_answer_the_open_challenge_goes_back_with_a_note_saying_so() {
        let extension = extension(vec![open_grid()]);
        let data = solve(&extension, json!({}), &analyst(vec![None]))
            .result
            .expect("the open challenge comes back");
        assert_eq!(
            [&data["state"], &data["challenge"]["image"]],
            [&json!("challenge"), &json!(IMAGE)]
        );
        assert_eq!(
            data["note"],
            "Vexdom’s captcha analyst could not answer this round. The challenge is yours now: look at the image and call page.solveCaptcha with your answer."
        );
        assert_eq!(data["analystRounds"], 0);
    }

    #[test]
    fn gives_up_after_ten_rounds_rather_than_looping_on_a_vendor_that_keeps_asking() {
        let extension = extension((0..12).map(|_| open_grid()).collect());
        let analyst = analyst((0..12).map(|_| tiles(&[1])).collect());
        let data = solve(&extension, json!({}), &analyst)
            .result
            .expect("the open challenge comes back");
        assert_eq!(*analyst.calls.borrow(), 10);
        let note = data["note"].as_str().expect("a note");
        assert!(note.contains("round limit after 10 rounds"), "{note}");
    }

    #[test]
    fn an_answer_the_caller_gave_itself_is_passed_straight_through() {
        let extension = extension(vec![open_grid()]);
        let analyst = analyst(vec![tiles(&[3])]);
        solve(&extension, json!({"tiles": [1, 2]}), &analyst);
        let asked = extension.asked.borrow();
        assert_eq!(asked.len(), 1);
        assert_eq!(asked[0]["tiles"], json!([1, 2]));
        assert_eq!(*analyst.calls.borrow(), 0);
    }

    #[test]
    fn a_note_the_extension_left_comes_ahead_of_the_hand_back_and_wait_ms_is_passed_on() {
        let mut noted = open_grid().expect("a canned grid");
        noted["note"] = json!("The grid changed.");
        let extension = extension(vec![open_grid(), Ok(noted)]);
        let data = solve(
            &extension,
            json!({"waitMs": 500}),
            &analyst(vec![tiles(&[7])]),
        )
        .result
        .expect("the open challenge comes back");
        assert_eq!(data["analystRounds"], 1);
        assert_eq!(
            data["note"],
            "The grid changed. Vexdom’s captcha analyst could not answer this round after 1 round. The challenge is yours now: look at the image and call page.solveCaptcha with your answer."
        );
        assert_eq!(extension.asked.borrow()[1]["waitMs"], 500);
    }
}
