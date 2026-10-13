//! Layout at the top level, where the fixtures' fragments can't reach:
//! a pattern ends at the first token that isn't indented.

use super::*;
use crate::grammar::testing::TopLevel;
use crate::parser::DEPTH_LIMIT;

/// A pattern as a top-level function's parameter, after a name.
const PATTERN: TopLevel = TopLevel {
    prefix: 1,
    rule: pattern,
    root: K::PatternFragment,
};

#[test]
fn arguments_stop_at_column_zero() {
    PATTERN.check(
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
        let tree = PATTERN.render(text);
        assert!(tree.contains("\n  VarPattern@2..3\n"), "{tree}");
        assert!(tree.contains("\n  Error@4..8\n"), "{tree}");
        assert_eq!(PATTERN.errors(text), [], "{text}");
    }
}

#[test]
fn indented_cons_and_as_continue() {
    let text = "f x\n  :: y\n  as z";
    let tree = PATTERN.render(text);
    assert!(
        tree.contains("\n  AliasPattern@2..17\n    ConsPattern@2..10\n"),
        "{tree}"
    );
    assert_eq!(PATTERN.errors(text), []);
}

#[test]
fn a_pattern_at_column_zero_is_missing() {
    PATTERN.check(
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
    assert_eq!(
        PATTERN.errors("f x as\ny"),
        [SyntaxErrorKind::ExpectedIndent]
    );
}

#[test]
fn list_and_record_punctuation_must_be_indented() {
    let indent = SyntaxErrorKind::ExpectedIndent;
    assert_eq!(PATTERN.errors("f [ a\n, b\n]"), [indent, indent]);
    assert_eq!(PATTERN.errors("f { a\n, b\n}"), [indent, indent]);
    assert_eq!(PATTERN.errors("f { a,\nb }"), [indent]);
    assert_eq!(PATTERN.errors("f [\n]"), [indent]);
    assert_eq!(PATTERN.errors("f {\n}"), [indent]);
}

#[test]
fn tuple_punctuation_must_be_indented() {
    let indent = SyntaxErrorKind::ExpectedIndent;
    assert_eq!(PATTERN.errors("f ( a\n, b\n)"), [indent, indent]);
    assert_eq!(PATTERN.errors("f ( a\n)"), [indent]);
    assert_eq!(PATTERN.errors("f (\n)"), [indent]);
}

#[test]
fn an_unclosed_bracket_stops_at_column_zero() {
    for (text, closer) in [
        ("f (a\nb", K::RParen),
        ("f [a\nb", K::RBracket),
        ("f {a\nb", K::RBrace),
    ] {
        assert_eq!(
            PATTERN.errors(text),
            [SyntaxErrorKind::Expected(closer)],
            "{text}"
        );
        assert!(PATTERN.render(text).contains("\n  Error@5..6\n"), "{text}");
    }
}

#[test]
fn deep_nesting_is_one_error() {
    let n = DEPTH_LIMIT as usize + 5_000;
    nomi_base::on_core_stack(|| {
        let parens = format!("f {}x{}", "(".repeat(n), ")".repeat(n));
        assert_eq!(PATTERN.parse(&parens).too_deep(), 1);
        let lists = format!("f {}x{}", "[".repeat(n), "]".repeat(n));
        assert_eq!(PATTERN.parse(&lists).too_deep(), 1);
        let cons = format!("f {}x", "x :: ".repeat(n));
        assert_eq!(PATTERN.parse(&cons).too_deep(), 1);
    });
}
