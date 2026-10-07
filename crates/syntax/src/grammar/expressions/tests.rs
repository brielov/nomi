//! Layout at the top level, where the fixtures' fragments can't reach:
//! an expression ends at the first token that isn't indented.

use super::*;
use crate::parser::{DEPTH_LIMIT, Parse};

/// Parses an expression as a top-level definition's body would be,
/// after `x =`, then sweeps up the rest.
fn top_level(text: &str) -> Parse {
    let mut p = Parser::new(text);
    let m = p.start();
    p.anchor();
    p.bump();
    p.bump();
    expr(&mut p);
    if !p.at(K::Eof) {
        let rest = p.start();
        p.with_min_column(0, |p| {
            while !p.at(K::Eof) {
                p.bump();
            }
        });
        rest.complete(&mut p, K::Error);
    }
    m.complete(&mut p, K::ExprFragment);
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
fn arguments_and_operators_stop_at_column_zero() {
    check(
        "x = f\ny\n+ z",
        r#"
ExprFragment@0..11
  LowerIdent@0..1 "x"
  Whitespace@1..2 " "
  Eq@2..3 "="
  Whitespace@3..4 " "
  NameExpr@4..5
    LowerIdent@4..5 "f"
  Newline@5..6 "\n"
  Error@6..11
    LowerIdent@6..7 "y"
    Newline@7..8 "\n"
    Operator@8..9 "+"
    Whitespace@9..10 " "
    LowerIdent@10..11 "z"
"#,
    );
}

#[test]
fn a_negative_argument_at_column_zero_ends_the_call() {
    let tree = render("x = f\n-1");
    assert!(tree.contains("\n  NameExpr@4..5\n"), "{tree}");
    assert!(tree.contains("\n  Error@6..8\n"), "{tree}");
}

#[test]
fn an_indented_continuation_carries_on() {
    let text = "x = f\n y\n -1\n + z";
    let tree = render(text);
    assert!(
        tree.contains("\n  OperatorChain@4..17\n    CallExpr@4..12\n"),
        "{tree}"
    );
    assert_eq!(errors(text), []);
}

#[test]
fn an_expression_at_column_zero_is_missing() {
    check(
        "x =\ny",
        r#"
ExprFragment@0..5
  LowerIdent@0..1 "x"
  Whitespace@1..2 " "
  Eq@2..3 "="
  Missing@3..3
  Newline@3..4 "\n"
  Error@4..5
    LowerIdent@4..5 "y"
error 4..5: ExpectedExpression
"#,
    );
}

#[test]
fn a_negation_at_column_zero_is_missing() {
    assert_eq!(errors("x =\n-y"), [SyntaxErrorKind::ExpectedExpression]);
    assert!(render("x =\n-y").contains("\n  Error@4..6\n"));
}

#[test]
fn an_operand_at_column_zero_is_missing() {
    assert_eq!(errors("x = a +\nb"), [SyntaxErrorKind::ExpectedExpression]);
    assert!(render("x = a +\nb").contains("    Missing@7..7\n"));
}

#[test]
fn punctuation_must_be_indented() {
    let indent = SyntaxErrorKind::ExpectedIndent;
    assert_eq!(errors("x = [ a\n, b\n]"), [indent, indent]);
    assert_eq!(errors("x = { a = 1\n, b = 2\n}"), [indent, indent]);
    assert_eq!(errors("x = { a\n= 1 }"), [indent]);
    assert_eq!(errors("x = { a : 1, b\n: 2 }"), [indent]);
    assert_eq!(errors("x = { a = 1,\nb = 2 }"), [indent]);
    assert_eq!(errors("x = { r\n| a = 1 }"), [indent]);
    assert_eq!(errors("x = {\nr | a = 1 }"), [indent]);
    assert_eq!(errors("x = [\n]"), [indent]);
    assert_eq!(errors("x = {\n}"), [indent]);
    assert_eq!(errors("x = (\n)"), [indent]);
    assert_eq!(
        errors("x = (\n+)"),
        [SyntaxErrorKind::SpacedOperator, indent]
    );
    assert_eq!(errors("x = (\n,)"), [indent]);
    assert_eq!(errors("x = ( a\n, b )"), [indent]);
}

#[test]
fn a_closing_parenthesis_must_be_indented() {
    assert_eq!(errors("x = ( a\n)"), [SyntaxErrorKind::ExpectedIndent]);
}

#[test]
fn an_unclosed_bracket_stops_at_column_zero() {
    for (text, closer) in [
        ("x = (a\nb", K::RParen),
        ("x = [a\nb", K::RBracket),
        ("x = {a=1\nb", K::RBrace),
    ] {
        assert_eq!(errors(text), [SyntaxErrorKind::Expected(closer)], "{text}");
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
        let parens = format!("x = {}y{}", "(".repeat(n), ")".repeat(n));
        assert_eq!(too_deep(&parens), 1);
        let lists = format!("x = {}y{}", "[".repeat(n), "]".repeat(n));
        assert_eq!(too_deep(&lists), 1);
        let records = format!("x = {}y{}", "{a=".repeat(n), "}".repeat(n));
        assert_eq!(too_deep(&records), 1);
        let negations = format!("x = {}y{}", "-(".repeat(n), ")".repeat(n));
        assert_eq!(too_deep(&negations), 1);
        let accesses = format!("x = y{}", ".a".repeat(n));
        assert_eq!(too_deep(&accesses), 1);
        let lambdas = format!("x = {}y", "\\a -> ".repeat(n));
        assert_eq!(too_deep(&lambdas), 1);
        let lets = format!("x = {}y", "let a = 1 in ".repeat(n));
        assert_eq!(too_deep(&lets), 1);
        let ifs = format!("x = {}y", "if a then b else ".repeat(n));
        assert_eq!(too_deep(&ifs), 1);
    });
}

#[test]
fn long_flat_chains_are_not_deep() {
    let n = DEPTH_LIMIT as usize * 2;
    let chain = format!("x = y{}", " + y".repeat(n));
    assert_eq!(errors(&chain), []);
    let call = format!("x = f{}", " y".repeat(n));
    assert_eq!(errors(&call), []);
}

#[test]
fn long_blocks_are_not_deep() {
    let n = DEPTH_LIMIT as usize * 2;
    let case = format!("x = case y of{}", "\n  _ -> 1".repeat(n));
    assert_eq!(errors(&case), []);
    let let_ = format!("x = let{} in 1", "\n      a = 1".repeat(n));
    assert_eq!(errors(&let_), []);
}

#[test]
fn a_case_ends_at_column_zero() {
    let tree = render("x = case a of\n  B -> 1\ny");
    assert!(tree.contains("\n  CaseExpr@4..22\n"), "{tree}");
    assert!(tree.contains("\n  Error@23..24\n"), "{tree}");
}

#[test]
fn branch_bodies_are_right_of_the_branch() {
    // The `1` starts another branch, one without an arrow.
    assert_eq!(
        errors("x = case a of\n  B ->\n  1"),
        [
            SyntaxErrorKind::ExpectedExpression,
            SyntaxErrorKind::Expected(K::Arrow)
        ]
    );
    assert_eq!(errors("x = case a of\n  B ->\n   1"), []);
}

#[test]
fn a_case_inside_a_branch_takes_the_branches_right_of_it() {
    let text = "x = case a of\n  B ->\n    case b of\n      C -> 1\n  D -> 2";
    assert_eq!(errors(text), []);
    let tree = render(text);
    assert!(tree.contains("\n      CaseExpr@25..47\n"), "{tree}");
    assert!(tree.contains("\n    CaseBranch@50..56\n"), "{tree}");
}

#[test]
fn let_definitions_are_right_of_the_let() {
    assert_eq!(errors("x =\n let\n  a = 1\n in a"), []);
    let indent = SyntaxErrorKind::ExpectedIndent;
    assert_eq!(errors("x =\n  let\n  a = 1\n  in a"), [indent]);
    assert_eq!(errors("x = let\n a = 1\n in a"), [indent]);
}

#[test]
fn in_must_be_indented() {
    assert_eq!(
        errors("x = let a = 1\nin a"),
        [SyntaxErrorKind::ExpectedIndent]
    );
}

#[test]
fn then_and_else_must_be_indented() {
    let indent = SyntaxErrorKind::ExpectedIndent;
    assert_eq!(errors("x = if a\nthen b else c"), [indent]);
    assert_eq!(errors("x = if a then b\nelse c"), [indent]);
    assert_eq!(errors("x = if a\n then b\n else c"), []);
}
