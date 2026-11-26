//! External confirmations (ADR-032): an MCP client or the CLI asks for an action a guardrail
//! wants the user to approve. The call waits here until the user answers from the desktop app,
//! `vexdom approve`, or an MCP client's own prompt, and every watcher of
//! `Topic::ConfirmationsChanged` hears about the list as it changes.

use std::time::Duration;

use futures::channel::oneshot;
use serde_json::{Value, json};
use vexdom_core::guardrails::secrets::seal_secrets;
use vexdom_core::guardrails::{DECLINED_MESSAGE, UNANSWERED_MESSAGE};
use vexdom_core::proto::CONFIRM_WAIT_MS;
use vexdom_core::result::ActionError;
use vexdom_core::{approvals, log, rand};
use vexdom_schema::Topic;

use crate::Daemon;

/// The input shown with a request, cut to this many bytes.
const PREVIEW_BYTES: usize = 2048;
/// The key a caller tags its input with, so it can find its own request in the list.
pub(crate) const APPROVER_KEY: &str = "_approver";

/// The user's answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Answer {
    Allow { remember: bool },
    Deny,
}

struct Pending {
    id: String,
    action: String,
    summary: String,
    site: Option<String>,
    input_preview: Value,
    approver: Option<String>,
    created_at: i64,
    tx: Option<oneshot::Sender<Answer>>,
}

impl Pending {
    fn json(&self) -> Value {
        json!({
            "id": self.id,
            "action": self.action,
            "summary": self.summary,
            "site": self.site,
            "input": self.input_preview,
            "approver": self.approver,
            "createdAt": self.created_at,
        })
    }
}

/// The requests waiting for an answer.
#[derive(Default)]
pub(crate) struct Confirmations(Vec<Pending>);

impl Confirmations {
    pub(crate) fn list(&self) -> Value {
        Value::Array(self.0.iter().map(Pending::json).collect())
    }

    /// Answers request `id`; `false` when there is none.
    pub(crate) fn answer(&mut self, id: &str, answer: Answer) -> bool {
        let Some(pending) = self.0.iter_mut().find(|pending| pending.id == id) else {
            return false;
        };
        pending.tx.take().is_some_and(|tx| tx.send(answer).is_ok())
    }

    fn remove(&mut self, id: &str) {
        self.0.retain(|pending| pending.id != id);
    }
}

/// The input without its approver tag, and the tag.
pub(crate) fn take_approver(input: &mut Value) -> Option<String> {
    input
        .as_object_mut()
        .and_then(|object| object.shift_remove(APPROVER_KEY))
        .and_then(|tag| tag.as_str().map(str::to_owned))
}

/// What the user sees of the input: sealed, and short.
fn preview(input: &Value) -> Value {
    let text = seal_secrets(&input.to_string());
    if text.len() <= PREVIEW_BYTES {
        return serde_json::from_str(&text).unwrap_or(Value::String(text));
    }
    let mut cut = PREVIEW_BYTES;
    while !text.is_char_boundary(cut) {
        cut -= 1;
    }
    Value::String(format!("{}…", &text[..cut]))
}

/// A request for approval.
pub(crate) struct Request<'call> {
    pub action: &'call str,
    pub summary: String,
    pub site: Option<String>,
    pub input: &'call Value,
    pub approver: Option<String>,
}

impl Daemon {
    pub(crate) fn confirmations_list(&self) -> Value {
        self.0.confirmations.borrow().list()
    }

    /// Answers request `id` (from `Control.confirm`); `false` when there is none.
    pub(crate) fn confirm(&self, id: &str, answer: Answer) -> bool {
        let found = self.0.confirmations.borrow_mut().answer(id, answer);
        if found {
            log::log(&format!("confirmation {id}: {answer:?}"));
        }
        found
    }

    /// Waits for the user to allow `request`: `Ok` to go ahead, else `DECLINED` or
    /// `APPROVAL_TIMEOUT`. A standing "always on this site" approval answers at once.
    pub(crate) async fn await_confirmation(&self, request: Request<'_>) -> Result<(), ActionError> {
        if let Some(site) = &request.site
            && approvals::is_granted(request.action, site)
        {
            return Ok(());
        }
        let id = format!("c-{}", rand::hex(4));
        let (tx, rx) = oneshot::channel();
        self.0.confirmations.borrow_mut().0.push(Pending {
            id: id.clone(),
            action: request.action.to_owned(),
            summary: request.summary,
            site: request.site.clone(),
            input_preview: preview(request.input),
            approver: request.approver,
            created_at: log::now_ms(),
            tx: Some(tx),
        });
        log::log(&format!("confirmation {id} waits: {}", request.action));
        self.notify(Topic::ConfirmationsChanged, &self.confirmations_list());
        // Gone from the list however the wait ends, the caller hanging up included.
        let _listed = Listed {
            daemon: self.clone(),
            id,
        };
        let answer = tokio::time::timeout(Duration::from_millis(CONFIRM_WAIT_MS), rx).await;
        match answer {
            Ok(Ok(Answer::Allow { remember })) => {
                if remember && let Some(site) = &request.site {
                    approvals::remember_grant(request.action, site, &log::iso_now());
                }
                Ok(())
            }
            Ok(Ok(Answer::Deny)) => Err(error("DECLINED", DECLINED_MESSAGE)),
            Ok(Err(_)) | Err(_) => Err(error("APPROVAL_TIMEOUT", UNANSWERED_MESSAGE)),
        }
    }
}

/// Takes a request off the list when its wait ends.
struct Listed {
    daemon: Daemon,
    id: String,
}

impl Drop for Listed {
    fn drop(&mut self) {
        self.daemon.0.confirmations.borrow_mut().remove(&self.id);
        self.daemon.notify(
            Topic::ConfirmationsChanged,
            &self.daemon.confirmations_list(),
        );
    }
}

fn error(code: &str, message: &str) -> ActionError {
    ActionError {
        code: code.into(),
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_approver_tag_is_taken_off_the_input() {
        let mut input = json!({"selector": "form", "_approver": "cli:12"});
        assert_eq!(take_approver(&mut input).as_deref(), Some("cli:12"));
        assert_eq!(input, json!({"selector": "form"}));
    }

    #[test]
    fn previews_are_sealed_and_short() {
        let long = json!({"text": "x".repeat(5000)});
        let shown = preview(&long);
        assert!(shown.as_str().unwrap().len() <= PREVIEW_BYTES + 3);
        let secret = json!({"text": "sk-ant-api03-abcdefghijklmnopqrstuvwxyz0123456789"});
        assert!(!preview(&secret).to_string().contains("abcdefghijklmnop"));
    }
}
