//! React reports (agent-browser `native/react`): the extension walks the fibers and answers raw
//! JSON; these turn it into the text agents read.

pub mod renders;
// Vendored as agent-browser wrote it, for edition 2021 (no let chains).
#[allow(clippy::collapsible_if)]
pub mod suspense;
pub mod tree;

pub use renders::{RendersData, format_renders_report};
pub use suspense::{Boundary, format_suspense_report};
pub use tree::{TreeNode, format_tree};
