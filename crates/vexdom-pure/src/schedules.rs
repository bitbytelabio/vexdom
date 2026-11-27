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
pub const MAX_DATES_PER_YEAR: usize = 12;
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

/// Local wall time on a calendar date; a day past the month's end is its last day (31 → 30 Apr).
fn at_date(year: i16, month: i8, day: i8, clock: (i8, i8), tz: &TimeZone) -> i64 {
    let Ok(first) = Date::new(year, month, 1) else {
        return 0;
    };
    let date = Date::new(year, month, day.min(first.days_in_month())).unwrap_or(first);
    at_clock_time(date, 0, clock, tz)
}

/// The month `offset` months after `year`-`month`.
fn month_after(year: i16, month: i8, offset: i64) -> (i16, i8) {
    let total = i64::from(year) * 12 + i64::from(month - 1) + offset;
    (
        total.div_euclid(12) as i16,
        (total.rem_euclid(12) + 1) as i8,
    )
}

/// The most days a month ever has, Feb 29 included.
fn max_day_of(month: i64) -> i64 {
    Date::new(2024, month as i8, 1).map_or(0, |date| i64::from(date.days_in_month()))
}

fn clocks_of(rule: &Value) -> Vec<(i8, i8)> {
    rule.get("times")
        .and_then(Value::as_array)
        .map(|times| {
            times
                .iter()
                .filter_map(|time| time.as_str().and_then(parse_clock_time))
                .collect()
        })
        .unwrap_or_default()
}

fn whole(value: &Value) -> Option<i64> {
    value
        .as_f64()
        .filter(|n| n.fract() == 0.0)
        .map(|n| n as i64)
}

/// A yearly rule's `{month, day}` dates.
fn dates_of(rule: &Value) -> Vec<(i64, i64)> {
    rule.get("dates")
        .and_then(Value::as_array)
        .map(|dates| {
            dates
                .iter()
                .filter_map(|date| Some((whole(date.get("month")?)?, whole(date.get("day")?)?)))
                .collect()
        })
        .unwrap_or_default()
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
        "monthly" => {
            let days: Vec<i8> = rule
                .get("days")?
                .as_array()?
                .iter()
                .filter_map(whole)
                .filter(|day| (1..=31).contains(day))
                .map(|day| day as i8)
                .collect();
            let clocks = clocks_of(rule);
            if days.is_empty() || clocks.is_empty() {
                return None;
            }
            let start = local(after, tz).date();
            (0..=12).find_map(|offset| {
                let (year, month) = month_after(start.year(), start.month(), offset);
                days.iter()
                    .flat_map(|day| clocks.iter().map(move |clock| (*day, *clock)))
                    .map(|(day, clock)| at_date(year, month, day, clock, tz))
                    .filter(|at| *at > after)
                    .min()
            })
        }
        "yearly" => {
            let dates: Vec<(i8, i8)> = dates_of(rule)
                .into_iter()
                .filter(|(month, day)| (1..=12).contains(month) && (1..=31).contains(day))
                .map(|(month, day)| (month as i8, day as i8))
                .collect();
            let clocks = clocks_of(rule);
            if dates.is_empty() || clocks.is_empty() {
                return None;
            }
            let start = local(after, tz).date().year();
            (0..=1).find_map(|offset| {
                dates
                    .iter()
                    .flat_map(|date| clocks.iter().map(move |clock| (*date, *clock)))
                    .map(|((month, day), clock)| at_date(start + offset, month, day, clock, tz))
                    .filter(|at| *at > after)
                    .min()
            })
        }
        _ => None,
    }
}

/// `1st`, `2nd`, `3rd`, `4th`, `11th`, `21st`.
fn ordinal(day: i64) -> String {
    let suffix = match (day % 10, day % 100) {
        (_, 11..=13) => "th",
        (1, _) => "st",
        (2, _) => "nd",
        (3, _) => "rd",
        _ => "th",
    };
    format!("{day}{suffix}")
}

fn sorted_times(rule: &Value) -> String {
    let mut times: Vec<String> = rule["times"]
        .as_array()
        .map(|t| {
            t.iter()
                .filter_map(|v| v.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default();
    times.sort();
    join_list(&times)
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
        Some("monthly") => {
            let mut days: Vec<i64> = rule["days"]
                .as_array()
                .map(|d| d.iter().filter_map(whole).collect())
                .unwrap_or_default();
            days.sort_unstable();
            let days: Vec<String> = days.into_iter().map(ordinal).collect();
            format!(
                "Monthly on the {} at {}",
                join_list(&days),
                sorted_times(rule)
            )
        }
        Some("yearly") => {
            let mut dates = dates_of(rule);
            dates.sort_unstable();
            let dates: Vec<String> = dates
                .into_iter()
                .filter(|(month, _)| (1..=12).contains(month))
                .map(|(month, day)| format!("{day} {}", MONTH_NAMES[(month - 1) as usize]))
                .collect();
            format!(
                "Every year on {} at {}",
                join_list(&dates),
                sorted_times(rule)
            )
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
            let times = valid_times(raw)?;
            // JS `sort()` compares as strings; single digits sort the same as numbers.
            let mut days: Vec<i64> = days
                .iter()
                .filter_map(|d| d.as_f64().map(|d| d as i64))
                .collect();
            days.sort_unstable();
            Ok(json!({"kind": "weekly", "days": days, "times": times}))
        }
        Some("monthly") => {
            let given = raw
                .get("days")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            if given.is_empty() {
                return Err("Pick at least one day of the month.".into());
            }
            let mut days: Vec<i64> = Vec::new();
            for day in &given {
                match whole(day).filter(|day| (1..=31).contains(day)) {
                    Some(day) if !days.contains(&day) => days.push(day),
                    Some(_) => {}
                    None => return Err("Days of the month are 1 to 31.".into()),
                }
            }
            let times = valid_times(raw)?;
            days.sort_unstable();
            Ok(json!({"kind": "monthly", "days": days, "times": times}))
        }
        Some("yearly") => {
            let given = raw
                .get("dates")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            if given.is_empty() {
                return Err("Pick at least one date.".into());
            }
            let mut dates: Vec<(i64, i64)> = Vec::new();
            for date in &given {
                let month = date.get("month").and_then(whole);
                let day = date.get("day").and_then(whole);
                let (Some(month), Some(day)) = (month, day) else {
                    return Err("That date does not exist.".into());
                };
                if !(1..=12).contains(&month) || day < 1 || day > max_day_of(month) {
                    return Err("That date does not exist.".into());
                }
                if !dates.contains(&(month, day)) {
                    dates.push((month, day));
                }
            }
            if dates.len() > MAX_DATES_PER_YEAR {
                return Err(format!("Up to {MAX_DATES_PER_YEAR} dates a year."));
            }
            let times = valid_times(raw)?;
            dates.sort_unstable();
            let dates: Vec<Value> = dates
                .into_iter()
                .map(|(month, day)| json!({"month": month, "day": day}))
                .collect();
            Ok(json!({"kind": "yearly", "dates": dates, "times": times}))
        }
        _ => Err("A task runs once, every so often, on set days, monthly or yearly.".into()),
    }
}

/// A rule's clock times, deduplicated and sorted, or why they will not do.
fn valid_times(raw: &Value) -> Result<Vec<String>, String> {
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
    let mut times: Vec<String> = times
        .iter()
        .filter_map(|t| t.as_str().map(str::to_owned))
        .collect();
    times.sort();
    Ok(times)
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

    fn berlin(year: i16, month: i8, day: i8, hour: i8) -> i64 {
        jiff::civil::date(year, month, day)
            .at(hour, 0, 0, 0)
            .to_zoned_datetime(TimeZone::get("Europe/Berlin").unwrap())
            .unwrap()
            .timestamp()
            .as_millisecond()
    }

    #[test]
    fn monthly_and_yearly_rules() {
        let tz = TimeZone::get("Europe/Berlin").unwrap();
        let last_day = json!({"kind": "monthly", "days": [31], "times": ["09:00"]});
        assert_eq!(
            next_after(&last_day, berlin(2026, 4, 15, 12), &tz),
            Some(berlin(2026, 4, 30, 9))
        );
        let twice = json!({"kind": "monthly", "days": [1, 15], "times": ["09:00"]});
        assert_eq!(
            next_after(&twice, berlin(2026, 1, 15, 10), &tz),
            Some(berlin(2026, 2, 1, 9))
        );
        let leap =
            json!({"kind": "yearly", "dates": [{"month": 2, "day": 29}], "times": ["09:00"]});
        assert_eq!(
            next_after(&leap, berlin(2026, 3, 1, 12), &tz),
            Some(berlin(2027, 2, 28, 9))
        );
        assert_eq!(
            describe_rule(&twice, &tz),
            "Monthly on the 1st and 15th at 09:00"
        );
        assert_eq!(
            describe_rule(
                &json!({"kind": "monthly", "days": [22, 11, 3], "times": ["09:00"]}),
                &tz
            ),
            "Monthly on the 3rd, 11th and 22nd at 09:00"
        );
        assert_eq!(
            describe_rule(
                &json!({"kind": "yearly", "dates": [{"month": 2, "day": 14}], "times": ["09:00"]}),
                &tz
            ),
            "Every year on 14 Feb at 09:00"
        );
    }

    #[test]
    fn validates_monthly_and_yearly() {
        let now = 1_700_000_000_000;
        assert_eq!(
            validate_rule(
                &json!({"kind": "monthly", "days": [0], "times": ["09:00"]}),
                now,
                None
            ),
            Err("Days of the month are 1 to 31.".into())
        );
        assert_eq!(
            validate_rule(
                &json!({"kind": "yearly", "dates": [{"month": 4, "day": 31}], "times": ["09:00"]}),
                now,
                None
            ),
            Err("That date does not exist.".into())
        );
        assert_eq!(
            validate_rule(
                &json!({"kind": "monthly", "days": [15, 1, 15], "times": ["09:00"]}),
                now,
                None
            ),
            Ok(json!({"kind": "monthly", "days": [1, 15], "times": ["09:00"]}))
        );
        assert!(
            validate_rule(
                &json!({"kind": "yearly", "dates": [{"month": 2, "day": 29}], "times": ["09:00"]}),
                now,
                None
            )
            .is_ok()
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
