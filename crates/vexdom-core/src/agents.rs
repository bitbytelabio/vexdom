//! The agent CLIs the side panel can run (`src/lib/agents/catalog.ts`). Only Claude Code.

use std::sync::LazyLock;

use regex::Regex;
use serde::Serialize;

pub const AGENT_KINDS: [&str; 1] = ["claude"];
pub const DEFAULT_AGENT: &str = "claude";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AgentDescriptor {
    pub kind: &'static str,
    pub label: &'static str,
    pub vendor: &'static str,
    pub bin: &'static str,
    pub install: &'static str,
    pub docs: &'static str,
    /// Offered, strongest first, whenever the CLI cannot list its own.
    pub models: &'static [&'static str],
}

pub const CLAUDE: AgentDescriptor = AgentDescriptor {
    kind: "claude",
    label: "Claude Code",
    vendor: "Anthropic",
    bin: "claude",
    install: "npm i -g @anthropic-ai/claude-code",
    docs: "https://claude.com/claude-code",
    models: &["fable", "opus", "sonnet", "haiku"],
};

pub const AGENT_LIST: [AgentDescriptor; 1] = [CLAUDE];

pub fn is_agent_kind(value: &str) -> bool {
    AGENT_KINDS.contains(&value)
}

pub fn agent(kind: &str) -> Option<&'static AgentDescriptor> {
    AGENT_LIST.iter().find(|agent| agent.kind == kind)
}

/// An agent's name, or its kind as sent when this build does not know it.
pub fn agent_label(kind: &str) -> String {
    agent(kind).map_or_else(|| kind.to_owned(), |agent| agent.label.to_owned())
}

static MODEL_ID: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[A-Za-z0-9][^\s\p{C}]{0,199}$").expect("model id compiles"));

/// Anything a CLI takes as a model and could not read as a flag.
pub fn is_model_id(value: &str) -> bool {
    MODEL_ID.is_match(value)
}

#[cfg(test)]
mod tests {
    #[test]
    fn model_ids() {
        assert!(super::is_model_id("claude-opus-4-8[effort=high]"));
        assert!(!super::is_model_id("--dangerously"));
        assert!(!super::is_model_id("a b"));
        assert!(!super::is_model_id(""));
    }
}
