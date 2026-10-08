use super::super::c;
use crate::tree::{
    Comment, FloatRepr, IntRepr, LiteralValue, Lower, Pattern, Sequence, StringRepr, Upper,
};

fn lower(text: &str) -> Pattern {
    crate::lower::pattern_fragment(text)
}

fn literal(text: &str) -> LiteralValue {
    let Pattern::Literal(value) = lower(text) else {
        panic!("{text}")
    };
    value
}

fn block(text: &str) -> Comment {
    Comment::Block(vec![text.to_owned()])
}

fn var(name: &str) -> Pattern {
    Pattern::Var(Lower(name.to_owned()))
}

fn none<T>(value: T) -> crate::tree::C2<T> {
    c((Vec::new(), Vec::new()), value)
}

fn chars(text: &str) -> Vec<u32> {
    text.chars().map(u32::from).collect()
}

#[test]
fn cons_and_alias() {
    assert_eq!(
        lower("x -- e\n :: {- c -} y {- d -} :: rest {- a -} as {- n -} all"),
        Pattern::Alias(
            Box::new(c(
                vec![block("a")],
                Pattern::Cons {
                    first: Box::new(c(Some(" e".to_owned()), var("x"))),
                    rest: Sequence(vec![
                        c((vec![], vec![block("c")], None), var("y")),
                        c((vec![block("d")], vec![], None), var("rest")),
                    ]),
                }
            )),
            c(vec![block("n")], Lower("all".to_owned()))
        )
    );
}

#[test]
fn constructors() {
    let just = |arg| Pattern::Data((vec![], Upper("Just".to_owned())), vec![arg]);
    assert_eq!(
        lower("Just {- a -} True"),
        Pattern::Data(
            (vec![], Upper("Just".to_owned())),
            vec![c(
                vec![block("a")],
                Pattern::Literal(LiteralValue::Bool(true))
            )]
        )
    );
    assert_eq!(
        lower("Basics.False"),
        Pattern::Data(
            (vec![Upper("Basics".to_owned())], Upper("False".to_owned())),
            vec![]
        )
    );
    assert_eq!(lower("Just (_)"), just(c(vec![], Pattern::Anything)));
}

#[test]
fn delimited_patterns() {
    assert_eq!(lower("((x))"), var("x"));
    assert_eq!(
        lower("( x {- p -} )"),
        Pattern::Parens(Box::new(c((vec![], vec![block("p")]), var("x"))))
    );
    assert_eq!(
        lower("( {- a -} x, y )"),
        Pattern::Tuple(vec![
            c((vec![block("a")], vec![]), var("x")),
            none(var("y"))
        ])
    );
    assert_eq!(lower("( {- u -} )"), Pattern::Unit(vec![block("u")]));
    assert_eq!(lower("[ {- e -} ]"), Pattern::EmptyList(vec![block("e")]));
    assert_eq!(lower("[ x ]"), Pattern::List(vec![none(var("x"))]));
    assert_eq!(lower("{}"), Pattern::EmptyRecord(vec![]));
    assert_eq!(
        lower("{ a {- x -}, b }"),
        Pattern::Record(vec![
            c((vec![], vec![block("x")]), Lower("a".to_owned())),
            none(Lower("b".to_owned())),
        ])
    );
}

#[test]
fn numbers() {
    assert_eq!(literal("-5"), LiteralValue::Int(-5, IntRepr::Decimal));
    assert_eq!(literal("0x1F"), LiteralValue::Int(31, IntRepr::Hexadecimal));
    assert_eq!(
        literal("9223372036854775808"),
        LiteralValue::Int(i64::MIN, IntRepr::Decimal)
    );
    assert_eq!(
        literal("-1.5"),
        LiteralValue::Float(-1.5, FloatRepr::Decimal)
    );
    assert_eq!(
        literal("1e3"),
        LiteralValue::Float(1000.0, FloatRepr::Exponent)
    );
}

#[test]
fn strings_and_characters() {
    assert_eq!(literal(r"'\n'"), LiteralValue::Char(0x0A));
    assert_eq!(literal(r"'\u{1F600}'"), LiteralValue::Char(0x1F600));
    assert_eq!(literal(r"'\x41'"), LiteralValue::Char(0x41));
    assert_eq!(
        literal(r#""a\"\u{D835}\u{DC00}""#),
        LiteralValue::String(vec![0x61, 0x22, 0xD835, 0xDC00], StringRepr::SingleQuoted)
    );
    assert_eq!(
        literal("\"\"\"é\r\n\rb\"\"\""),
        LiteralValue::String(chars("é\n\rb"), StringRepr::TripleQuoted)
    );
    // A letter after `\x`'s digits makes the escape -2, a lone 0xFE byte
    // that reads the letters after it as the rest of its sequence.
    assert_eq!(
        literal("\"\\x202Bایران\""),
        LiteralValue::String(
            [0x3D_8A1B, 0x8C].into_iter().chain(chars("ران")).collect(),
            StringRepr::SingleQuoted
        )
    );
}
