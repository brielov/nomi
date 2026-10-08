//! Expected positions follow upstream's code in `Json/Decode.hs` for each
//! release; the `elm.json` fixtures check them against the binaries.

use nomi_base::Compat;
use proptest::prelude::*;

use super::decode::{self, Expectation, Problem};
use super::encode;
use super::*;

fn error(src: &str, compat: Compat) -> (ParseErrorKind, u32, u32) {
    error_bytes(src.as_bytes(), compat)
}

fn error_bytes(src: &[u8], compat: Compat) -> (ParseErrorKind, u32, u32) {
    let e = parse(src, compat).expect_err("a parse error");
    (e.kind, e.at.row, e.at.col)
}

/// The error every release reports for `src`.
fn same_error(src: &str) -> (ParseErrorKind, u32, u32) {
    let e = error(src, Compat::V0_19_1);
    for compat in Compat::ALL {
        assert_eq!(error(src, compat), e, "{src:?} in {compat}");
    }
    e
}

use ParseErrorKind as K;
use StringProblem as S;

#[test]
fn missing_values() {
    assert_eq!(same_error(""), (K::Start, 1, 1));
    assert_eq!(same_error("  "), (K::Start, 1, 3));
    assert_eq!(same_error("-1"), (K::Start, 1, 1));
    assert_eq!(same_error("[1,]"), (K::Start, 1, 4));
    assert_eq!(same_error("["), (K::Start, 1, 2));
    assert_eq!(same_error("[}"), (K::Start, 1, 2));
    assert_eq!(same_error(r#"{"a": }"#), (K::Start, 1, 7));
}

#[test]
fn objects() {
    assert_eq!(same_error("{"), (K::ObjectField, 1, 2));
    assert_eq!(same_error("{1}"), (K::ObjectField, 1, 2));
    assert_eq!(same_error(r#"{"a" 1}"#), (K::ObjectColon, 1, 6));
    assert_eq!(same_error(r#"{"a":1,}"#), (K::ObjectField, 1, 8));
    assert_eq!(same_error(r#"{"a":1 "b":2}"#), (K::ObjectEnd, 1, 8));
    assert_eq!(same_error(r#"{"a":1"#), (K::ObjectEnd, 1, 7));
}

#[test]
fn arrays() {
    assert_eq!(same_error("[1 2]"), (K::ArrayEnd, 1, 4));
    assert_eq!(same_error("[1"), (K::ArrayEnd, 1, 3));
}

#[test]
fn ints() {
    assert_eq!(same_error("01"), (K::NoLeadingZeros, 1, 2));
    assert_eq!(same_error("0.5"), (K::NoFloats, 1, 2));
    assert_eq!(same_error("12.5"), (K::NoFloats, 1, 3));
    assert_eq!(same_error("1e5"), (K::NoFloats, 1, 2));
    assert_eq!(same_error("10E5"), (K::NoFloats, 1, 3));
    // Only a `.` stops a zero; the rest is left over.
    assert_eq!(same_error("0e5"), (K::BadEnd, 1, 2));
    let wrapped = parse(b"9223372036854775808", Compat::V0_19_3).unwrap();
    assert_eq!(wrapped.kind, Kind::Int(i64::MIN));
}

#[test]
fn keywords_end_where_names_do() {
    assert_eq!(same_error("truex"), (K::Start, 1, 1));
    assert_eq!(same_error("null_"), (K::Start, 1, 1));
    assert_eq!(same_error("false1"), (K::Start, 1, 1));
    assert_eq!(same_error("trueé"), (K::Start, 1, 1));
    // Not a letter, so the keyword ends and the rest is left over.
    assert_eq!(same_error("true→"), (K::BadEnd, 1, 5));
    assert_eq!(same_error("true-"), (K::BadEnd, 1, 5));
    for compat in Compat::ALL {
        assert_eq!(
            error_bytes(b"true\xC3", compat),
            (K::BadEnd, 1, 5),
            "{compat}"
        );
    }
    // 0.19.1 assembles the character without checking it: 0xC3 0xE9 makes
    // U+0129 there, a letter.
    assert_eq!(
        error_bytes(b"true\xC3\xE9", Compat::V0_19_1),
        (K::Start, 1, 1)
    );
    assert_eq!(
        error_bytes(b"true\xC3\xE9", Compat::V0_19_3),
        (K::BadEnd, 1, 5)
    );
}

#[test]
fn strings() {
    assert_eq!(same_error("\"a\nb\""), (K::String(S::BadStringEnd), 1, 3));
    assert_eq!(same_error("\"abc"), (K::String(S::BadStringEnd), 1, 5));
    assert_eq!(
        same_error(r#""\x""#),
        (K::String(S::BadStringEscapeChar), 1, 2)
    );
    assert_eq!(
        same_error(r#"" \u12""#),
        (K::String(S::BadStringEscapeHex), 1, 3)
    );
    assert_eq!(
        same_error(r#""\u12"#),
        (K::String(S::BadStringEscapeHex), 1, 2)
    );
    assert_eq!(
        same_error("\"\t\""),
        (K::String(S::BadStringControlChar), 1, 2)
    );
    assert_eq!(
        same_error("\"\r\""),
        (K::String(S::BadStringControlChar), 1, 2)
    );
    // A character is a column, however many bytes it takes.
    assert_eq!(same_error("\"é→😀\" x"), (K::BadEnd, 1, 7));
    assert_eq!(same_error(r#""\u00e9\n\"" x"#), (K::BadEnd, 1, 14));
}

#[test]
fn a_backslash_at_the_end() {
    assert_eq!(
        error(r#""a\"#, Compat::V0_19_1),
        (K::String(S::BadStringEnd), 2, 3)
    );
    for compat in [Compat::V0_19_2, Compat::V0_19_3] {
        assert_eq!(error(r#""a\"#, compat), (K::String(S::BadStringEnd), 1, 4));
    }
}

#[test]
fn bytes_that_are_not_utf8() {
    for (src, col) in [
        (&b"\"\xFF\""[..], 2),
        (b"\"a\x80\"", 3),
        (b"\"\xC0\xAF\"", 2),
        (b"\"\xED\xA0\x80\"", 2),
        (b"\"\xE2\x82\"", 2),
    ] {
        for compat in [Compat::V0_19_2, Compat::V0_19_3] {
            assert_eq!(
                error_bytes(src, compat),
                (K::String(S::BadStringNotUtf8), 1, col),
                "{src:?}"
            );
        }
    }
    assert_eq!(error_bytes(b"\"\xFF\"", Compat::V0_19_1), (K::Crash, 1, 2));
    assert_eq!(error_bytes(b"\"a\x80\"", Compat::V0_19_1), (K::Crash, 1, 3));
    // 0.19.1 goes by the lead byte alone, past the end if it says so.
    assert!(parse(b"\"\xC0\xAF\"", Compat::V0_19_1).is_ok());
    assert!(parse(b"\"\xED\xA0\x80\"", Compat::V0_19_1).is_ok());
    assert_eq!(
        error_bytes(b"\"\xF0\"", Compat::V0_19_1),
        (K::String(S::BadStringEnd), 1, 3)
    );
}

#[test]
fn rows_and_columns() {
    assert_eq!(same_error("{\n  \"a\": 1,\n}"), (K::ObjectField, 3, 1));
    // A carriage return takes no column.
    assert_eq!(same_error("{\r\n  1"), (K::ObjectField, 2, 3));
    assert_eq!(same_error("\t\tx"), (K::Start, 1, 3));
}

#[test]
fn columns_wrap_in_0_19_1() {
    let src = format!("{}x", " ".repeat(65536));
    assert_eq!(error(&src, Compat::V0_19_1), (K::Start, 1, 1));
    assert_eq!(error(&src, Compat::V0_19_2), (K::Start, 1, 65537));
    let src = format!("{}x", "\n".repeat(65535));
    assert_eq!(error(&src, Compat::V0_19_1), (K::Start, 0, 1));
    assert_eq!(error(&src, Compat::V0_19_3), (K::Start, 65536, 1));
}

#[test]
fn regions_and_snippets() {
    let src = br#"{ "key": [1, true],
  "s": "a\"b" }"#;
    let value = parse(src, Compat::V0_19_3).unwrap();
    let at = |row, col| Position { row, col };
    assert_eq!(value.region.start, at(1, 1));
    assert_eq!(value.region.end, at(2, 16));
    let Kind::Object(fields) = &value.kind else {
        panic!("an object")
    };
    let (key, array) = &fields[0];
    assert_eq!(key.bytes, b"key");
    assert_eq!(
        key.region(),
        Region {
            start: at(1, 4),
            end: at(1, 7)
        }
    );
    assert_eq!(
        array.region,
        Region {
            start: at(1, 10),
            end: at(1, 19)
        }
    );
    let Kind::String(s) = &fields[1].1.kind else {
        panic!("a string")
    };
    assert_eq!(s.bytes, br#"a\"b"#);
    assert_eq!(s.start.position(), at(2, 9));
}

fn decoded<'a>(src: &'a str) -> Value<'a> {
    parse(src.as_bytes(), Compat::V0_19_3).unwrap()
}

#[test]
fn fields_wrap_their_problems() {
    let value = decoded(r#"{"a": {"b": 1}}"#);
    let p = decode::field(&value, b"a", |a| {
        decode::field(a, b"b", decode::string::<()>)
    });
    let region = Region {
        start: Position { row: 1, col: 13 },
        end: Position { row: 1, col: 14 },
    };
    assert_eq!(
        p,
        Err(Problem::Field(
            b"a".to_vec(),
            Box::new(Problem::Field(
                b"b".to_vec(),
                Box::new(Problem::Expecting(region, Expectation::String))
            ))
        ))
    );
    let missing = decode::field(&value, b"c", decode::int::<()>);
    assert!(matches!(missing, Err(Problem::Expecting(_, Expectation::ObjectWith(k))) if k == b"c"));
    // The first of duplicate fields counts.
    let value = decoded(r#"{"a": 1, "a": 2}"#);
    assert_eq!(decode::field(&value, b"a", decode::int::<()>), Ok(1));
}

#[test]
fn lists_index_their_problems() {
    let value = decoded(r#"[1, "x"]"#);
    let p = decode::list(&value, decode::int::<()>);
    assert!(matches!(p, Err(Problem::Index(1, _))));
    let empty = decoded("[]");
    assert!(matches!(
        decode::non_empty_list(&empty, decode::int, "none"),
        Err(Problem::Failure(_, "none"))
    ));
    assert!(matches!(
        decode::pair(&value, decode::int::<()>, decode::int),
        Err(Problem::Index(1, _))
    ));
    assert!(matches!(
        decode::pair(&empty, decode::int::<()>, decode::int),
        Err(Problem::Expecting(_, Expectation::ArrayPair(0)))
    ));
}

#[test]
fn keys_fail_on_their_own_region() {
    let value = decoded(r#"{"ok": 1, "bad": 2}"#);
    let key = |bytes: &[u8], _| {
        if bytes == b"ok" {
            Ok(())
        } else {
            Err("bad key")
        }
    };
    let p = decode::pairs(&value, key, decode::int);
    let region = Region {
        start: Position { row: 1, col: 12 },
        end: Position { row: 1, col: 15 },
    };
    assert_eq!(p, Err(Problem::Failure(region, "bad key")));
    // Dicts keep the last of equal keys.
    let value = decoded(r#"{"b": 1, "a": 2, "b": 3}"#);
    let dict = decode::dict(&value, |k, _| Ok::<_, ()>(k.to_vec()), decode::int).unwrap();
    assert_eq!(
        dict.into_iter().collect::<Vec<_>>(),
        [(b"a".to_vec(), 2), (b"b".to_vec(), 3)]
    );
}

#[test]
fn one_of_lists_every_problem_in_order() {
    let value = decoded("true");
    let p = decode::one_of(
        &value,
        &mut [
            &mut |v: &Value<'_>| decode::int::<()>(v).map(|_| ()),
            &mut |v: &Value<'_>| decode::string(v).map(|_| ()),
            &mut |v: &Value<'_>| decode::list(v, decode::int).map(|_| ()),
        ],
    );
    let Err(Problem::OneOf(first, rest)) = p else {
        panic!("{p:?}")
    };
    assert!(matches!(*first, Problem::Expecting(_, Expectation::Int)));
    assert!(matches!(
        rest.as_slice(),
        [
            Problem::Expecting(_, Expectation::String),
            Problem::Expecting(_, Expectation::Array)
        ]
    ));
}

#[test]
fn custom_strings_start_where_the_string_does() {
    let value = decoded(r#"  "abc""#);
    let start = decode::custom_string(&value, |bytes, cur| Ok::<_, ()>((bytes.len(), cur)));
    assert_eq!(start.map(|(n, cur)| (n, cur.position().col)), Ok((3, 4)));
    let failed = decode::custom_string(&value, |_, _| Err::<(), _>("no"));
    assert!(matches!(failed, Err(Problem::Failure(r, "no")) if r == value.region));
}

#[test]
fn write_indents_by_four() {
    let value = encode::Value::Object(vec![
        (b"type".to_vec(), encode::Value::string("application")),
        (
            b"dirs".to_vec(),
            encode::Value::Array(vec![encode::Value::chars(b"src"), encode::Value::Int(-1)]),
        ),
        (
            b"deps".to_vec(),
            encode::Value::Object(vec![
                (b"direct".to_vec(), encode::Value::Object(vec![])),
                (b"indirect".to_vec(), encode::Value::Array(vec![])),
            ]),
        ),
        (b"n".to_vec(), encode::Value::Null),
        (b"b".to_vec(), encode::Value::Bool(false)),
    ]);
    let expected = r#"{
    "type": "application",
    "dirs": [
        "src",
        -1
    ],
    "deps": {
        "direct": {},
        "indirect": []
    },
    "n": null,
    "b": false
}
"#;
    assert_eq!(String::from_utf8(encode::write(&value)).unwrap(), expected);
}

#[test]
fn chars_escape_four_characters() {
    assert_eq!(
        encode::Value::chars(b"a\r\n\"\\/\t\xC3\xA9"),
        encode::Value::String(b"a\\r\\n\\\"\\\\/\t\xC3\xA9".to_vec())
    );
}

#[test]
fn deep_nesting() {
    // Writing it would take quadratic space, as upstream's indentation
    // does; reading and converting it must merely not overflow the stack.
    let src = format!("{}{}", "[".repeat(100_000), "]".repeat(100_000));
    let value = parse(src.as_bytes(), Compat::V0_19_3).unwrap();
    let encoded = encode::Value::from(&value);
    assert!(matches!(encoded, encode::Value::Array(ref a) if a.len() == 1));
}

fn json() -> impl Strategy<Value = encode::Value> {
    let string = proptest::collection::vec(
        prop_oneof![
            any::<char>()
                .prop_filter("no quote, backslash or control", |c| {
                    !matches!(c, '"' | '\\') && !c.is_ascii_control()
                })
                .prop_map(|c| c.to_string()),
            Just(r"\n".to_owned()),
            Just(r#"\""#.to_owned()),
            Just(r"\u00E9".to_owned()),
        ],
        0..6,
    )
    .prop_map(|parts| parts.concat().into_bytes());
    let leaf = prop_oneof![
        string.clone().prop_map(encode::Value::String),
        (0..i64::MAX).prop_map(encode::Value::Int),
        any::<bool>().prop_map(encode::Value::Bool),
        Just(encode::Value::Null),
    ];
    leaf.prop_recursive(4, 32, 4, move |inner| {
        prop_oneof![
            proptest::collection::vec(inner.clone(), 0..4).prop_map(encode::Value::Array),
            proptest::collection::vec((string.clone(), inner), 0..4)
                .prop_map(encode::Value::Object),
        ]
    })
}

proptest! {
    #[test]
    fn what_is_written_reads_back(value in json()) {
        let written = encode::write(&value);
        for compat in Compat::ALL {
            let parsed = parse(&written, compat).unwrap();
            prop_assert_eq!(&encode::Value::from(&parsed), &value);
        }
    }
}
