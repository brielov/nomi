//! Layout at the top level, where the fixtures' fragments can't reach:
//! a type ends at the first token that isn't indented.

use super::*;
use crate::grammar::testing::TopLevel;

/// A type as in a top-level annotation, after the `:`.
const TYPE: TopLevel = TopLevel {
    prefix: 1,
    rule: type_,
    root: K::TypeFragment,
};

#[test]
fn arguments_stop_at_column_zero() {
    TYPE.check(
        ": List\nx",
        r#"
TypeFragment@0..8
  Colon@0..1 ":"
  Whitespace@1..2 " "
  NamedType@2..6
    UpperIdent@2..6 "List"
  Newline@6..7 "\n"
  Error@7..8
    LowerIdent@7..8 "x"
"#,
    );
}

#[test]
fn an_arrow_at_column_zero_ends_the_type() {
    TYPE.check(
        ": a\n-> b",
        r#"
TypeFragment@0..8
  Colon@0..1 ":"
  Whitespace@1..2 " "
  TypeVar@2..3
    LowerIdent@2..3 "a"
  Newline@3..4 "\n"
  Error@4..8
    Arrow@4..6 "->"
    Whitespace@6..7 " "
    LowerIdent@7..8 "b"
"#,
    );
}

#[test]
fn a_type_at_column_zero_is_missing() {
    TYPE.check(
        ":\nInt",
        r#"
TypeFragment@0..5
  Colon@0..1 ":"
  Missing@1..1
  Newline@1..2 "\n"
  Error@2..5
    UpperIdent@2..5 "Int"
error 2..5: ExpectedType
"#,
    );
}

#[test]
fn record_punctuation_must_be_indented() {
    TYPE.check(
        ": { a : Int\n, b : Int\n}",
        r#"
TypeFragment@0..23
  Colon@0..1 ":"
  Whitespace@1..2 " "
  RecordType@2..23
    LBrace@2..3 "{"
    Whitespace@3..4 " "
    FieldType@4..11
      LowerIdent@4..5 "a"
      Whitespace@5..6 " "
      Colon@6..7 ":"
      Whitespace@7..8 " "
      NamedType@8..11
        UpperIdent@8..11 "Int"
    Newline@11..12 "\n"
    Comma@12..13 ","
    Whitespace@13..14 " "
    FieldType@14..21
      LowerIdent@14..15 "b"
      Whitespace@15..16 " "
      Colon@16..17 ":"
      Whitespace@17..18 " "
      NamedType@18..21
        UpperIdent@18..21 "Int"
    Newline@21..22 "\n"
    RBrace@22..23 "}"
error 12..13: ExpectedIndent
error 22..23: ExpectedIndent
"#,
    );
}

#[test]
fn an_empty_record_closes_indented() {
    TYPE.check(
        ": {\n}",
        r#"
TypeFragment@0..5
  Colon@0..1 ":"
  Whitespace@1..2 " "
  RecordType@2..5
    LBrace@2..3 "{"
    Newline@3..4 "\n"
    RBrace@4..5 "}"
error 4..5: ExpectedIndent
"#,
    );
}

#[test]
fn an_unclosed_tuple_stops_at_column_zero() {
    TYPE.check(
        ": (a, b\nc",
        r#"
TypeFragment@0..9
  Colon@0..1 ":"
  Whitespace@1..2 " "
  TupleType@2..7
    LParen@2..3 "("
    TypeVar@3..4
      LowerIdent@3..4 "a"
    Comma@4..5 ","
    Whitespace@5..6 " "
    TypeVar@6..7
      LowerIdent@6..7 "b"
  Newline@7..8 "\n"
  Error@8..9
    LowerIdent@8..9 "c"
error 8..9: Expected(RParen)
"#,
    );
}

#[test]
fn deep_nesting_gives_up_on_the_rest() {
    let text = format!(": {}a{}", "(".repeat(30_000), ")".repeat(30_000));
    nomi_base::on_core_stack(|| {
        let parse = TYPE.parse(&text);
        let kinds: Vec<_> = parse.errors().iter().map(|e| e.kind).collect();
        assert_eq!(kinds, [SyntaxErrorKind::TooDeep]);
        assert_eq!(parse.syntax().to_string(), text);
    });
}
