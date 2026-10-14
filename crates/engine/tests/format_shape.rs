//! Formatting leaves what elm reads unchanged: every fixture
//! module elm accepts formats to a module of the same HIR shape. The
//! formatter's own fixtures and the compiler's packages; `cargo xtask
//! corpus hir` checks the corpus alike.

use std::path::{Path, PathBuf};

use nomi_base::Compat;
use nomi_engine::format::shape_change;
use nomi_syntax::{Source, Tool, parse_module, validate};
use nomi_testkit::{files_below, packages};

/// The `.elm` files below `dir`.
fn modules(dir: PathBuf) -> impl Iterator<Item = PathBuf> {
    let files = files_below(&dir).unwrap();
    (files.into_iter())
        .filter(|file| file.ends_with(".elm"))
        .map(move |file| dir.join(file))
}

#[test]
fn formatting_keeps_the_hir_shape() {
    let format = Path::new(env!("CARGO_MANIFEST_DIR")).join("../format/tests/format_fixtures");
    let found: Vec<PathBuf> = modules(format.join("inputs"))
        .chain(modules(packages::dir()))
        .collect();
    assert!(found.len() > 1000, "{} modules", found.len());
    // An application's rules, or for infix declarations and effect
    // managers, a kernel package's.
    let accepts = |text: &str, tool| {
        let parse = parse_module(text);
        parse.errors().is_empty() && validate(&parse, tool).is_empty()
    };
    let changed: Vec<String> = (found.iter())
        .filter_map(|path| {
            let text = std::fs::read_to_string(path).ok()?;
            let app = Tool::Elm(Source::Application, Compat::LATEST);
            let tool = if accepts(&text, app) {
                app
            } else {
                Tool::Elm(Source::Kernel, Compat::LATEST)
            };
            let change = shape_change(&text, tool)?;
            Some(format!("{}: {change}", path.display()))
        })
        .collect();
    assert!(changed.is_empty(), "{}", changed.join("\n"));
}

#[test]
fn a_change_in_meaning_shows() {
    let tool = Tool::Elm(Source::Application, Compat::LATEST);
    assert_eq!(
        shape_change("module A exposing (a)\n\n\na =\n    (f x) y\n", tool),
        None
    );
    // elm-format reads a multiline string's CRLF as `\n`; elm doesn't.
    let crlf = "module A exposing (a)\r\n\r\n\r\na =\r\n    \"\"\"x\r\ny\"\"\"\r\n";
    assert_eq!(shape_change(crlf, tool), None);
}
