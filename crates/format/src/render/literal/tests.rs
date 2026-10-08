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
fn floats_print_ghcs_digits() {
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
    assert_eq!(exponent(f64::INFINITY), "1.0e309");
    assert_eq!(exponent(f64::NEG_INFINITY), "-1.0e309");
    assert_eq!(fixed(f64::INFINITY), format!("1{}.0", "0".repeat(309)));
    assert_eq!("1.0e309".parse(), Ok(f64::INFINITY));
    assert_eq!("1.0e308".parse::<f64>().map(f64::is_finite), Ok(true));
    // GHC excludes the rounding boundaries, so digits lying exactly on one
    // are passed over for longer ones.
    assert_eq!(exponent(1e23), "9.999999999999999e22");
    assert_eq!(exponent(1.2345e21), "1.2344999999999999e21");
    assert_eq!(fixed(1e23), "99999999999999990000000.0");
    assert_eq!(exponent(5e-324), "5.0e-324");
    assert_eq!(exponent(f64::MAX), "1.7976931348623157e308");
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
    // Unicode 15.1's tables: U+0378 and U+1CC00 (assigned in 16.0) are
    // unassigned, U+2EBF0 (assigned in 15.1) prints, and U+110BD is a
    // format character.
    assert_eq!(
        single("\u{378}\u{1CC00}\u{2EBF0}\u{110BD}"),
        "\"\\u{0378}\\u{1CC00}\u{2EBF0}\\u{110BD}\""
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
