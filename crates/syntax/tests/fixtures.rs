//! Tree snapshots for every file under `test_data/parser`, one test per
//! directory, named after it. Run with `INSTA_UPDATE=always` after a
//! grammar change and review the diffs.

use std::fmt::Write;

use nomi_syntax::{Parse, Source, Tool};

fn render(text: &str, parse: impl Fn(&str) -> Parse) -> String {
    let parse = parse(text);
    assert_eq!(parse.syntax().to_string(), text, "the tree is lossless");
    let mut out = parse.debug_tree();
    for error in parse.errors() {
        writeln!(out, "error {:?}: {:?}", error.range, error.kind).unwrap();
    }
    out
}

#[test]
fn module() {
    insta::glob!("../test_data/parser", "module/*.elm", |path| {
        let text = std::fs::read_to_string(path).unwrap();
        insta::assert_snapshot!(render(&text, nomi_syntax::parse_module));
    });
}

#[test]
fn declarations() {
    insta::glob!("../test_data/parser", "declarations/*.elm", |path| {
        let text = std::fs::read_to_string(path).unwrap();
        insta::assert_snapshot!(render(&text, nomi_syntax::parse_module));
    });
}

#[test]
fn recovery() {
    insta::glob!("../test_data/parser", "recovery/*.elm", |path| {
        let text = std::fs::read_to_string(path).unwrap();
        insta::assert_snapshot!(render(&text, nomi_syntax::parse_module));
    });
}

#[test]
fn declarations_fragment() {
    insta::glob!(
        "../test_data/parser",
        "declarations_fragment/*.elm",
        |path| {
            let text = std::fs::read_to_string(path).unwrap();
            insta::assert_snapshot!(render(&text, nomi_syntax::parse_declarations));
        }
    );
}

#[test]
fn expressions_fragment() {
    insta::glob!(
        "../test_data/parser",
        "expressions_fragment/*.elm",
        |path| {
            let text = std::fs::read_to_string(path).unwrap();
            insta::assert_snapshot!(render(&text, nomi_syntax::parse_expressions));
        }
    );
}

#[test]
fn types() {
    insta::glob!("../test_data/parser", "types/*.elm", |path| {
        let text = std::fs::read_to_string(path).unwrap();
        insta::assert_snapshot!(render(&text, nomi_syntax::parse_type));
    });
}

#[test]
fn patterns() {
    insta::glob!("../test_data/parser", "patterns/*.elm", |path| {
        let text = std::fs::read_to_string(path).unwrap();
        insta::assert_snapshot!(render(&text, nomi_syntax::parse_pattern));
    });
}

#[test]
fn expressions() {
    insta::glob!("../test_data/parser", "expressions/*.elm", |path| {
        let text = std::fs::read_to_string(path).unwrap();
        insta::assert_snapshot!(render(&text, nomi_syntax::parse_expr));
    });
}

/// What each tool rejects in a module that parses cleanly.
#[test]
fn validation() {
    let tools = [
        ("elm, application", Tool::Elm(Source::Application)),
        ("elm, package", Tool::Elm(Source::Package)),
        ("elm, kernel package", Tool::Elm(Source::Kernel)),
        ("elm-format", Tool::ElmFormat),
    ];
    insta::glob!("../test_data/parser", "validation/*.elm", |path| {
        let text = std::fs::read_to_string(path).unwrap();
        let parse = nomi_syntax::parse_module(&text);
        assert_eq!(parse.errors(), [], "validation fixtures parse cleanly");
        let mut out = String::new();
        for (name, tool) in tools {
            writeln!(out, "{name}:").unwrap();
            for error in nomi_syntax::validate(&parse, tool) {
                let range = error.range;
                let found = &text[usize::from(range.start())..usize::from(range.end())];
                writeln!(out, "  {range:?} {:?} {found:?}", error.kind).unwrap();
            }
        }
        insta::assert_snapshot!(out);
    });
}

/// A fixture renamed or removed must not leave its snapshot behind.
#[test]
fn every_snapshot_has_a_fixture() {
    let root = concat!(env!("CARGO_MANIFEST_DIR"), "/tests");
    for entry in std::fs::read_dir(format!("{root}/snapshots")).unwrap() {
        let name = entry.unwrap().file_name().into_string().unwrap();
        let (test, file) = name
            .strip_prefix("fixtures__")
            .and_then(|rest| rest.strip_suffix(".snap"))
            .and_then(|rest| rest.split_once('@'))
            .unwrap_or_else(|| panic!("unexpected snapshot {name}"));
        let fixture = format!("{root}/../test_data/parser/{test}/{file}");
        assert!(
            std::path::Path::new(&fixture).exists(),
            "{name} has no fixture"
        );
    }
}
