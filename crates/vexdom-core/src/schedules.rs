//! `schedules.json` (`src/daemon/schedules/store.ts`), shared by the daemon and the CLI. The rules
//! and validation are `vexdom_pure::schedules`; here clock times are the system's local time.

use std::path::PathBuf;

use jiff::tz::TimeZone;
use serde_json::{Value, json};
use vexdom_pure::schedules::{MAX_RUNS_KEPT, next_run_for};

use crate::paths;

pub fn schedules_path() -> PathBuf {
    paths::state_dir().join("schedules.json")
}

/// `{paused, tasks}`; a task is kept only if it has a string id, name and url and object rule and job.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct StoredSchedules {
    pub paused: bool,
    pub tasks: Vec<Value>,
}

fn looks_like_task(task: &Value) -> bool {
    ["id", "name", "url"]
        .iter()
        .all(|key| task.get(key).is_some_and(Value::is_string))
        && ["rule", "job"]
            .iter()
            .all(|key| task.get(key).is_some_and(|v| v.is_object() || v.is_null()))
        && task.get("rule").is_some()
        && task.get("job").is_some()
}

pub fn read_schedules() -> StoredSchedules {
    let Some(parsed) = std::fs::read(schedules_path())
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
    else {
        return StoredSchedules::default();
    };
    let tasks = parsed
        .get("tasks")
        .and_then(Value::as_array)
        .map(|tasks| {
            tasks
                .iter()
                .filter(|task| looks_like_task(task))
                .map(|task| {
                    let mut task = task.clone();
                    if !task.get("runs").is_some_and(Value::is_array) {
                        task["runs"] = json!([]);
                    }
                    if task.get("runCount").is_none_or(Value::is_null) {
                        task["runCount"] = json!(0);
                    }
                    task
                })
                .collect()
        })
        .unwrap_or_default();
    StoredSchedules {
        paused: parsed.get("paused") == Some(&Value::Bool(true)),
        tasks,
    }
}

fn write_schedules(state: &StoredSchedules) -> std::io::Result<()> {
    let path = schedules_path();
    let staged = path.with_file_name(format!("schedules.json.{}.tmp", std::process::id()));
    paths::write_private_json(
        &staged,
        &json!({"paused": state.paused, "tasks": state.tasks}),
    )?;
    std::fs::rename(staged, path)
}

/// One synchronous read-modify-write.
pub fn update_schedules<T>(change: impl FnOnce(&mut StoredSchedules) -> T) -> T {
    let mut state = read_schedules();
    let outcome = change(&mut state);
    if let Err(error) = write_schedules(&state) {
        crate::log::log_detail("could not write schedules.json", &error.to_string());
    }
    outcome
}

/// Newest first, at most [`MAX_RUNS_KEPT`].
pub fn log_run(task: &mut Value, run: Value) {
    let mut runs = vec![run];
    if let Some(old) = task.get("runs").and_then(Value::as_array) {
        runs.extend(old.iter().cloned());
    }
    runs.truncate(MAX_RUNS_KEPT);
    task["runs"] = Value::Array(runs);
}

fn next_json(next: Option<i64>) -> Value {
    next.map_or(Value::Null, |at| json!(at))
}

pub fn set_paused(state: &mut StoredSchedules, paused: bool, now: i64) {
    state.paused = paused;
    if paused {
        return;
    }
    for task in &mut state.tasks {
        if task
            .get("nextRunAt")
            .and_then(Value::as_f64)
            .is_some_and(|at| at < now as f64)
        {
            task["nextRunAt"] = next_json(next_run_for(task, now, &TimeZone::system()));
        }
    }
}

pub fn set_enabled(task: &mut Value, enabled: bool, now: i64) {
    task["enabled"] = json!(enabled);
    task["nextRunAt"] = next_json(next_run_for(task, now, &TimeZone::system()));
}

/// An exact id, else a unique prefix.
pub fn find_task<'a>(state: &'a mut StoredSchedules, id_or_prefix: &str) -> Option<&'a mut Value> {
    let id_of = |task: &Value| {
        task.get("id")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned()
    };
    if let Some(at) = state
        .tasks
        .iter()
        .position(|task| id_of(task) == id_or_prefix)
    {
        return state.tasks.get_mut(at);
    }
    let matches: Vec<usize> = state
        .tasks
        .iter()
        .enumerate()
        .filter(|(_, task)| id_of(task).starts_with(id_or_prefix))
        .map(|(at, _)| at)
        .collect();
    match matches.as_slice() {
        [only] => state.tasks.get_mut(*only),
        _ => None,
    }
}

/// Sets `nextRunAt` from the rule.
pub fn with_next_run(task: &mut Value, now: i64) {
    task["nextRunAt"] = next_json(next_run_for(task, now, &TimeZone::system()));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth_store::tests::scratch_home;
    use vexdom_pure::schedules::order_for;

    #[test]
    fn store() {
        let _home = scratch_home("schedules");
        let now = 1_700_000_000_000;
        update_schedules(|state| {
            let mut task = json!({"name": "T", "id": "abc-1", "url": "https://a", "rule": {"kind": "every", "minutes": 10},
                "job": {"kind": "instruction", "text": "x"}, "enabled": true});
            with_next_run(&mut task, now);
            state.tasks.push(task);
        });
        let mut state = read_schedules();
        assert_eq!(state.tasks[0]["runCount"], 0);
        assert_eq!(state.tasks[0]["nextRunAt"], now + 600_000);
        let task = find_task(&mut state, "abc").unwrap();
        log_run(task, json!({"at": now, "outcome": "ok", "headline": "h"}));
        assert_eq!(order_for(task, "r")["previous"], "h");
        set_enabled(task, false, now);
        assert_eq!(task["nextRunAt"], Value::Null);
    }
}
