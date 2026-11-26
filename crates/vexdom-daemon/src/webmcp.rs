//! The WebMCP announcement (ported from agent-browser's `RuntimeState::context_update`, f0a9835,
//! Apache-2.0, see NOTICE): after a navigation or a snapshot, the tab's WebMCP tools are listed,
//! and a bounded summary rides along when they changed since the last announcement. Unchanged
//! tools, and ordinary pages after ordinary pages, add nothing.

use std::rc::Rc;

use serde_json::{Value, json};
use vexdom_snapshot::webmcp_text::{ToolRecord, context_from_tools};

use crate::Daemon;
use crate::link::Link;

/// Actions after which the tab's tools are announced.
pub(crate) const ANNOUNCE_ACTIONS: &[&str] = &[
    "page.navigate",
    "page.openTab",
    "page.switchTab",
    "page.pushState",
];

/// What the agent was last told about: the tab, its origin, and its tools.
#[derive(Default)]
pub(crate) struct Advertised(Option<(i64, Option<String>, Vec<ToolRecord>)>);

impl Advertised {
    /// `None` means no update. Only a change from advertised tools to none speaks up about an
    /// empty catalog; ordinary pages stay quiet.
    fn update(
        &mut self,
        tab: i64,
        origin: Option<String>,
        tools: Vec<ToolRecord>,
    ) -> Option<Value> {
        if tools.is_empty() {
            return self.0.take().map(|_| context_from_tools(&[]));
        }
        if self
            .0
            .as_ref()
            .is_some_and(|(previous, page_origin, records)| {
                *previous == tab && *page_origin == origin && *records == tools
            })
        {
            return None;
        }
        let mut context = context_from_tools(&tools);
        if let Some(origin) = &origin {
            context["pageOrigin"] = json!(origin);
        }
        self.0 = Some((tab, origin, tools));
        Some(context)
    }
}

impl Daemon {
    /// The context to announce for `tab` (the active one when `None`), or `None`.
    pub(crate) async fn webmcp_context(&self, link: &Rc<Link>, tab: Option<i64>) -> Option<Value> {
        let reply = link
            .invoke("page.listSiteTools", &json!({}), tab, None)
            .await;
        let (tools, origin) = match reply.result {
            Ok(data) => (
                ToolRecord::list_of(&data),
                data["pageOrigin"].as_str().map(str::to_owned),
            ),
            // A page without tools says so; any other failure says nothing about them.
            Err(error) if matches!(error.code.as_str(), "NO_SITE_TOOLS" | "UNSUPPORTED") => {
                (Vec::new(), None)
            }
            Err(_) => return None,
        };
        self.with_page(link, |state| {
            state.advertised.update(tab.unwrap_or(-1), origin, tools)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tool(name: &str) -> ToolRecord {
        ToolRecord {
            name: name.into(),
            description: "d".into(),
            origin: "https://a.test".into(),
            frame_id: "0".into(),
        }
    }

    #[test]
    fn only_changes_are_announced() {
        let mut advertised = Advertised::default();
        let origin = Some("https://a.test".to_owned());
        assert_eq!(advertised.update(1, origin.clone(), Vec::new()), None);
        let first = advertised
            .update(1, origin.clone(), vec![tool("a")])
            .unwrap();
        assert_eq!(first["toolCount"], 1);
        assert_eq!(first["pageOrigin"], "https://a.test");
        assert_eq!(advertised.update(1, origin.clone(), vec![tool("a")]), None);
        assert!(
            advertised
                .update(1, origin.clone(), vec![tool("b")])
                .is_some()
        );
        let cleared = advertised.update(1, origin, Vec::new()).unwrap();
        assert_eq!(cleared["toolCount"], 0);
        assert_eq!(advertised.update(1, None, Vec::new()), None);
    }
}
