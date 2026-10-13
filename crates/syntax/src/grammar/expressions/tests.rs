//! Layout at the top level, where the fixtures' fragments can't reach:
//! an expression ends at the first token that isn't indented.

use super::*;
use crate::grammar::testing::TopLevel;
use crate::parser::DEPTH_LIMIT;

/// An expression as a top-level definition's body, after `x =`.
const EXPR: TopLevel = TopLevel {
    prefix: 2,
    rule: expr,
    root: K::ExprFragment,
};

#[test]
fn arguments_and_operators_stop_at_column_zero() {
    EXPR.check(
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
    let tree = EXPR.render("x = f\n-1");
    assert!(tree.contains("\n  NameExpr@4..5\n"), "{tree}");
    assert!(tree.contains("\n  Error@6..8\n"), "{tree}");
}

#[test]
fn an_indented_continuation_carries_on() {
    let text = "x = f\n y\n -1\n + z";
    let tree = EXPR.render(text);
    assert!(
        tree.contains("\n  OperatorChain@4..17\n    CallExpr@4..12\n"),
        "{tree}"
    );
    assert_eq!(EXPR.errors(text), []);
}

#[test]
fn an_expression_at_column_zero_is_missing() {
    EXPR.check(
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
    assert_eq!(
        EXPR.errors("x =\n-y"),
        [SyntaxErrorKind::ExpectedExpression]
    );
    assert!(EXPR.render("x =\n-y").contains("\n  Error@4..6\n"));
}

#[test]
fn an_operand_at_column_zero_is_missing() {
    assert_eq!(
        EXPR.errors("x = a +\nb"),
        [SyntaxErrorKind::ExpectedExpression]
    );
    assert!(EXPR.render("x = a +\nb").contains("    Missing@7..7\n"));
}

#[test]
fn punctuation_must_be_indented() {
    let indent = SyntaxErrorKind::ExpectedIndent;
    assert_eq!(EXPR.errors("x = [ a\n, b\n]"), [indent, indent]);
    assert_eq!(EXPR.errors("x = { a = 1\n, b = 2\n}"), [indent, indent]);
    assert_eq!(EXPR.errors("x = { a\n= 1 }"), [indent]);
    assert_eq!(EXPR.errors("x = { a : 1, b\n: 2 }"), [indent]);
    assert_eq!(EXPR.errors("x = { a = 1,\nb = 2 }"), [indent]);
    assert_eq!(EXPR.errors("x = { r\n| a = 1 }"), [indent]);
    assert_eq!(EXPR.errors("x = {\nr | a = 1 }"), [indent]);
    assert_eq!(EXPR.errors("x = [\n]"), [indent]);
    assert_eq!(EXPR.errors("x = {\n}"), [indent]);
    assert_eq!(EXPR.errors("x = (\n)"), [indent]);
    assert_eq!(
        EXPR.errors("x = (\n+)"),
        [SyntaxErrorKind::SpacedOperator, indent]
    );
    assert_eq!(EXPR.errors("x = (\n,)"), [indent]);
    assert_eq!(EXPR.errors("x = ( a\n, b )"), [indent]);
}

#[test]
fn a_closing_parenthesis_must_be_indented() {
    assert_eq!(EXPR.errors("x = ( a\n)"), [SyntaxErrorKind::ExpectedIndent]);
}

#[test]
fn an_unclosed_bracket_stops_at_column_zero() {
    for (text, closer) in [
        ("x = (a\nb", K::RParen),
        ("x = [a\nb", K::RBracket),
        ("x = {a=1\nb", K::RBrace),
    ] {
        assert_eq!(
            EXPR.errors(text),
            [SyntaxErrorKind::Expected(closer)],
            "{text}"
        );
    }
}

#[test]
fn deep_nesting_is_one_error() {
    let n = DEPTH_LIMIT as usize + 5_000;
    nomi_base::on_core_stack(|| {
        let parens = format!("x = {}y{}", "(".repeat(n), ")".repeat(n));
        assert_eq!(EXPR.parse(&parens).too_deep(), 1);
        let lists = format!("x = {}y{}", "[".repeat(n), "]".repeat(n));
        assert_eq!(EXPR.parse(&lists).too_deep(), 1);
        let records = format!("x = {}y{}", "{a=".repeat(n), "}".repeat(n));
        assert_eq!(EXPR.parse(&records).too_deep(), 1);
        let negations = format!("x = {}y{}", "-(".repeat(n), ")".repeat(n));
        assert_eq!(EXPR.parse(&negations).too_deep(), 1);
        let accesses = format!("x = y{}", ".a".repeat(n));
        assert_eq!(EXPR.parse(&accesses).too_deep(), 1);
        let lambdas = format!("x = {}y", "\\a -> ".repeat(n));
        assert_eq!(EXPR.parse(&lambdas).too_deep(), 1);
        let lets = format!("x = {}y", "let a = 1 in ".repeat(n));
        assert_eq!(EXPR.parse(&lets).too_deep(), 1);
        let ifs = format!("x = {}y", "if a then b else ".repeat(n));
        assert_eq!(EXPR.parse(&ifs).too_deep(), 1);
    });
}

#[test]
fn long_flat_chains_are_not_deep() {
    let n = DEPTH_LIMIT as usize * 2;
    let chain = format!("x = y{}", " + y".repeat(n));
    assert_eq!(EXPR.errors(&chain), []);
    let call = format!("x = f{}", " y".repeat(n));
    assert_eq!(EXPR.errors(&call), []);
}

#[test]
fn long_blocks_are_not_deep() {
    let n = DEPTH_LIMIT as usize * 2;
    let case = format!("x = case y of{}", "\n  _ -> 1".repeat(n));
    assert_eq!(EXPR.errors(&case), []);
    let let_ = format!("x = let{} in 1", "\n      a = 1".repeat(n));
    assert_eq!(EXPR.errors(&let_), []);
}

#[test]
fn a_case_ends_at_column_zero() {
    let tree = EXPR.render("x = case a of\n  B -> 1\ny");
    assert!(tree.contains("\n  CaseExpr@4..22\n"), "{tree}");
    assert!(tree.contains("\n  Error@23..24\n"), "{tree}");
}

#[test]
fn branch_bodies_are_right_of_the_branch() {
    // The `1` starts another branch, one without an arrow.
    assert_eq!(
        EXPR.errors("x = case a of\n  B ->\n  1"),
        [
            SyntaxErrorKind::ExpectedExpression,
            SyntaxErrorKind::Expected(K::Arrow)
        ]
    );
    assert_eq!(EXPR.errors("x = case a of\n  B ->\n   1"), []);
}

#[test]
fn a_case_inside_a_branch_takes_the_branches_right_of_it() {
    let text = "x = case a of\n  B ->\n    case b of\n      C -> 1\n  D -> 2";
    assert_eq!(EXPR.errors(text), []);
    let tree = EXPR.render(text);
    assert!(tree.contains("\n      CaseExpr@25..47\n"), "{tree}");
    assert!(tree.contains("\n    CaseBranch@50..56\n"), "{tree}");
}

#[test]
fn let_definitions_are_right_of_the_let() {
    assert_eq!(EXPR.errors("x =\n let\n  a = 1\n in a"), []);
    let indent = SyntaxErrorKind::ExpectedIndent;
    assert_eq!(EXPR.errors("x =\n  let\n  a = 1\n  in a"), [indent]);
    assert_eq!(EXPR.errors("x = let\n a = 1\n in a"), [indent]);
}

#[test]
fn in_must_be_indented() {
    assert_eq!(
        EXPR.errors("x = let a = 1\nin a"),
        [SyntaxErrorKind::ExpectedIndent]
    );
}

#[test]
fn then_and_else_must_be_indented() {
    let indent = SyntaxErrorKind::ExpectedIndent;
    assert_eq!(EXPR.errors("x = if a\nthen b else c"), [indent]);
    assert_eq!(EXPR.errors("x = if a then b\nelse c"), [indent]);
    assert_eq!(EXPR.errors("x = if a\n then b\n else c"), []);
}
