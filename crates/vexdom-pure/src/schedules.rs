//! Scheduled tasks' rules and validation (`src/lib/schedules/{rule,task}.ts`). Tasks stay JSON
//! objects so their key order and any field this build does not know survive. Clock times are wall
//! times in the time zone the caller passes: the system's in the daemon, the browser's in the
//! extension. The file they live in is `vexdom_core::schedules`.

use jiff::civil::{Date, DateTime, Time};
use jiff::tz::TimeZone;
use jiff::{Timestamp, ToSpan};
use serde_json::{Map, Value, json};

use crate::text::utf16_prefix;

pub const MAX_TASKS: usize = 25;
pub const MIN_EVERY_MINUTES: i64 = 5;
pub const MAX_EVERY_MINUTES: i64 = 7 * 24 * 60;
pub const MAX_TIMES_PER_DAY: usize = 12;
pub const MAX_RUNS_KEPT: usize = 20;
pub const MAX_TASK_RUNS: i64 = 1_000;
pub const MAX_TASK_NAME: usize = 80;
pub const MAX_INSTRUCTION_CHARS: usize = 2_000;
pub const MAX_VARIABLES: usize = 20;
pub const MAX_VARIABLE_CHARS: usize = 200;
pub const MAX_HEADLINE_CHARS: usize = 240;
pub const TASK_APPROVAL_WAIT_MS: i64 = 10 * 60_000;
pub const TASK_RUN_MAX_MS: i64 = 20 * 60_000;

const DAY_NAMES: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
const MONTH_NAMES: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];
const WEEKDAYS: [i64; 7] = [1, 2, 3, 4, 5, 6, 0];
const WORKDAYS: [i64; 5] = [1, 2, 3, 4, 5];

fn len16(text: &str) -> usize {
    text.encode_utf16().count()
}

/// `^([01]\d|2[0-3]):([0-5]\d)$`.
pub fn parse_clock_time(text: &str) -> Option<(i8, i8)> {
    let bytes = text.as_bytes();
    if bytes.len() != 5
        || bytes[2] != b':'
        || !bytes
            .iter()
            .enumerate()
            .all(|(i, b)| i == 2 || b.is_ascii_digit())
    {
        return None;
    }
    let hours = (bytes[0] - b'0') * 10 + (bytes[1] - b'0');
    let minutes = (bytes[3] - b'0') * 10 + (bytes[4] - b'0');
    (hours <= 23 && minutes <= 59).then_some((hours as i8, minutes as i8))
}

fn is_clock_time(value: &Value) -> bool {
    value
        .as_str()
        .is_some_and(|text| parse_clock_time(text).is_some())
}

fn local(at: i64, tz: &TimeZone) -> DateTime {
    Timestamp::from_millisecond(at)
        .unwrap_or(Timestamp::UNIX_EPOCH)
        .to_zoned_datetime(tz.clone())
        .datetime()
}

/// `new Date(y, m, d + offset, h, min).getTime()`: local wall time, days overflowing as JS does.
fn at_clock_time(day: Date, offset_days: i64, (hours, minutes): (i8, i8), tz: &TimeZone) -> i64 {
    let date = day.checked_add(offset_days.days()).unwrap_or(day);
    let wall = date
        .to_datetime(Time::constant(0, 0, 0, 0))
        .with()
        .hour(hours)
        .minute(minutes)
        .build();
    wall.ok()
        .and_then(|wall| wall.to_zoned_datetime(tz.clone()).ok())
        .map_or(0, |zoned| zoned.timestamp().as_millisecond())
}

fn minute_of_day(at: i64, tz: &TimeZone) -> i64 {
    let time = local(at, tz);
    i64::from(time.hour()) * 60 + i64::from(time.minute())
}

fn minute_of_clock((hours, minutes): (i8, i8)) -> i64 {
    i64::from(hours) * 60 + i64::from(minutes)
}

pub fn inside_window(at: i64, window: &Value, tz: &TimeZone) -> bool {
    let from = window
        .get("from")
        .and_then(Value::as_str)
        .and_then(parse_clock_time);
    let to = window
        .get("to")
        .and_then(Value::as_str)
        .and_then(parse_clock_time);
    let (Some(from), Some(to)) = (from, to) else {
        return true;
    };
    let now = minute_of_day(at, tz);
    let (start, end) = (minute_of_clock(from), minute_of_clock(to));
    if start <= end {
        now >= start && now < end
    } else {
        now >= start || now < end
    }
}

fn next_window_start(after: i64, window: &Value, tz: &TimeZone) -> i64 {
    let from = window
        .get("from")
        .and_then(Value::as_str)
        .and_then(parse_clock_time)
        .unwrap_or((0, 0));
    let day = local(after, tz).date();
    let today = at_clock_time(day, 0, from, tz);
    if today > after {
        today
    } else {
        at_clock_time(day, 1, from, tz)
    }
}

fn weekday(at: i64, tz: &TimeZone) -> i64 {
    i64::from(local(at, tz).weekday().to_sunday_zero_offset())
}

/// The first run of a rule strictly after `after`, or `None` when it never runs again.
pub fn next_after(rule: &Value, after: i64, tz: &TimeZone) -> Option<i64> {
    match rule.get("kind")?.as_str()? {
        "once" => {
            let at = rule.get("at")?.as_f64()? as i64;
            (at > after).then_some(at)
        }
        "every" => {
            let minutes = rule.get("minutes")?.as_f64()? as i64;
            let candidate = after + minutes * 60_000;
            match rule.get("window") {
                Some(window) if !window.is_null() && !inside_window(candidate, window, tz) => {
                    Some(next_window_start(candidate, window, tz))
                }
                _ => Some(candidate),
            }
        }
        "weekly" => {
            let days: Vec<i64> = rule
                .get("days")?
                .as_array()?
                .iter()
                .filter_map(Value::as_i64)
                .collect();
            let clocks: Vec<(i8, i8)> = rule
                .get("times")?
                .as_array()?
                .iter()
                .filter_map(|time| time.as_str().and_then(parse_clock_time))
                .collect();
            if days.is_empty() || clocks.is_empty() {
                return None;
            }
            let start = local(after, tz).date();
            (0..=7).find_map(|offset| {
                clocks
                    .iter()
                    .map(|time| at_clock_time(start, offset, *time, tz))
                    .filter(|at| *at > after && days.contains(&weekday(*at, tz)))
                    .min()
            })
        }
        _ => None,
    }
}

pub fn upcoming(rule: &Value, from: i64, count: usize, tz: &TimeZone) -> Vec<i64> {
    let mut runs = Vec::new();
    let mut cursor = from;
    while runs.len() < count {
        let Some(next) = next_after(rule, cursor, tz) else {
            break;
        };
        runs.push(next);
        cursor = next;
    }
    runs
}

pub fn clock_time_of(at: i64, tz: &TimeZone) -> String {
    let time = local(at, tz);
    format!("{:02}:{:02}", time.hour(), time.minute())
}

fn same_set(a: &[i64], b: &[i64]) -> bool {
    a.len() == b.len() && b.iter().all(|day| a.contains(day))
}

pub fn describe_days(days: &[i64]) -> String {
    if same_set(days, &WEEKDAYS) {
        return "Every day".into();
    }
    if same_set(days, &WORKDAYS) {
        return "Weekdays".into();
    }
    if same_set(days, &[0, 6]) {
        return "Weekends".into();
    }
    WEEKDAYS
        .iter()
        .filter(|day| days.contains(day))
        .map(|day| DAY_NAMES[*day as usize])
        .collect::<Vec<_>>()
        .join(", ")
}

pub fn join_list(items: &[String]) -> String {
    match items {
        [] => String::new(),
        [one] => one.clone(),
        [head @ .., last] => format!("{} and {last}", head.join(", ")),
    }
}

pub fn describe_interval(minutes: i64) -> String {
    if minutes % (24 * 60) == 0 {
        return if minutes == 24 * 60 {
            "Every day".into()
        } else {
            format!("Every {} days", minutes / (24 * 60))
        };
    }
    if minutes % 60 == 0 {
        return if minutes == 60 {
            "Every hour".into()
        } else {
            format!("Every {} h", minutes / 60)
        };
    }
    format!("Every {minutes} min")
}

pub fn describe_moment(at: i64, tz: &TimeZone) -> String {
    let date = local(at, tz);
    format!(
        "{} {} {} at {}",
        DAY_NAMES[weekday(at, tz) as usize],
        date.day(),
        MONTH_NAMES[(date.month() - 1) as usize],
        clock_time_of(at, tz)
    )
}

pub fn describe_rule(rule: &Value, tz: &TimeZone) -> String {
    match rule.get("kind").and_then(Value::as_str) {
        Some("once") => format!(
            "Once, {}",
            describe_moment(
                rule.get("at").and_then(Value::as_f64).unwrap_or(0.0) as i64,
                tz
            )
        ),
        Some("every") => {
            let interval =
                describe_interval(rule.get("minutes").and_then(Value::as_i64).unwrap_or(0));
            match rule.get("window") {
                Some(window) if window.is_object() => format!(
                    "{interval}, {}–{}",
                    window["from"].as_str().unwrap_or(""),
                    window["to"].as_str().unwrap_or("")
                ),
                _ => interval,
            }
        }
        Some("weekly") => {
            let days: Vec<i64> = rule["days"]
                .as_array()
                .map(|d| d.iter().filter_map(Value::as_i64).collect())
                .unwrap_or_default();
            let mut times: Vec<String> = rule["times"]
                .as_array()
                .map(|t| {
                    t.iter()
                        .filter_map(|v| v.as_str().map(str::to_owned))
                        .collect()
                })
                .unwrap_or_default();
            times.sort();
            format!("{} at {}", describe_days(&days), join_list(&times))
        }
        _ => String::new(),
    }
}

fn check_job(raw: &Value) -> Result<Value, String> {
    if !raw.is_object() {
        return Err("Say what the task should do.".into());
    }
    match raw.get("kind").and_then(Value::as_str) {
        Some("instruction") => {
            let text = raw
                .get("text")
                .and_then(Value::as_str)
                .map(str::trim)
                .unwrap_or("");
            if text.is_empty() {
                return Err("Write the instruction the agent should follow.".into());
            }
            if len16(text) > MAX_INSTRUCTION_CHARS {
                return Err(format!(
                    "Keep the instruction under {MAX_INSTRUCTION_CHARS} characters."
                ));
            }
            Ok(json!({"kind": "instruction", "text": text}))
        }
        Some("recording") => {
            let Some(recording_id) = raw
                .get("recordingId")
                .and_then(Value::as_str)
                .filter(|id| !id.is_empty())
            else {
                return Err("Pick the recording to replay.".into());
            };
            let name = raw
                .get("name")
                .and_then(Value::as_str)
                .map_or(String::new(), |name| {
                    utf16_prefix(name.trim(), MAX_TASK_NAME).to_owned()
                });
            let entries = raw
                .get("variables")
                .and_then(Value::as_object)
                .cloned()
                .unwrap_or_default();
            if entries.len() > MAX_VARIABLES {
                return Err(format!("A recording takes at most {MAX_VARIABLES} values."));
            }
            if entries.values().any(|value| {
                value
                    .as_str()
                    .is_none_or(|text| len16(text) > MAX_VARIABLE_CHARS)
            }) {
                return Err(format!(
                    "Each value the recording needs is text of at most {MAX_VARIABLE_CHARS} characters."
                ));
            }
            let mut job = json!({"kind": "recording", "recordingId": recording_id, "name": name});
            if !entries.is_empty() {
                job["variables"] = Value::Object(entries);
            }
            Ok(job)
        }
        _ => Err("A task either follows an instruction or replays a recording.".into()),
    }
}

/// A time already in the past is refused only when it is new.
pub fn validate_rule(raw: &Value, now: i64, stored: Option<&Value>) -> Result<Value, String> {
    if !raw.is_object() {
        return Err("Say when the task should run.".into());
    }
    match raw.get("kind").and_then(Value::as_str) {
        Some("once") => {
            let Some(at) = raw
                .get("at")
                .and_then(Value::as_f64)
                .filter(|at| at.is_finite())
            else {
                return Err("Pick the date and time to run it.".into());
            };
            let kept = stored.is_some_and(|stored| {
                stored.get("kind").and_then(Value::as_str) == Some("once")
                    && stored.get("at").and_then(Value::as_f64) == Some(at)
            });
            if at <= now as f64 && !kept {
                return Err("That time has already passed.".into());
            }
            Ok(json!({"kind": "once", "at": raw["at"]}))
        }
        Some("every") => {
            let Some(minutes) = raw
                .get("minutes")
                .and_then(Value::as_f64)
                .filter(|m| m.fract() == 0.0)
            else {
                return Err("Say how often to repeat it.".into());
            };
            let minutes = minutes as i64;
            if minutes < MIN_EVERY_MINUTES {
                return Err(format!(
                    "Repeat it at most every {MIN_EVERY_MINUTES} minutes."
                ));
            }
            if minutes > MAX_EVERY_MINUTES {
                return Err("Repeat it at least once a week.".into());
            }
            let Some(window) = raw.get("window") else {
                return Ok(json!({"kind": "every", "minutes": minutes}));
            };
            if !window.is_object()
                || !window.get("from").is_some_and(is_clock_time)
                || !window.get("to").is_some_and(is_clock_time)
            {
                return Err("Give the window as two times, like 08:00 and 22:00.".into());
            }
            if window["from"] == window["to"] {
                return Err("The window has to start and end at different times.".into());
            }
            Ok(
                json!({"kind": "every", "minutes": minutes, "window": {"from": window["from"], "to": window["to"]}}),
            )
        }
        Some("weekly") => {
            let mut days: Vec<Value> = Vec::new();
            for day in raw
                .get("days")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default()
            {
                if !days.contains(&day) {
                    days.push(day);
                }
            }
            let weekday_ok = |day: &Value| {
                day.as_f64()
                    .is_some_and(|d| d.fract() == 0.0 && (0.0..=6.0).contains(&d))
            };
            if days.is_empty() || !days.iter().all(weekday_ok) {
                return Err("Pick at least one day.".into());
            }
            let mut times: Vec<Value> = Vec::new();
            for time in raw
                .get("times")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default()
            {
                if !times.contains(&time) {
                    times.push(time);
                }
            }
            if times.is_empty() || !times.iter().all(is_clock_time) {
                return Err("Give at least one time, like 09:00.".into());
            }
            if times.len() > MAX_TIMES_PER_DAY {
                return Err(format!("Pick at most {MAX_TIMES_PER_DAY} times a day."));
            }
            // JS `sort()` compares as strings; single digits sort the same as numbers.
            let mut days: Vec<i64> = days
                .iter()
                .filter_map(|d| d.as_f64().map(|d| d as i64))
                .collect();
            days.sort_unstable();
            let mut times: Vec<String> = times
                .iter()
                .filter_map(|t| t.as_str().map(str::to_owned))
                .collect();
            times.sort();
            Ok(json!({"kind": "weekly", "days": days, "times": times}))
        }
        _ => Err("A task runs once, every so often, or on set days.".into()),
    }
}

fn is_web_url(value: &Value) -> bool {
    value
        .as_str()
        .and_then(|text| url::Url::parse(text).ok())
        .is_some_and(|url| matches!(url.scheme(), "http" | "https"))
}

/// `validateTask`: `Ok(draft)` or `Err((field, message))`.
pub fn validate_task(
    input: &Value,
    now: i64,
    stored: Option<&Value>,
) -> Result<Map<String, Value>, (String, String)> {
    let invalid = |field: &str, message: String| Err((field.to_owned(), message));
    if !input.is_object() {
        return invalid("task", "Nothing to save.".into());
    }
    let name = input
        .get("name")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or("");
    if name.is_empty() {
        return invalid("name", "Give the task a name.".into());
    }
    if len16(name) > MAX_TASK_NAME {
        return invalid(
            "name",
            format!("Keep the name under {MAX_TASK_NAME} characters."),
        );
    }
    let job = match check_job(input.get("job").unwrap_or(&Value::Null)) {
        Ok(job) => job,
        Err(message) => return invalid("job", message),
    };
    let url = input.get("url").cloned().unwrap_or(Value::Null);
    if !is_web_url(&url) {
        return invalid(
            "url",
            "Start it on a web page — an http or https address.".into(),
        );
    }
    let rule = match validate_rule(
        input.get("rule").unwrap_or(&Value::Null),
        now,
        stored.and_then(|s| s.get("rule")),
    ) {
        Ok(rule) => rule,
        Err(message) => return invalid("rule", message),
    };
    let missed = input
        .get("missed")
        .and_then(Value::as_str)
        .filter(|policy| matches!(*policy, "runOnce" | "skip"))
        .unwrap_or("runOnce");
    let notify = input
        .get("notify")
        .and_then(Value::as_str)
        .filter(|policy| matches!(*policy, "always" | "failure" | "never"))
        .unwrap_or("always");
    let max_runs = input.get("maxRuns");
    if let Some(max_runs) = max_runs
        && !max_runs
            .as_f64()
            .is_some_and(|n| n.fract() == 0.0 && n >= 1.0 && n <= MAX_TASK_RUNS as f64)
    {
        return invalid(
            "maxRuns",
            format!("Stop after between 1 and {MAX_TASK_RUNS} runs."),
        );
    }
    let until = input.get("until");
    if let Some(until) = until {
        let kept = stored.and_then(|s| s.get("until")) == Some(until);
        if !until.is_number() || (until.as_f64().unwrap_or(0.0) <= now as f64 && !kept) {
            return invalid("until", "The end date has already passed.".into());
        }
    }
    let browser = input.get("browser");
    if browser.is_some_and(|b| !b.is_string()) {
        return invalid("browser", "Pick a browser.".into());
    }
    let mut draft = Map::new();
    if let Some(id) = input
        .get("id")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
    {
        draft.insert("id".into(), json!(id));
    }
    draft.insert("name".into(), json!(name));
    draft.insert("job".into(), job);
    draft.insert("url".into(), url);
    if let Some(browser) = browser.and_then(Value::as_str).filter(|b| !b.is_empty()) {
        draft.insert("browser".into(), json!(browser));
    }
    draft.insert("rule".into(), rule);
    draft.insert("missed".into(), json!(missed));
    draft.insert("notify".into(), json!(notify));
    draft.insert(
        "keepTab".into(),
        json!(input.get("keepTab") == Some(&Value::Bool(true))),
    );
    if let Some(max_runs) = max_runs {
        draft.insert("maxRuns".into(), max_runs.clone());
    }
    if let Some(until) = until {
        draft.insert("until".into(), until.clone());
    }
    draft.insert(
        "enabled".into(),
        json!(input.get("enabled") != Some(&Value::Bool(false))),
    );
    Ok(draft)
}

/// When a task next runs, or `None`.
pub fn next_run_for(task: &Value, after: i64, tz: &TimeZone) -> Option<i64> {
    if task.get("enabled") != Some(&Value::Bool(true)) {
        return None;
    }
    let run_count = task.get("runCount").and_then(Value::as_i64).unwrap_or(0);
    if task
        .get("maxRuns")
        .and_then(Value::as_i64)
        .is_some_and(|max| run_count >= max)
    {
        return None;
    }
    let next = next_after(task.get("rule")?, after, tz)?;
    match task.get("until").and_then(Value::as_f64) {
        Some(until) if next as f64 > until => None,
        _ => Some(next),
    }
}

/// The order the extension runs: `{id, runId, name, job, url, keepTab, notify, previous?}`.
pub fn order_for(task: &Value, run_id: &str) -> Value {
    let mut order = json!({
        "id": task["id"], "runId": run_id, "name": task["name"], "job": task["job"],
        "url": task["url"], "keepTab": task["keepTab"], "notify": task["notify"],
    });
    let previous = task.get("runs").and_then(Value::as_array).and_then(|runs| {
        runs.iter()
            .find(|run| {
                run.get("outcome").and_then(Value::as_str) == Some("ok")
                    && run
                        .get("headline")
                        .and_then(Value::as_str)
                        .is_some_and(|h| !h.is_empty())
            })
            .and_then(|run| run.get("headline").cloned())
    });
    if let Some(previous) = previous {
        order["previous"] = previous;
    }
    order
}

/// The last meaningful line of an answer, unformatted, as a run's headline.
pub fn headline_of(text: &str) -> Option<String> {
    static BULLET: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(r"^\s*(?:[#>]+|[-*+]|[0-9]+\.)\s+").expect("compiles")
    });
    let last = text
        .split('\n')
        .map(|line| {
            BULLET
                .replace(line, "")
                .replace("**", "")
                .replace("__", "")
                .replace('`', "")
                .trim()
                .to_owned()
        })
        .rfind(|line| !line.is_empty())?;
    if len16(&last) > MAX_HEADLINE_CHARS {
        let cut = utf16_prefix(&last, MAX_HEADLINE_CHARS - 1);
        return Some(format!("{cut}…"));
    }
    Some(last)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rules() {
        let now = 1_700_000_000_000;
        let tz = TimeZone::get("Europe/Berlin").unwrap();
        assert_eq!(
            next_after(&json!({"kind": "once", "at": now + 5}), now, &tz),
            Some(now + 5)
        );
        assert_eq!(
            next_after(&json!({"kind": "once", "at": now}), now, &tz),
            None
        );
        assert_eq!(
            next_after(&json!({"kind": "every", "minutes": 10}), now, &tz),
            Some(now + 600_000)
        );
        let weekly = json!({"kind": "weekly", "days": [0, 1, 2, 3, 4, 5, 6], "times": ["09:00"]});
        let next = next_after(&weekly, now, &tz).unwrap();
        assert!(next > now && next - now <= 24 * 3_600_000);
        assert_eq!(clock_time_of(next, &tz), "09:00");
        assert_eq!(upcoming(&weekly, now, 3, &tz).len(), 3);
        let window = json!({"from": "22:00", "to": "06:00"});
        let inside = next_after(
            &json!({"kind": "every", "minutes": 60, "window": window}),
            now,
            &tz,
        )
        .unwrap();
        assert!(inside_window(inside, &window, &tz));
        assert_eq!(describe_rule(&weekly, &tz), "Every day at 09:00");
        assert_eq!(
            describe_rule(
                &json!({"kind": "weekly", "days": [6, 0], "times": ["10:00", "08:30"]}),
                &tz
            ),
            "Weekends at 08:30 and 10:00"
        );
        assert_eq!(describe_interval(90), "Every 90 min");
        assert_eq!(describe_interval(120), "Every 2 h");
    }

    #[test]
    fn validates() {
        let now = 1_700_000_000_000;
        let ok = validate_task(&json!({"name": " T ", "job": {"kind": "instruction", "text": "do"}, "url": "https://a.com",
            "rule": {"kind": "weekly", "days": [3, 1, 1], "times": ["09:00"]}}), now, None).unwrap();
        assert_eq!(
            Value::Object(ok.clone()),
            json!({"name": "T", "job": {"kind": "instruction", "text": "do"}, "url": "https://a.com",
            "rule": {"kind": "weekly", "days": [1, 3], "times": ["09:00"]}, "missed": "runOnce", "notify": "always", "keepTab": false, "enabled": true})
        );
        let err = validate_task(
            &json!({"name": "T", "job": {"kind": "instruction", "text": "do"}, "url": "ftp://a",
            "rule": {"kind": "every", "minutes": 1}}),
            now,
            None,
        )
        .unwrap_err();
        assert_eq!(err.0, "url");
        let err = validate_task(
            &json!({"name": "T", "job": {"kind": "instruction", "text": "do"}, "url": "https://a",
            "rule": {"kind": "every", "minutes": 1}}),
            now,
            None,
        )
        .unwrap_err();
        assert_eq!(err.1, "Repeat it at most every 5 minutes.");
        assert_eq!(
            headline_of("# Done\n- **All** `good`\n").as_deref(),
            Some("All good")
        );
    }

    #[test]
    fn berlin_daylight_saving() {
        // 2026-03-29 02:00 → 03:00 in Berlin: a 09:00 run stays at 09:00 wall time across it.
        let tz = TimeZone::get("Europe/Berlin").unwrap();
        let saturday_noon = jiff::civil::date(2026, 3, 28)
            .at(12, 0, 0, 0)
            .to_zoned_datetime(tz.clone())
            .unwrap()
            .timestamp()
            .as_millisecond();
        let daily = json!({"kind": "weekly", "days": [0, 1, 2, 3, 4, 5, 6], "times": ["09:00"]});
        let next = next_after(&daily, saturday_noon, &tz).unwrap();
        assert_eq!(next - saturday_noon, 20 * 3_600_000);
        assert_eq!(describe_moment(next, &tz), "Sun 29 Mar at 09:00");
    }
}
