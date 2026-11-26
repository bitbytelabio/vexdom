//! Deciding which tier sends a request next, and the writes already sent.

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use serde_json::Value;
use vexdom_core::result::{ActionError, ActionResult};

use super::outcome::{Failure, Stop, TierResult};
use super::{Flow, HarEntry, PreparedRequest, Tier, Write};

pub(crate) fn is_accepted(status: Option<u16>) -> bool {
    matches!(status, Some(200..=399))
}

/// The writes a run has sent, by any tier. A write goes out once per run.
#[derive(Default)]
pub(crate) struct WriteLedger {
    sent: HashSet<String>,
    replayed: bool,
}

impl WriteLedger {
    /// Note a write's answer: an accepted answer or an unknown outcome (which
    /// includes a 5xx or 408) means it may have been applied; a 4xx rejection
    /// does not.
    pub(crate) fn record(&mut self, req: &PreparedRequest, result: &TierResult) {
        if req.write && (is_accepted(result.status) || result.stop == Some(Stop::OutcomeUnknown)) {
            self.sent.insert(req.id.clone());
        }
    }

    /// The replay repeated every recorded command, writes included.
    pub(crate) fn replay_ran(&mut self, flow: &Flow) {
        self.replayed = true;
        self.sent.extend(flow.writes.iter().map(|w| w.id.clone()));
    }

    pub(crate) fn already_sent(&self, id: &str) -> bool {
        self.sent.contains(id)
    }

    /// A write the replay would repeat that this run already sent.
    pub(crate) fn first_repeated_by_replay(&self, flow: &Flow) -> Option<String> {
        flow.writes
            .iter()
            .find(|w| self.sent.contains(&w.id))
            .map(|w| w.id.clone())
    }
}

/// The tier to try next for a request, given the attempts made so far, or
/// `None` when the request is settled. A write that was already sent is only
/// answered from the replay's capture, never sent again.
pub(crate) fn next_tier(
    flow: &Flow,
    req: &PreparedRequest,
    attempts: &[TierResult],
    ledger: &WriteLedger,
) -> Option<Tier> {
    if req.write && ledger.already_sent(&req.id) {
        return (attempts.is_empty() && ledger.replayed).then_some(Tier::Replay);
    }
    let order = tiers_for(flow, &req.id);
    let Some(last) = attempts.last() else {
        return order.first().copied();
    };
    // The site answered with something every tier would get again, or the
    // request worked, or a write was accepted.
    if last.ok || last.stop.is_some() || (req.write && is_accepted(last.status)) {
        return None;
    }
    // A rejected input fails the same way on every tier, except a write the
    // site did not accept: its tokens may be stale and the page can send it once.
    // A read also goes on after a transport error, and after a 5xx once an
    // earlier tier saw the site change.
    if !last.drift && !last.skipped && !req.write {
        let server_error = matches!(last.status, Some(500..=599));
        let saw_drift = attempts.iter().any(|a| a.drift);
        if !(last.transport || (server_error && saw_drift)) {
            return None;
        }
    }
    let at = order.iter().position(|t| *t == last.tier)?;
    order.get(at + 1).copied()
}

/// Whether a request starts at the replay tier.
pub(crate) fn starts_at_replay(flow: &Flow, id: &str) -> bool {
    tiers_for(flow, id).first() == Some(&Tier::Replay)
}

/// Tiers to try for a request, starting from the one that last worked.
pub(crate) fn tiers_for(flow: &Flow, id: &str) -> Vec<Tier> {
    let preferred = flow
        .request(id)
        .and_then(|r| r.tier)
        .and_then(|t| Tier::ALL.iter().position(|x| *x == t))
        .unwrap_or(0);
    Tier::ALL[preferred..].to_vec()
}

/// What a sender may look at besides the request.
pub(crate) struct SendCtx<'a> {
    pub ledger: &'a WriteLedger,
    /// Requests from this one on that may fall back to the replay.
    pub wanted: &'a [&'a PreparedRequest],
}

/// The senders of a run, one per tier, sharing whatever they borrow.
pub(crate) trait Senders {
    async fn send(&mut self, tier: Tier, req: &PreparedRequest, ctx: &SendCtx<'_>) -> TierResult;
    /// Whether the recorded commands have run in this run.
    fn replay_ran(&self) -> bool;
    /// Refuses a request, its references filled, before any tier sends it.
    async fn check(&mut self, req: &PreparedRequest) -> Result<(), ActionError>;
}

/// What the tiers need from the browser: the flow's tab, its page, and the
/// commands a replay runs there.
pub(crate) trait Browser {
    /// The tab's address, or `None` when it cannot be read.
    async fn tab_url(&mut self) -> Option<String>;
    /// `page.evaluate` in the tab: the script's value, or why it did not run.
    async fn eval(&mut self, script: &str, timeout: Duration) -> Result<Value, ActionError>;
    /// Start collecting the tab's requests for a replay.
    async fn begin_capture(&mut self) -> Result<(), String>;
    /// One recorded command, `{"do": "<action>", ...its input}`.
    async fn run(&mut self, cmd: &Value) -> ActionResult;
    /// Waits until the requests at `expected` (host, path) have answered after
    /// `since_ms` (or `appear` passes), then until the page goes quiet, never
    /// past `deadline`.
    async fn settle(
        &mut self,
        expected: &[(String, String)],
        since_ms: i64,
        appear: Duration,
        deadline: Instant,
    );
    /// Stop collecting and hand back what the replay sent.
    async fn end_capture(&mut self) -> Vec<HarEntry>;
    /// Whether a write only the replay sends may go out.
    async fn write_approved(&mut self, write: &Write) -> Result<(), String>;
}

/// Send each request through its tiers. `check` runs on every request after
/// its references are filled and may refuse the run.
pub(crate) async fn run_requests<S: Senders>(
    flow: &Flow,
    requests: &[PreparedRequest],
    senders: &mut S,
) -> Result<Vec<TierResult>, ActionError> {
    let mut results = Vec::new();
    let mut bodies: HashMap<String, Value> = HashMap::new();
    let mut ledger = WriteLedger::default();
    for (index, prepared) in requests.iter().enumerate() {
        let req = &match prepared.resolve(&bodies) {
            Ok(r) => r,
            Err(e) => {
                results.push(TierResult::fail(
                    prepared,
                    Tier::ALL[0],
                    Instant::now(),
                    Failure::drift(e),
                ));
                break;
            }
        };
        senders.check(req).await?;
        // Later requests are filled with what is known so far, so the replay
        // can find their captured answers.
        let filled: Vec<PreparedRequest> = requests[index..]
            .iter()
            .filter(|r| r.id == req.id || r.write || starts_at_replay(flow, &r.id))
            .map(|r| match r.id == req.id {
                true => req.clone(),
                false => r.resolve(&bodies).unwrap_or_else(|_| r.clone()),
            })
            .collect();
        let wanted: Vec<&PreparedRequest> = filled.iter().collect();
        let first_attempt = results.len();
        let mut done = false;
        while let Some(tier) = next_tier(flow, req, &results[first_attempt..], &ledger) {
            let result = {
                let ctx = SendCtx {
                    ledger: &ledger,
                    wanted: &wanted,
                };
                senders.send(tier, req, &ctx).await
            };
            if tier == Tier::Replay && senders.replay_ran() {
                ledger.replay_ran(flow);
            }
            if let Some(body) = result.body.as_ref().filter(|_| result.ok) {
                bodies.insert(req.id.clone(), body.clone());
            }
            ledger.record(req, &result);
            done = result.ok;
            results.push(result);
        }
        if !done {
            break;
        }
    }
    Ok(results)
}
