//! Standing approvals, "Allow for this site" remembered (`src/daemon/agent/approvals.ts`):
//! `approvals.json`. A grant only ever short-circuits a `confirm`.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{log, paths};

const MAX_GRANTS: usize = 200;

/// A grant for everything on a site: "always on this site" in Ask and Follow-a-plan modes.
pub const ANY_ACTION: &str = "*";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Grant {
    pub action: String,
    pub host: String,
    pub at: String,
}

fn approvals_path() -> PathBuf {
    paths::state_dir().join("approvals.json")
}

fn read() -> Vec<Grant> {
    std::fs::read(approvals_path())
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        .and_then(|value| value.get("grants").and_then(Value::as_array).cloned())
        .map(|grants| {
            grants
                .into_iter()
                .filter_map(|grant| serde_json::from_value(grant).ok())
                .collect()
        })
        .unwrap_or_default()
}

fn write(grants: &[Grant]) {
    if let Err(error) = paths::write_private_json(&approvals_path(), &json!({ "grants": grants })) {
        log::log_detail("could not write approvals.json", &error.to_string());
    }
}

pub fn list_grants() -> Vec<Grant> {
    read()
}

pub fn is_granted(action: &str, host: &str) -> bool {
    read()
        .iter()
        .any(|grant| grant.action == action && grant.host == host)
}

/// Whether the user allowed a side-panel run to act on `host` for good, in Ask and Follow-a-plan
/// modes. Only that site check reads it: a site grant never stands in for a rule's confirm.
pub fn site_granted(host: &str) -> bool {
    is_granted(ANY_ACTION, host)
}

pub fn remember_grant(action: &str, host: &str, at: &str) {
    let mut grants = vec![Grant {
        action: action.to_owned(),
        host: host.to_owned(),
        at: at.to_owned(),
    }];
    grants.extend(
        read()
            .into_iter()
            .filter(|grant| !(grant.action == action && grant.host == host)),
    );
    grants.truncate(MAX_GRANTS);
    write(&grants);
    log::log(&format!("remembered approval: {action} on {host}"));
}

/// Forgets every grant, or one host's; returns how many went.
pub fn forget_grants(host: Option<&str>) -> usize {
    let grants = read();
    let kept: Vec<Grant> = match host {
        Some(host) => grants
            .iter()
            .filter(|grant| grant.host != host)
            .cloned()
            .collect(),
        None => Vec::new(),
    };
    write(&kept);
    grants.len() - kept.len()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth_store::tests::scratch_home;

    #[test]
    fn grants() {
        let _home = scratch_home("approvals");
        remember_grant("page.submitForm", "a.com", "t1");
        remember_grant("page.submitForm", "b.com", "t2");
        remember_grant("page.submitForm", "a.com", "t3");
        assert_eq!(list_grants()[0].at, "t3");
        assert!(is_granted("page.submitForm", "b.com"));
        assert_eq!(forget_grants(Some("a.com")), 1);
        assert_eq!(forget_grants(None), 1);
    }

    #[test]
    fn a_site_grant_allows_the_site_and_confirms_nothing() {
        let _home = scratch_home("approvals-any");
        remember_grant(ANY_ACTION, "a.com", "t1");
        assert!(site_granted("a.com"));
        assert!(!site_granted("b.com"));
        assert!(
            !is_granted("page.submitForm", "a.com"),
            "a site grant must not skip a rule's confirm"
        );
    }
}
