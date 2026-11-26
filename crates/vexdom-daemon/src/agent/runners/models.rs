//! The models the picker offers for an agent (`runners/models.ts`).

use serde_json::{Value, json};
use vexdom_core::agents;

/// The catalog's curated models: `ModelList` (`src/lib/agents/catalog.ts`).
pub fn models_for(kind: &str) -> Value {
    let ids = agents::agent(kind).map_or(&[][..], |agent| agent.models);
    json!({"ids": ids, "from": "catalog"})
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_picker_offers_the_curated_list() {
        assert_eq!(
            models_for("claude"),
            json!({"ids": agents::CLAUDE.models, "from": "catalog"})
        );
    }
}
