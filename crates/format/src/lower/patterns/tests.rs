use super::super::c;
use crate::test_support::{block, code_points, lower as name, separated, upper_ref};
use crate::tree::{Around, C2, FloatRepr, IntRepr, LiteralValue, Pattern, Sequence, StringRepr};

fn lower(text: &str) -> Pattern {
    crate::lower::pattern_fragment(text)
}

fn literal(text: &str) -> LiteralValue {
    let Pattern::Literal(value) = lower(text) else {
        panic!("{text}")
    };
    value
}

fn var(name: &str) -> Pattern {
    Pattern::Var(self::name(name))
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
                        c(separated(vec![], vec![block("c")], None), var("y")),
                        c(separated(vec![block("d")], vec![], None), var("rest")),
                    ]),
                }
            )),
            c(vec![block("n")], name("all"))
        )
    );
}

#[test]
fn constructors() {
    let just = |arg| Pattern::Data(upper_ref(&[], "Just"), vec![arg]);
    assert_eq!(
        lower("Just {- a -} True"),
        just(c(
            vec![block("a")],
            Pattern::Literal(LiteralValue::Bool(true))
        ))
    );
    assert_eq!(
        lower("Basics.False"),
        Pattern::Data(upper_ref(&["Basics"], "False"), vec![])
    );
    assert_eq!(lower("Just (_)"), just(c(vec![], Pattern::Anything)));
}

#[test]
fn delimited_patterns() {
    assert_eq!(lower("((x))"), var("x"));
    assert_eq!(
        lower("( x {- p -} )"),
        Pattern::Parens(Box::new(c(Around::new(vec![], vec![block("p")]), var("x"))))
    );
    assert_eq!(
        lower("( {- a -} x, y )"),
        Pattern::Tuple(vec![
            c(Around::new(vec![block("a")], vec![]), var("x")),
            C2::bare(var("y"))
        ])
    );
    assert_eq!(lower("( {- u -} )"), Pattern::Unit(vec![block("u")]));
    assert_eq!(lower("[ {- e -} ]"), Pattern::EmptyList(vec![block("e")]));
    assert_eq!(lower("[ x ]"), Pattern::List(vec![C2::bare(var("x"))]));
    assert_eq!(lower("{}"), Pattern::EmptyRecord(vec![]));
    assert_eq!(
        lower("{ a {- x -}, b }"),
        Pattern::Record(vec![
            c(Around::new(vec![], vec![block("x")]), name("a")),
            C2::bare(name("b")),
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
        LiteralValue::String(code_points("é\n\rb"), StringRepr::TripleQuoted)
    );
    // A letter after `\x`'s digits makes the escape -2, a lone 0xFE byte
    // that reads the letters after it as the rest of its sequence.
    assert_eq!(
        literal("\"\\x202Bایران\""),
        LiteralValue::String(
            [0x3D_8A1B, 0x8C]
                .into_iter()
                .chain(code_points("ران"))
                .collect(),
            StringRepr::SingleQuoted
        )
    );
}
