//! The machinery driven by a toy grammar: `name = expr` lines, where an
//! expression is names and parentheses joined by operators.

use super::*;
use SyntaxKind as K;

fn module(p: &mut Parser<'_>) {
    let m = p.start();
    while !p.at(K::Eof) {
        if p.at(K::LowerIdent) && p.nth(1) == K::Eq {
            let decl = p.start();
            let def = p.start();
            p.bump();
            p.bump();
            expr(p);
            def.complete(p, K::Definition);
            decl.complete(p, K::ValueDecl);
        } else {
            p.error_and_bump(SyntaxErrorKind::Expected(K::LowerIdent));
        }
    }
    m.complete(p, K::Module);
}

fn expr(p: &mut Parser<'_>) {
    if p.guarded(chain).is_none() {
        // Too deep: give up on the rest of the input.
        let m = p.start();
        while !p.at(K::Eof) {
            p.bump();
        }
        m.complete(p, K::Error);
    }
}

fn chain(p: &mut Parser<'_>) {
    let mut lhs = atom(p);
    while p.at(K::Operator) {
        let m = lhs.precede(p);
        p.bump();
        atom(p);
        lhs = m.complete(p, K::OperatorChain);
    }
}

fn atom(p: &mut Parser<'_>) -> CompletedMarker {
    match p.current() {
        K::LowerIdent => {
            let m = p.start();
            p.bump();
            m.complete(p, K::NameExpr)
        }
        K::LParen => {
            let m = p.start();
            p.bump();
            expr(p);
            p.expect(K::RParen);
            m.complete(p, K::ParenExpr)
        }
        _ => {
            p.error(SyntaxErrorKind::Expected(K::LowerIdent));
            p.missing()
        }
    }
}

fn parse(text: &str) -> Parse {
    let mut p = Parser::new(text);
    module(&mut p);
    p.finish()
}

#[track_caller]
fn check(text: &str, expected: &str) {
    let parse = parse(text);
    assert_eq!(parse.syntax().to_string(), text);
    assert_eq!(parse.debug_tree().trim(), expected.trim());
}

#[test]
fn empty_input_is_an_empty_root() {
    check("", "Module@0..0");
}

#[test]
fn trivia_only_input_goes_to_the_root() {
    check(
        "  \n-- c\n",
        r#"
Module@0..8
  Whitespace@0..2 "  "
  Newline@2..3 "\n"
  LineComment@3..7 "-- c"
  Newline@7..8 "\n"
"#,
    );
}

#[test]
fn precede_wraps_completed_nodes() {
    check(
        "x = a + b + c",
        r#"
Module@0..13
  ValueDecl@0..13
    Definition@0..13
      LowerIdent@0..1 "x"
      Whitespace@1..2 " "
      Eq@2..3 "="
      Whitespace@3..4 " "
      OperatorChain@4..13
        OperatorChain@4..9
          NameExpr@4..5
            LowerIdent@4..5 "a"
          Whitespace@5..6 " "
          Operator@6..7 "+"
          Whitespace@7..8 " "
          NameExpr@8..9
            LowerIdent@8..9 "b"
        Whitespace@9..10 " "
        Operator@10..11 "+"
        Whitespace@11..12 " "
        NameExpr@12..13
          LowerIdent@12..13 "c"
"#,
    );
}

#[test]
fn comments_above_a_node_lead_it() {
    check(
        "-- detached\n\n-- a\n{- b -} x = y\n",
        r#"
Module@0..32
  LineComment@0..11 "-- detached"
  Newline@11..12 "\n"
  Newline@12..13 "\n"
  ValueDecl@13..31
    LineComment@13..17 "-- a"
    Newline@17..18 "\n"
    BlockComment@18..25 "{- b -}"
    Whitespace@25..26 " "
    Definition@26..31
      LowerIdent@26..27 "x"
      Whitespace@27..28 " "
      Eq@28..29 "="
      Whitespace@29..30 " "
      NameExpr@30..31
        LowerIdent@30..31 "y"
  Newline@31..32 "\n"
"#,
    );
}

#[test]
fn stacked_comments_all_lead() {
    check(
        "-- a\n-- b\nx = y",
        r#"
Module@0..15
  ValueDecl@0..15
    LineComment@0..4 "-- a"
    Newline@4..5 "\n"
    LineComment@5..9 "-- b"
    Newline@9..10 "\n"
    Definition@10..15
      LowerIdent@10..11 "x"
      Whitespace@11..12 " "
      Eq@12..13 "="
      Whitespace@13..14 " "
      NameExpr@14..15
        LowerIdent@14..15 "y"
"#,
    );
}

#[test]
fn a_trailing_comment_stays_with_the_code_before_it() {
    check(
        "x = y -- why\nz = w",
        r#"
Module@0..18
  ValueDecl@0..5
    Definition@0..5
      LowerIdent@0..1 "x"
      Whitespace@1..2 " "
      Eq@2..3 "="
      Whitespace@3..4 " "
      NameExpr@4..5
        LowerIdent@4..5 "y"
  Whitespace@5..6 " "
  LineComment@6..12 "-- why"
  Newline@12..13 "\n"
  ValueDecl@13..18
    Definition@13..18
      LowerIdent@13..14 "z"
      Whitespace@14..15 " "
      Eq@15..16 "="
      Whitespace@16..17 " "
      NameExpr@17..18
        LowerIdent@17..18 "w"
"#,
    );
}

#[test]
fn an_indented_comment_leads_the_next_node() {
    // The comment starts its line, so it belongs to `b`, not to the
    // chain before it.
    check(
        "x = a +\n  -- b\n  b",
        r#"
Module@0..18
  ValueDecl@0..18
    Definition@0..18
      LowerIdent@0..1 "x"
      Whitespace@1..2 " "
      Eq@2..3 "="
      Whitespace@3..4 " "
      OperatorChain@4..18
        NameExpr@4..5
          LowerIdent@4..5 "a"
        Whitespace@5..6 " "
        Operator@6..7 "+"
        Newline@7..8 "\n"
        Whitespace@8..10 "  "
        NameExpr@10..18
          LineComment@10..14 "-- b"
          Newline@14..15 "\n"
          Whitespace@15..17 "  "
          LowerIdent@17..18 "b"
"#,
    );
}

#[test]
fn missing_and_error_nodes() {
    let parse = parse("x = ) y");
    assert_eq!(
        parse.debug_tree().trim(),
        r#"
Module@0..7
  ValueDecl@0..3
    Definition@0..3
      LowerIdent@0..1 "x"
      Whitespace@1..2 " "
      Eq@2..3 "="
      Missing@3..3
  Whitespace@3..4 " "
  Error@4..5
    RParen@4..5 ")"
  Whitespace@5..6 " "
  Error@6..7
    LowerIdent@6..7 "y"
"#
        .trim()
    );
    let errors: Vec<_> = parse.errors().iter().map(|e| (e.kind, e.range)).collect();
    let at = |a: u32, b: u32| TextRange::new(a.into(), b.into());
    assert_eq!(
        errors,
        [
            (SyntaxErrorKind::Expected(K::LowerIdent), at(4, 5)),
            (SyntaxErrorKind::Expected(K::LowerIdent), at(4, 5)),
            (SyntaxErrorKind::Expected(K::LowerIdent), at(6, 7)),
        ]
    );
}

#[test]
fn errors_at_the_end_are_empty_ranges() {
    let parse = parse("x = (y");
    let end = TextRange::empty(6.into());
    assert_eq!(
        parse.errors(),
        [SyntaxError {
            kind: SyntaxErrorKind::Expected(K::RParen),
            range: end,
        }]
    );
}

#[test]
fn lexical_and_syntax_errors_merge_in_source_order() {
    // The parser reports the unterminated string where the lexer did;
    // the lexer's report comes first.
    let parse = parse("x = )\n\"open");
    let kinds: Vec<_> = parse.errors().iter().map(|e| e.kind).collect();
    assert_eq!(
        kinds,
        [
            SyntaxErrorKind::Expected(K::LowerIdent),
            SyntaxErrorKind::Expected(K::LowerIdent),
            SyntaxErrorKind::Lexical(LexErrorKind::UnterminatedString),
            SyntaxErrorKind::Expected(K::LowerIdent),
        ]
    );
    assert!(parse.errors().is_sorted_by_key(|e| e.range.start()));
}

#[test]
fn bump_as_remaps_a_token() {
    let mut p = Parser::new("alias");
    let m = p.start();
    p.bump_as(K::AliasKw);
    m.complete(&mut p, K::Module);
    let parse = p.finish();
    assert_eq!(
        parse.debug_tree().trim(),
        "Module@0..5\n  AliasKw@0..5 \"alias\""
    );
}

#[test]
fn an_abandoned_marker_leaves_no_node() {
    let mut p = Parser::new("a b");
    let root = p.start();
    let first = p.start();
    first.abandon(&mut p);
    p.bump();
    let second = p.start();
    p.bump();
    // Not the last event any more: it stays a tombstone.
    second.abandon(&mut p);
    root.complete(&mut p, K::Module);
    let parse = p.finish();
    assert_eq!(
        parse.debug_tree().trim(),
        r#"
Module@0..3
  LowerIdent@0..1 "a"
  Whitespace@1..2 " "
  LowerIdent@2..3 "b"
"#
        .trim()
    );
}

#[test]
fn lookahead_skips_trivia() {
    let p = Parser::new("a -- c\n  =");
    assert_eq!(p.current(), K::LowerIdent);
    assert_eq!(p.nth(1), K::Eq);
    assert_eq!(p.nth(2), K::Eof);
    assert_eq!(p.current_text(), "a");
}

#[test]
#[should_panic(expected = "unconsumed")]
fn finishing_early_panics() {
    let mut p = Parser::new("a b");
    let m = p.start();
    p.bump();
    m.complete(&mut p, K::Module);
    p.finish();
}

fn nested(depth: usize) -> String {
    format!("x = {}y{}", "(".repeat(depth), ")".repeat(depth))
}

/// Runs `f` on the stack the shell gives threads that run core work.
fn on_core_stack(f: impl FnOnce()) {
    stacker::grow(nomi_base::STACK_SIZE, f);
}

fn too_deep(parse: &Parse) -> usize {
    parse
        .errors()
        .iter()
        .filter(|e| e.kind == SyntaxErrorKind::TooDeep)
        .count()
}

#[test]
fn nesting_up_to_the_limit_parses_and_drops() {
    on_core_stack(|| {
        // The declaration's expression takes one level.
        let text = nested(DEPTH_LIMIT as usize - 1);
        let deepest = parse(&text);
        assert_eq!(deepest.errors(), []);
        assert_eq!(usize::from(deepest.green().text_len()), text.len());
        drop(deepest);

        assert_eq!(too_deep(&parse(&nested(DEPTH_LIMIT as usize))), 1);
    });
}

#[test]
fn nesting_far_past_the_limit_is_an_error() {
    on_core_stack(|| {
        let text = nested(100_000);
        let parse = parse(&text);
        assert_eq!(too_deep(&parse), 1);
        assert_eq!(parse.syntax().to_string(), text);
    });
}

#[test]
fn guarded_rules_restore_the_depth() {
    let mut p = Parser::new("");
    let inner = p.guarded(|p| p.guarded(|p| p.depth));
    assert_eq!(inner, Some(Some(2)));
    assert_eq!(p.depth, 0);
}
