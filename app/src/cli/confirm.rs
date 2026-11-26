//! External confirmations from the terminal (ADR-032): `vexdom requests` lists them,
//! `vexdom approve c-… [--always]` and `vexdom deny c-…` answer them, and a page verb that waits on
//! one says so once on stderr.

use std::cell::RefCell;
use std::io::IsTerminal as _;
use std::rc::Rc;

use serde_json::Value;
use vexdom_client::Bridge;
use vexdom_schema::Topic;

/// The approver tag this process puts on its page actions.
pub(super) fn approver_tag() -> String {
    format!("cli:{}", std::process::id())
}

/// Whether an id names a confirmation rather than a pairing request.
pub(super) fn is_confirmation(id: &str) -> bool {
    id.starts_with("c-")
}

/// `c-1a2b3c4d  page.submitForm on example.com — Submits a form`.
pub(super) fn describe(confirmation: &Value) -> String {
    let field = |key: &str| confirmation.get(key).and_then(Value::as_str).unwrap_or("");
    let site = match field("site") {
        "" => String::new(),
        site => format!(" on {site}"),
    };
    format!(
        "{}  {}{site} — {}",
        field("id"),
        field("action"),
        field("summary")
    )
}

/// Approving takes a person at a terminal: an agent driving the CLI has no terminal, so it
/// cannot approve its own actions this way.
pub(super) fn needs_terminal() -> Option<&'static str> {
    (!std::io::stdin().is_terminal()).then_some(
        "Approving an action needs you at a terminal (or the Vexdom app); an agent cannot approve its own actions.",
    )
}

/// While this process waits on a page action, tells stderr once per confirmation it is waiting
/// on. Keep the handle for as long as the action runs.
pub(super) async fn announce_waiting(bridge: &Bridge) -> Option<vexdom_schema::handle::Client> {
    let tag = approver_tag();
    let told: Rc<RefCell<Vec<String>>> = Rc::default();
    bridge
        .watch(Topic::ConfirmationsChanged, None, false, move |_, list| {
            for confirmation in list.as_array().into_iter().flatten() {
                let mine =
                    confirmation.get("approver").and_then(Value::as_str) == Some(tag.as_str());
                let id = confirmation
                    .get("id")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_owned();
                if !mine || told.borrow().contains(&id) {
                    continue;
                }
                eprintln!(
                    "Waiting for approval {id}: approve it in Vexdom, or run vexdom approve {id} (vexdom deny {id} to refuse)."
                );
                told.borrow_mut().push(id);
            }
        })
        .await
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_confirmation_reads_as_one_line() {
        let row = json!({"id": "c-1a2b3c4d", "action": "page.submitForm", "site": "example.com", "summary": "Submits a form"});
        assert_eq!(
            describe(&row),
            "c-1a2b3c4d  page.submitForm on example.com — Submits a form"
        );
        assert!(is_confirmation("c-1a2b3c4d") && !is_confirmation("r-1"));
    }
}
