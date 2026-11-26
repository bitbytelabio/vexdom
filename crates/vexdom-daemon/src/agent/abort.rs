//! `AbortController`/`AbortSignal` for the agent code: one flag, an optional reason, and a wait.

use std::cell::{Cell, RefCell};
use std::future::Future;
use std::rc::Rc;
use std::time::Duration;

use tokio::sync::Notify;
use vexdom_core::result::ActionError;

/// A run's error (`RunError`): a code and a message, as an `ActionResult` failure carries them.
pub type RunError = ActionError;

pub fn run_error(code: &str, message: impl Into<String>) -> RunError {
    ActionError {
        code: code.to_owned(),
        message: message.into(),
    }
}

#[derive(Default)]
struct Inner {
    aborted: Cell<bool>,
    reason: RefCell<Option<RunError>>,
    notify: Notify,
}

/// Cheap to clone; every clone sees the same abort.
#[derive(Clone, Default)]
pub struct Abort(Rc<Inner>);

impl Abort {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn abort(&self) {
        self.abort_with(None);
    }

    /// The first abort wins; later ones change nothing.
    pub fn abort_with(&self, reason: Option<RunError>) {
        if self.0.aborted.replace(true) {
            return;
        }
        *self.0.reason.borrow_mut() = reason;
        self.0.notify.notify_waiters();
    }

    pub fn aborted(&self) -> bool {
        self.0.aborted.get()
    }

    /// Why it was aborted, when whoever aborted it said.
    pub fn reason(&self) -> Option<RunError> {
        self.0.reason.borrow().clone()
    }

    /// Resolves once aborted.
    pub async fn wait(&self) {
        loop {
            // Registered before the flag is read, so an abort in between still wakes it.
            let notified = self.0.notify.notified();
            if self.aborted() {
                return;
            }
            notified.await;
        }
    }
}

/// Resolves when `signal` aborts, or never when there is none.
pub async fn aborted(signal: Option<&Abort>) {
    match signal {
        Some(signal) => signal.wait().await,
        None => std::future::pending().await,
    }
}

/// Runs `work` while `stop` is aborted with `timeout` once `ms` have passed, or with `parent`'s
/// reason when `parent` aborts first: a deadline and a caller's cancel feeding one signal.
pub async fn bounded<T>(
    work: impl Future<Output = T>,
    stop: &Abort,
    parent: Option<&Abort>,
    ms: u64,
    timeout: RunError,
) -> T {
    let watch = async {
        tokio::select! {
            () = tokio::time::sleep(Duration::from_millis(ms)) => stop.abort_with(Some(timeout)),
            () = aborted(parent) => stop.abort_with(parent.and_then(Abort::reason)),
        }
        std::future::pending::<T>().await
    };
    tokio::select! {
        out = work => out,
        out = watch => out,
    }
}

/// Runs `work` while `signal` is aborted once `ms` have passed (`setTimeout(() => abort(), ms)`).
pub async fn abort_after<T>(work: impl Future<Output = T>, signal: &Abort, ms: u64) -> T {
    let watch = async {
        tokio::time::sleep(Duration::from_millis(ms)).await;
        signal.abort();
        std::future::pending::<T>().await
    };
    tokio::select! {
        out = work => out,
        out = watch => out,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_reason_wins_and_waiters_wake() {
        crate::agent::test_support::local(async {
            let signal = Abort::new();
            let waiter = signal.clone();
            let woke = tokio::task::spawn_local(async move { waiter.wait().await });
            tokio::task::yield_now().await;
            signal.abort_with(Some(run_error("CANCELLED", "first")));
            signal.abort_with(Some(run_error("TIMEOUT", "second")));
            woke.await.unwrap();
            assert!(signal.aborted());
            assert_eq!(signal.reason().unwrap().code, "CANCELLED");
            signal.wait().await;
        });
    }

    #[test]
    fn a_deadline_aborts_with_its_reason_and_a_parent_with_its_own() {
        crate::agent::test_support::local(async {
            let stop = Abort::new();
            let waited = bounded(
                {
                    let stop = stop.clone();
                    async move {
                        stop.wait().await;
                        stop.reason()
                    }
                },
                &stop,
                None,
                10,
                run_error("TIMEOUT", "slow"),
            )
            .await;
            assert_eq!(waited.unwrap().code, "TIMEOUT");

            let parent = Abort::new();
            parent.abort_with(Some(run_error("CANCELLED", "gone")));
            let stop = Abort::new();
            let waited = bounded(
                {
                    let stop = stop.clone();
                    async move {
                        stop.wait().await;
                        stop.reason()
                    }
                },
                &stop,
                Some(&parent),
                60_000,
                run_error("TIMEOUT", "slow"),
            )
            .await;
            assert_eq!(waited.unwrap().code, "CANCELLED");
        });
    }
}
