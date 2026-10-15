//! No test is skipped with `#[ignore]`: a test that cannot pass yet belongs
//! in a ratchet, not in the suite.

use super::purity::{Comments, blanked};

pub(crate) fn check(label: &str, source: &str) -> Vec<String> {
    blanked(source, Comments::Blank)
        .lines()
        .enumerate()
        .filter(|(_, line)| line.replace(' ', "").contains("#[ignore"))
        .map(|(i, _)| format!("{label}:{}: #[ignore] is not allowed; use a ratchet", i + 1))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_test_passes() {
        assert!(check("t.rs", "#[test]\nfn t() {}\n").is_empty());
    }

    #[test]
    fn ignored_test_fails() {
        let source = "#[test]\n#[ignore]\nfn t() {}\n";
        assert_eq!(
            check("t.rs", source),
            ["t.rs:2: #[ignore] is not allowed; use a ratchet"]
        );
    }

    #[test]
    fn ignore_with_reason_and_spaces_fails() {
        assert_eq!(check("t.rs", "#[ ignore = \"slow\"]\n").len(), 1);
    }

    #[test]
    fn comments_and_strings_pass() {
        assert!(check("t.rs", "// #[ignore]\nlet s = \"#[ignore]\";\n").is_empty());
    }
}
