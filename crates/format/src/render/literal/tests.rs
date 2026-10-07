//! Expected spellings are elm-format 0.8.8's.

use super::*;

fn spell(value: LiteralValue) -> String {
    literal(&value).render().trim_end_matches('\n').to_owned()
}

fn code_points(text: &str) -> Vec<u32> {
    text.chars().map(u32::from).collect()
}

#[test]
fn integers_keep_their_base() {
    assert_eq!(spell(LiteralValue::Int(255, IntRepr::Decimal)), "255");
    assert_eq!(spell(LiteralValue::Int(-5, IntRepr::Decimal)), "-5");
    assert_eq!(spell(LiteralValue::Int(0x1F, IntRepr::Hexadecimal)), "0x1F");
    assert_eq!(
        spell(LiteralValue::Int(-0x100, IntRepr::Hexadecimal)),
        "-0x0100"
    );
    assert_eq!(
        spell(LiteralValue::Int(0xF_FFFF_FFFF, IntRepr::Hexadecimal)),
        "0x0000000FFFFFFFFF"
    );
    assert_eq!(
        spell(LiteralValue::Int(i64::MIN, IntRepr::Hexadecimal)),
        "-0x8000000000000000"
    );
}

#[test]
fn floats_print_their_shortest_digits() {
    let fixed = |f| spell(LiteralValue::Float(f, FloatRepr::Decimal));
    let exponent = |f| spell(LiteralValue::Float(f, FloatRepr::Exponent));
    assert_eq!(fixed(1.5), "1.5");
    assert_eq!(fixed(0.0), "0.0");
    assert_eq!(fixed(0.000_123), "0.000123");
    assert_eq!(fixed(123_456_789.125), "123456789.125");
    assert_eq!(fixed(1e22), "10000000000000000000000.0");
    assert_eq!(fixed(-2.0), "-2.0");
    assert_eq!(exponent(1e10), "1.0e10");
    assert_eq!(exponent(1e22), "1.0e22");
    assert_eq!(exponent(1.5e-7), "1.5e-7");
    assert_eq!(exponent(f64::INFINITY), "Infinity");
}

#[test]
fn strings_escape_what_does_not_print() {
    let single = |s: &str| {
        spell(LiteralValue::String(
            code_points(s),
            StringRepr::SingleQuoted,
        ))
    };
    assert_eq!(
        single("a\"b\\c\td\u{200B}e\u{7}"),
        r#""a\"b\\c\td\u{200B}e\u{0007}""#
    );
    assert_eq!(
        single("\u{1F600}\u{E000}\u{2028}\u{AD}\u{3000}\r"),
        "\"\u{1F600}\\u{E000}\\u{2028}\\u{00AD}\\u{3000}\\u{000D}\""
    );
    // A lone surrogate, as an escape decodes it.
    assert_eq!(
        spell(LiteralValue::String(vec![0xD835], StringRepr::SingleQuoted)),
        r#""\u{D835}""#
    );
}

#[test]
fn characters_escape_their_quote() {
    let char = |c: char| spell(LiteralValue::Char(u32::from(c)));
    assert_eq!(char('a'), "'a'");
    assert_eq!(char('\''), r"'\''");
    assert_eq!(char('"'), "'\"'");
    assert_eq!(char('\u{A0}'), r"'\u{00A0}'");
}

#[test]
fn triple_quoted_strings_keep_newlines_and_short_quote_runs() {
    let triple = |s: &str| {
        spell(LiteralValue::String(
            code_points(s),
            StringRepr::TripleQuoted,
        ))
    };
    assert_eq!(triple("x\ny\tz\u{0}"), "\"\"\"x\ny\\tz\\u{0000}\"\"\"");
    assert_eq!(triple("a\"b\"\"c"), "\"\"\"a\"b\"\"c\"\"\"");
    assert_eq!(triple("a\"\"\"b\""), r#""""a\"\"\"b\"""""#);
}

#[test]
fn booleans() {
    assert_eq!(spell(LiteralValue::Bool(true)), "True");
}
