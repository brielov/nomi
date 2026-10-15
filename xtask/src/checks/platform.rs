//! The compiler never names `Platform` (rule R1): a module means the same
//! on every platform, so only what links and packages a program knows
//! where it runs.

use super::purity::{Comments, blanked, token_offsets};

/// The crate that may not name it.
pub(crate) const COMPILER: &str = "crates/compiler/";

pub(crate) fn check(label: &str, source: &str) -> Vec<String> {
    let code = blanked(source, Comments::Blank);
    token_offsets(&code, "Platform")
        .into_iter()
        .map(|offset| {
            let line = code[..offset].matches('\n').count() + 1;
            format!("{label}:{line}: the compiler may not name `Platform` (rule R1)")
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn naming_it_fails() {
        assert_eq!(
            check("c.rs", "fn f() {}\nuse nomi_base::Platform;\n"),
            ["c.rs:2: the compiler may not name `Platform` (rule R1)"]
        );
        assert_eq!(check("c.rs", "fn f(p: nomi_base::Platform) {}").len(), 1);
    }

    #[test]
    fn elm_names_comments_and_longer_identifiers_pass() {
        let source =
            "// Platform.worker\nlet m = \"Platform\";\nlet platform = 1;\nstruct PlatformCmd;\n";
        assert!(check("c.rs", source).is_empty());
    }
}
