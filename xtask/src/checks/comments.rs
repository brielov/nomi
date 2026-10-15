//! No comment names a plan step or milestone (architecture.md §13, N2):
//! what a piece of code does stays true, which step wrote it is git's to
//! say.

use super::purity::{Comments, blanked};

/// Checks one file's comments: `plan 9.4`, `steps 9.8`, `(M5)`, `CR.2`.
pub(crate) fn check(label: &str, source: &str) -> Vec<String> {
    let mut violations = Vec::new();
    // This file's own examples.
    if label.ends_with("checks/comments.rs") {
        return violations;
    }
    // With strings blank, a `//` in one isn't a comment.
    for (number, line) in blanked(source, Comments::Keep).lines().enumerate() {
        let Some(at) = line.find("//") else { continue };
        let words: Vec<&str> = line[at + 2..]
            .split_whitespace()
            .map(|w| w.trim_matches(|c: char| "()[],.;:'\"`!/".contains(c)))
            .collect();
        let named = words.iter().enumerate().find_map(|(i, word)| {
            let after_plan = i > 0 && matches!(words[i - 1], "plan" | "plans" | "step" | "steps");
            ((after_plan && is_step(word)) || is_milestone(word)).then_some(*word)
        });
        if let Some(step) = named {
            violations.push(format!(
                "{label}:{}: a comment names plan step `{step}`; say what the code does, git has the history",
                number + 1
            ));
        }
    }
    violations
}

/// `9`, `9.4`, `10.6c`, `B.3` or `CR.2`.
fn is_step(word: &str) -> bool {
    let letters = word
        .find(|c: char| !c.is_ascii_uppercase())
        .unwrap_or(word.len());
    let rest = &word[letters..];
    let rest = rest.strip_prefix('.').unwrap_or(rest);
    let rest = rest.trim_end_matches(|c: char| c.is_ascii_lowercase());
    letters <= 2
        && !rest.is_empty()
        && rest
            .split('.')
            .all(|part| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit()))
}

/// `M5`, `M8b`, `MQ` or a lettered step with its dot: `B.3`, `CR.2`.
fn is_milestone(word: &str) -> bool {
    if word == "MQ" {
        return true;
    }
    if let Some(number) = word.strip_prefix('M') {
        let number = number.strip_suffix('b').unwrap_or(number);
        return !number.is_empty() && number.bytes().all(|b| b.is_ascii_digit());
    }
    let Some((letters, number)) = word.split_once('.') else {
        return false;
    };
    (1..=2).contains(&letters.len())
        && letters.bytes().all(|b| b.is_ascii_uppercase())
        && !number.is_empty()
        && number.bytes().all(|b| b.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn named(source: &str) -> usize {
        check("a.rs", source).len()
    }

    #[test]
    fn plan_steps_and_milestones_are_named() {
        assert_eq!(named("//! The test runner (plan M10).\n"), 1);
        assert_eq!(named("/// Completion (plans 9.8 and 9.9).\n"), 1);
        assert_eq!(named("x(); // see step 10.6c\n"), 1);
        assert_eq!(named("// published while current (CR.2).\n"), 1);
        assert_eq!(named("// codegen (M8) will.\n"), 1);
        assert_eq!(named("// the scoreboard (plan B.1).\n"), 1);
    }

    #[test]
    fn other_numbers_are_not() {
        assert_eq!(named("// elm 0.19.1 and 0.19.3\n"), 0);
        assert_eq!(named("// a bare CR stays, and R7 holds\n"), 0);
        assert_eq!(named("// the plan is to retry\n"), 0);
        assert_eq!(named("let plan = 9.4; // no comment here names one\n"), 0);
        assert_eq!(named("// Mb and M are letters\n"), 0);
        assert_eq!(named("let url = \"https://M5\"; // a URL\n"), 0);
    }
}
