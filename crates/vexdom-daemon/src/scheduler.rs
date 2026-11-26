//! Scheduled tasks (`src/daemon/schedules/scheduler.ts`): one clock, re-armed to the next due task
//! (at most a minute away), dispatching `runTask` to a browser and logging what became of it.

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};
use std::time::Duration;

use jiff::tz::TimeZone;
use serde_json::{Value, json};
use tokio::sync::Notify;
use vexdom_core::downloads::uuid_v4;
use vexdom_core::log;
use vexdom_core::result::{ActionResult, failure};
use vexdom_core::schedules::{
    log_run, read_schedules, set_paused, update_schedules, with_next_run,
};
use vexdom_pure::schedules::{MAX_TASKS, TASK_RUN_MAX_MS, next_after, order_for, validate_task};

use crate::link::Link;
use crate::{Daemon, Inner};

const CLOCK_MAX_WAIT_MS: i64 = 60_000;
const FIRST_WAKE_MS: u64 = 15_000;
const LINK_SETTLE_MS: u64 = 2_000;
const LATE_AFTER_MS: i64 = 2 * CLOCK_MAX_WAIT_MS;
const REPORT_GRACE_MS: i64 = 5 * 60_000;
const MAX_MISSED_COUNTED: usize = 1_000;

struct InFlight {
    run_id: String,
    link_id: String,
    started_at: i64,
}

pub(crate) struct Scheduler {
    inner: Weak<Inner>,
    in_flight: RefCell<Vec<(String, InFlight)>>,
    published: RefCell<String>,
    rearm: Rc<Notify>,
    next_wait: Cell<Option<u64>>,
}

fn plural(count: usize, noun: &str) -> String {
    format!("{count} {noun}{}", if count == 1 { "" } else { "s" })
}

fn not_found(task_id: &str) -> ActionResult {
    failure(
        "TASK_NOT_FOUND",
        format!(
            "No scheduled task with id “{task_id}”. It may have been deleted from another browser or the CLI."
        ),
    )
}

fn str_field<'a>(task: &'a Value, key: &str) -> &'a str {
    task.get(key).and_then(Value::as_str).unwrap_or("")
}

fn next_run_at(task: &Value) -> Option<i64> {
    task.get("nextRunAt")
        .and_then(Value::as_f64)
        .map(|at| at as i64)
}

fn count_missed(task: &Value, now: i64) -> usize {
    let mut count = 0;
    let mut cursor = next_run_at(task);
    let tz = TimeZone::system();
    while let Some(at) = cursor {
        if at > now || count >= MAX_MISSED_COUNTED {
            break;
        }
        count += 1;
        cursor = task.get("rule").and_then(|rule| next_after(rule, at, &tz));
    }
    count
}

impl Scheduler {
    pub(crate) fn new(inner: Weak<Inner>) -> Self {
        Self {
            inner,
            in_flight: RefCell::new(Vec::new()),
            published: RefCell::new(String::new()),
            rearm: Rc::new(Notify::new()),
            next_wait: Cell::new(Some(FIRST_WAKE_MS)),
        }
    }

    fn daemon(&self) -> Option<Daemon> {
        self.inner.upgrade().map(Daemon)
    }

    fn link_for(&self, browser: Option<&str>) -> Option<Rc<Link>> {
        let daemon = self.daemon()?;
        let link = match browser.filter(|b| !b.is_empty()) {
            Some(browser) => daemon.link(browser),
            None => daemon.active_link(),
        };
        link.filter(|link| link.is_open())
    }

    pub(crate) fn start(&self) {
        let weak = self.inner.clone();
        tokio::task::spawn_local(async move {
            loop {
                let Some(inner) = weak.upgrade() else { return };
                let rearm = Rc::clone(&inner.scheduler.rearm);
                let wait = inner
                    .scheduler
                    .next_wait
                    .take()
                    .unwrap_or_else(|| inner.scheduler.until_due());
                drop(inner);
                let rearmed = rearm.notified();
                tokio::pin!(rearmed);
                rearmed.as_mut().enable();
                tokio::select! {
                    () = tokio::time::sleep(Duration::from_millis(wait)) => {
                        if let Some(inner) = weak.upgrade() {
                            inner.scheduler.wake();
                        }
                    }
                    () = &mut rearmed => {}
                }
            }
        });
    }

    fn arm(&self, wait: Option<u64>) {
        self.next_wait.set(wait);
        self.rearm.notify_one();
    }

    fn waits_for_browser(&self, task: &Value) -> bool {
        str_field(task, "missed") == "runOnce"
            && self
                .link_for(task.get("browser").and_then(Value::as_str))
                .is_none()
    }

    fn fires_at(&self, task: &Value, now: i64) -> Option<i64> {
        let at = next_run_at(task)?;
        (!(at <= now && self.waits_for_browser(task))).then_some(at)
    }

    fn until_due(&self) -> u64 {
        let now = log::now_ms();
        let state = read_schedules();
        let due = if state.paused {
            None
        } else {
            state
                .tasks
                .iter()
                .filter_map(|task| self.fires_at(task, now))
                .min()
        };
        let until = due.map_or(CLOCK_MAX_WAIT_MS, |at| (at - now).max(0));
        until.min(CLOCK_MAX_WAIT_MS) as u64
    }

    fn running(&self) -> Value {
        let mut running = serde_json::Map::new();
        for (id, flight) in self.in_flight.borrow().iter() {
            running.insert(id.clone(), json!(flight.started_at));
        }
        Value::Object(running)
    }

    /// `{paused, tasks, running}`.
    pub(crate) fn list(&self) -> Value {
        let state = read_schedules();
        json!({"paused": state.paused, "tasks": state.tasks, "running": self.running()})
    }

    fn publish(&self) {
        let current = self.list();
        *self.published.borrow_mut() = current.to_string();
        if let Some(daemon) = self.daemon() {
            for link in daemon.open_links() {
                link.task_list(&Ok(current.clone()));
            }
        }
    }

    fn wake(&self) {
        let now = log::now_ms();
        self.expire_silent(now);
        let state = read_schedules();
        if !state.paused {
            for task in &state.tasks {
                if self.fires_at(task, now).is_some_and(|at| at <= now) {
                    self.fire(str_field(task, "id"), now);
                }
            }
        }
        let now = self.list().to_string();
        let changed = now != *self.published.borrow();
        if changed {
            self.publish();
        }
    }

    fn in_flight_has(&self, task_id: &str) -> bool {
        self.in_flight.borrow().iter().any(|(id, _)| id == task_id)
    }

    fn take_flight(&self, task_id: &str) -> Option<InFlight> {
        let mut flights = self.in_flight.borrow_mut();
        let at = flights.iter().position(|(id, _)| id == task_id)?;
        Some(flights.remove(at).1)
    }

    fn fire(&self, task_id: &str, now: i64) {
        let plan = update_schedules(|state| {
            let task = state
                .tasks
                .iter_mut()
                .find(|task| str_field(task, "id") == task_id)?;
            let next = next_run_at(task)?;
            if next > now {
                return None;
            }
            let settle = |task: &mut Value, run: Value| {
                log_run(task, run);
                with_next_run(task, now);
                None
            };
            if now - next > LATE_AFTER_MS {
                let missed = plural(count_missed(task, now), "run");
                let run = json!({"at": next, "outcome": "missed",
                    "reason": format!("Missed {missed} while nothing could run it — the browser was closed, the computer asleep, or no daemon was running.")});
                if str_field(task, "missed") == "skip" {
                    return settle(task, run);
                }
                log_run(task, run);
            }
            if self.in_flight_has(task_id) {
                return settle(
                    task,
                    json!({"at": now, "outcome": "skipped", "reason": "The previous run was still going."}),
                );
            }
            let browser = task
                .get("browser")
                .and_then(Value::as_str)
                .map(str::to_owned);
            let Some(link) = self.link_for(browser.as_deref()) else {
                let reason = if browser.is_some() {
                    "The browser it runs in was not connected."
                } else {
                    "No browser was connected."
                };
                return settle(
                    task,
                    json!({"at": now, "outcome": "missed", "reason": reason}),
                );
            };
            let count = task.get("runCount").and_then(Value::as_i64).unwrap_or(0) + 1;
            task["runCount"] = json!(count);
            with_next_run(task, now);
            let run_id = uuid_v4();
            self.in_flight.borrow_mut().push((
                task_id.to_owned(),
                InFlight {
                    run_id: run_id.clone(),
                    link_id: link.install_id.clone(),
                    started_at: now,
                },
            ));
            Some((order_for(task, &run_id), link))
        });
        self.publish();
        if let Some((order, link)) = plan {
            self.dispatch(order, link);
        }
    }

    fn dispatch(&self, order: Value, link: Rc<Link>) {
        log::log(&format!(
            "task “{}” → {}",
            str_field(&order, "name"),
            link.label()
        ));
        let weak = self.inner.clone();
        tokio::task::spawn_local(async move {
            let started = link.run_task(&order).await;
            let Err(error) = started else { return };
            let Some(inner) = weak.upgrade() else { return };
            let scheduler = &inner.scheduler;
            let task_id = str_field(&order, "id");
            let run_id = str_field(&order, "runId");
            let flight = scheduler
                .in_flight
                .borrow()
                .iter()
                .find(|(id, flight)| id == task_id && flight.run_id == run_id)
                .map(|(_, flight)| flight.started_at);
            if flight.is_some() {
                scheduler.take_flight(task_id);
            }
            log::log(&format!(
                "task “{}” did not start: {}",
                str_field(&order, "name"),
                error.code
            ));
            scheduler.record(
                task_id,
                json!({"at": flight.unwrap_or_else(log::now_ms), "runId": run_id, "outcome": "failed",
                       "reason": format!("{}: {}", error.code, error.message)}),
            );
        });
    }

    fn record(&self, task_id: &str, run: Value) {
        update_schedules(|state| {
            if let Some(task) = state
                .tasks
                .iter_mut()
                .find(|task| str_field(task, "id") == task_id)
            {
                log_run(task, run);
            }
        });
        self.publish();
    }

    fn expire_silent(&self, now: i64) {
        let expired: Vec<(String, InFlight)> = {
            let mut flights = self.in_flight.borrow_mut();
            let (old, kept): (Vec<_>, Vec<_>) = flights.drain(..).partition(|(_, flight)| {
                now - flight.started_at >= TASK_RUN_MAX_MS + REPORT_GRACE_MS
            });
            *flights = kept;
            old
        };
        for (task_id, flight) in expired {
            self.record(
                &task_id,
                json!({"at": flight.started_at, "runId": flight.run_id, "outcome": "failed", "reason": "The run never reported back."}),
            );
        }
    }

    pub(crate) fn save(&self, input: &Value) -> ActionResult {
        let now = log::now_ms();
        let editing = input.get("id").and_then(Value::as_str).map(str::to_owned);
        let stored = editing.as_deref().and_then(|id| {
            read_schedules()
                .tasks
                .into_iter()
                .find(|task| str_field(task, "id") == id)
        });
        let mut draft = match validate_task(input, now, stored.as_ref()) {
            Ok(draft) => draft,
            Err((_, message)) => return failure("INVALID_INPUT", message),
        };
        let id = draft
            .shift_remove("id")
            .and_then(|id| id.as_str().map(str::to_owned));
        let saved = update_schedules(|state| -> ActionResult {
            if let Some(id) = &id {
                let Some(at) = state
                    .tasks
                    .iter()
                    .position(|task| str_field(task, "id") == id)
                else {
                    return not_found(id);
                };
                let old = &state.tasks[at];
                let mut task = Value::Object(draft.clone());
                task["id"] = json!(id);
                task["createdAt"] = old.get("createdAt").cloned().unwrap_or(json!(now));
                task["runCount"] = old.get("runCount").cloned().unwrap_or(json!(0));
                task["runs"] = old.get("runs").cloned().unwrap_or(json!([]));
                task["nextRunAt"] = Value::Null;
                with_next_run(&mut task, now);
                state.tasks[at] = task.clone();
                return Ok(task);
            }
            if state.tasks.len() >= MAX_TASKS {
                return failure(
                    "TASK_LIMIT",
                    format!(
                        "{MAX_TASKS} tasks are already scheduled — delete one before adding another."
                    ),
                );
            }
            let mut task = Value::Object(draft.clone());
            task["id"] = json!(uuid_v4());
            task["createdAt"] = json!(now);
            task["runCount"] = json!(0);
            task["runs"] = json!([]);
            task["nextRunAt"] = Value::Null;
            with_next_run(&mut task, now);
            state.tasks.push(task.clone());
            Ok(task)
        });
        let task = saved?;
        let next = next_run_at(&task)
            .and_then(|at| jiff::Timestamp::from_millisecond(at).ok())
            .map(|at| format!("; next run {}", log::iso(at)))
            .unwrap_or_default();
        log::log(&format!("task “{}” saved{next}", str_field(&task, "name")));
        self.arm(None);
        self.publish();
        Ok(self.list())
    }

    pub(crate) fn remove(&self, task_id: &str) -> ActionResult {
        let removed = update_schedules(|state| {
            let before = state.tasks.len();
            state.tasks.retain(|task| str_field(task, "id") != task_id);
            state.tasks.len() < before
        });
        if !removed {
            return not_found(task_id);
        }
        self.arm(None);
        self.publish();
        Ok(self.list())
    }

    pub(crate) fn pause(&self, paused: bool) -> ActionResult {
        update_schedules(|state| set_paused(state, paused, log::now_ms()));
        log::log(if paused {
            "scheduled tasks paused"
        } else {
            "scheduled tasks resumed"
        });
        self.arm(None);
        self.publish();
        Ok(self.list())
    }

    pub(crate) fn run_now(&self, task_id: &str, link: Rc<Link>) -> ActionResult {
        let Some(task) = read_schedules()
            .tasks
            .into_iter()
            .find(|task| str_field(task, "id") == task_id)
        else {
            return not_found(task_id);
        };
        if self.in_flight_has(task_id) {
            return failure(
                "RUN_IN_PROGRESS",
                format!(
                    "“{}” is already running — stop it, or wait for it to finish.",
                    str_field(&task, "name")
                ),
            );
        }
        let run_id = uuid_v4();
        self.in_flight.borrow_mut().push((
            task_id.to_owned(),
            InFlight {
                run_id: run_id.clone(),
                link_id: link.install_id.clone(),
                started_at: log::now_ms(),
            },
        ));
        self.publish();
        self.dispatch(order_for(&task, &run_id), link);
        Ok(self.list())
    }

    /// A report for a run the daemon already gave up on replaces that entry.
    pub(crate) fn finished(&self, task_id: &str, result: &Value) {
        let run_id = result
            .get("runId")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let flight = {
            let matches = self
                .in_flight
                .borrow()
                .iter()
                .any(|(id, flight)| id == task_id && Some(&flight.run_id) == run_id.as_ref());
            if matches {
                self.take_flight(task_id)
            } else {
                None
            }
        };
        log::log(&format!(
            "task {task_id} finished: {}",
            result.get("outcome").and_then(Value::as_str).unwrap_or("")
        ));
        update_schedules(|state| {
            let Some(task) = state
                .tasks
                .iter_mut()
                .find(|task| str_field(task, "id") == task_id)
            else {
                return;
            };
            let given = run_id.as_ref().and_then(|run_id| {
                task.get("runs").and_then(Value::as_array).and_then(|runs| {
                    runs.iter()
                        .position(|run| run.get("runId").and_then(Value::as_str) == Some(run_id))
                })
            });
            match given {
                Some(at) => {
                    let kept_at = task["runs"][at]["at"].clone();
                    let mut run = json!({"at": kept_at});
                    if let Some(object) = result.as_object() {
                        for (key, value) in object {
                            run[key] = value.clone();
                        }
                    }
                    run["at"] = kept_at;
                    task["runs"][at] = run;
                }
                None => {
                    let duration = result
                        .get("durationMs")
                        .and_then(Value::as_i64)
                        .unwrap_or(0);
                    let at = flight
                        .as_ref()
                        .map_or_else(|| log::now_ms() - duration, |flight| flight.started_at);
                    let mut run = json!({"at": at});
                    if let Some(object) = result.as_object() {
                        for (key, value) in object {
                            run[key] = value.clone();
                        }
                    }
                    log_run(task, run);
                }
            }
        });
        self.publish();
    }

    pub(crate) fn link_opened(&self) {
        self.arm(Some(LINK_SETTLE_MS));
    }

    pub(crate) fn link_closed(&self, link_id: &str) {
        let gone: Vec<(String, InFlight)> = {
            let mut flights = self.in_flight.borrow_mut();
            let (gone, kept): (Vec<_>, Vec<_>) = flights
                .drain(..)
                .partition(|(_, flight)| flight.link_id == link_id);
            *flights = kept;
            gone
        };
        for (task_id, flight) in gone {
            self.record(
                &task_id,
                json!({"at": flight.started_at, "runId": flight.run_id, "outcome": "failed",
                       "reason": "The browser disconnected before the run finished."}),
            );
        }
    }
}
