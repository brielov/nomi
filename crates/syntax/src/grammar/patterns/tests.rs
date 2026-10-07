//! Layout at the top level, where the fixtures' fragments can't reach:
//! a pattern ends at the first token that isn't indented.

use super::*;
use crate::parser::{DEPTH_LIMIT, Parse};

/// Parses a pattern as a top-level function's parameter would be, after
/// a name, then sweeps up the rest.
fn top_level(text: &str) -> Parse {
    let mut p = Parser::new(text);
    let m = p.start();
    p.bump();
    pattern(&mut p);
    if !p.at(K::Eof) {
        let rest = p.start();
        while !p.at(K::Eof) {
            p.bump();
        }
        rest.complete(&mut p, K::Error);
    }
    m.complete(&mut p, K::PatternFragment);
    p.finish()
}

fn render(text: &str) -> String {
    let parse = top_level(text);
    let mut out = parse.debug_tree();
    for error in parse.elm_errors() {
        out.push_str(&format!("error {:?}: {:?}\n", error.range, error.kind));
    }
    out
}

#[track_caller]
fn check(text: &str, expected: &str) {
    assert_eq!(render(text).trim(), expected.trim());
}

fn errors(text: &str) -> Vec<SyntaxErrorKind> {
    top_level(text)
        .elm_errors()
        .iter()
        .map(|e| e.kind)
        .collect()
}

#[test]
fn arguments_stop_at_column_zero() {
    check(
        "f Just\nx",
        r#"
PatternFragment@0..8
  LowerIdent@0..1 "f"
  Whitespace@1..2 " "
  ConstructorPattern@2..6
    UpperIdent@2..6 "Just"
  Newline@6..7 "\n"
  Error@7..8
    LowerIdent@7..8 "x"
"#,
    );
}

#[test]
fn cons_and_as_at_column_zero_end_the_pattern() {
    for text in ["f x\n:: y", "f x\nas y"] {
        let tree = render(text);
        assert!(tree.contains("\n  VarPattern@2..3\n"), "{tree}");
        assert!(tree.contains("\n  Error@4..8\n"), "{tree}");
        assert_eq!(errors(text), [], "{text}");
    }
}

#[test]
fn indented_cons_and_as_continue() {
    let text = "f x\n  :: y\n  as z";
    let tree = render(text);
    assert!(
        tree.contains("\n  AliasPattern@2..17\n    ConsPattern@2..10\n"),
        "{tree}"
    );
    assert_eq!(errors(text), []);
}

#[test]
fn a_pattern_at_column_zero_is_missing() {
    check(
        "f\nx",
        r#"
PatternFragment@0..3
  LowerIdent@0..1 "f"
  Missing@1..1
  Newline@1..2 "\n"
  Error@2..3
    LowerIdent@2..3 "x"
error 2..3: ExpectedPattern
"#,
    );
}

#[test]
fn an_alias_name_at_column_zero_is_reported() {
    assert_eq!(errors("f x as\ny"), [SyntaxErrorKind::ExpectedIndent]);
}

#[test]
fn list_and_record_punctuation_must_be_indented() {
    let indent = SyntaxErrorKind::ExpectedIndent;
    assert_eq!(errors("f [ a\n, b\n]"), [indent, indent]);
    assert_eq!(errors("f { a\n, b\n}"), [indent, indent]);
    assert_eq!(errors("f { a,\nb }"), [indent]);
    assert_eq!(errors("f [\n]"), [indent]);
    assert_eq!(errors("f {\n}"), [indent]);
}

#[test]
fn tuple_punctuation_must_be_indented() {
    let indent = SyntaxErrorKind::ExpectedIndent;
    assert_eq!(errors("f ( a\n, b\n)"), [indent, indent]);
    assert_eq!(errors("f ( a\n)"), [indent]);
    assert_eq!(errors("f (\n)"), [indent]);
}

#[test]
fn an_unclosed_bracket_stops_at_column_zero() {
    for (text, closer) in [
        ("f (a\nb", K::RParen),
        ("f [a\nb", K::RBracket),
        ("f {a\nb", K::RBrace),
    ] {
        assert_eq!(errors(text), [SyntaxErrorKind::Expected(closer)], "{text}");
        assert!(render(text).contains("\n  Error@5..6\n"), "{text}");
    }
}

fn too_deep(text: &str) -> usize {
    errors(text)
        .iter()
        .filter(|&&e| e == SyntaxErrorKind::TooDeep)
        .count()
}

#[test]
fn deep_nesting_is_one_error() {
    let n = DEPTH_LIMIT as usize + 5_000;
    stacker::grow(nomi_base::STACK_SIZE, || {
        let parens = format!("f {}x{}", "(".repeat(n), ")".repeat(n));
        assert_eq!(too_deep(&parens), 1);
        let lists = format!("f {}x{}", "[".repeat(n), "]".repeat(n));
        assert_eq!(too_deep(&lists), 1);
        let cons = format!("f {}x", "x :: ".repeat(n));
        assert_eq!(too_deep(&cons), 1);
    });
}
