//! `snapshot` with `find`: the lines of a rendered tree that mention a text, so an agent can look
//! something up on a big page without reading the whole tree.

use crate::snapshot::keep_with_ancestors;

/// The lines of `tree` whose text contains `needle` (case-insensitively), each with its ancestors,
/// under a header that counts them: `find "{needle}": {n} of {total} lines match`. With no match,
/// one line that says so and how to widen the search.
#[must_use]
pub fn find_lines(tree: &str, needle: &str) -> String {
    let lines: Vec<&str> = tree.lines().collect();
    let total = lines.len();
    let needle_lower = needle.to_lowercase();
    let matches = |line: &str| line.to_lowercase().contains(&needle_lower);
    let found = lines.iter().filter(|line| matches(line)).count();
    if found == 0 {
        return format!(
            "find {needle:?}: no line of {total} matches; try another word, or snapshot without find"
        );
    }
    let kept = keep_with_ancestors(&lines, matches);
    format!(
        "find {needle:?}: {found} of {total} lines match\n{}",
        kept.join("\n")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const TREE: &str = "- navigation\n  - link \"Home\" [ref=e1]\n- main\n  - list\n    - listitem\n      - link \"Checkout\" [ref=e2]\n  - button \"Pay\" [ref=e3]";

    #[test]
    fn a_match_keeps_its_ancestors() {
        assert_eq!(
            find_lines(TREE, "checkout"),
            "find \"checkout\": 1 of 7 lines match\n- main\n  - list\n    - listitem\n      - link \"Checkout\" [ref=e2]"
        );
    }

    #[test]
    fn matching_ignores_case() {
        assert_eq!(
            find_lines(TREE, "PAY"),
            "find \"PAY\": 1 of 7 lines match\n- main\n  - button \"Pay\" [ref=e3]"
        );
    }

    #[test]
    fn no_match_says_how_to_widen() {
        assert_eq!(
            find_lines(TREE, "refund"),
            "find \"refund\": no line of 7 matches; try another word, or snapshot without find"
        );
    }
}
