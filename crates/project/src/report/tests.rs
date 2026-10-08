//! Reports `cargo xtask oracle outline` can't record: those about more than
//! the file, which elm writes as these do (`tests/reports.rs` has the
//! rest), and those where nomi differs from an elm that hangs or dies.

use nomi_base::diagnostic::render::{Styles, render};

use super::*;
use crate::Outline;

fn plain(report: &Report, compat: Compat) -> String {
    render(&report.to_doc(), compat, 80, Styles::Plain)
}

fn to_json(report: &Report, compat: Compat) -> String {
    nomi_base::diagnostic::json::report(report, compat).to_string()
}

fn read(src: &[u8], compat: Compat) -> Report {
    let error = Outline::read(src, compat).expect_err("a bad outline");
    outline(&error, src, compat, &|c| format!("elm {c}"))
}

const APPLICATION: &str = r#"{
    "type": "application",
    "source-directories": [
        "src"
    ],
    "elm-version": "0.19.1",
    "dependencies": {
        "direct": {
            "elm/core": "1.0.5",
            "elm/json": "1.1.4"
        },
        "indirect": {}
    },
    "test-dependencies": {
        "direct": {},
        "indirect": {}
    }
}
"#;

#[test]
fn missing_source_directories() {
    assert_eq!(
        plain(
            &missing_src_dirs(&["lib".into(), "other".into()]),
            Compat::LATEST
        ),
        "\
-- MISSING SOURCE DIRECTORIES ----------------------------------------- elm.json

I need a valid elm.json file, but the \"source-directories\" field lists the
following directories:

    lib
    other

I cannot find them though. Are they missing? Are there typos?

"
    );
    assert_eq!(
        plain(&missing_src_dirs(&["lib".into()]), Compat::LATEST),
        "\
-- MISSING SOURCE DIRECTORY ------------------------------------------- elm.json

I need a valid elm.json file, but the \"source-directories\" field lists the
following directory:

    lib

I cannot find it though. Is it missing? Is there a typo?

"
    );
}

#[test]
fn duplicate_source_directories() {
    assert_eq!(
        plain(&duplicate_src_dirs("/p/src", "src", "src"), Compat::LATEST),
        "\
-- REDUNDANT SOURCE DIRECTORIES --------------------------------------- elm.json

I need a valid elm.json file, but the \"source-directories\" field lists the same
directory twice:

    src
    src

Remove one of the entries!

"
    );
    assert_eq!(
        plain(
            &duplicate_src_dirs("/p/src", "./src", "src"),
            Compat::LATEST
        ),
        "\
-- REDUNDANT SOURCE DIRECTORIES --------------------------------------- elm.json

I need a valid elm.json file, but the \"source-directories\" field has some
redundant directories:

    ./src
    src

These are two different ways of refering to the same directory:

    /p/src

Remove one of the redundant entries from your \"source-directories\" field.

"
    );
}

#[test]
fn elm_version_mismatches() {
    let version = Version::new(0, 19, 0);
    assert_eq!(
        plain(&app_elm(version, Compat::V0_19_1), Compat::V0_19_1),
        "\
-- ELM VERSION MISMATCH ----------------------------------------------- elm.json

Your elm.json says this application needs a different version of Elm.

It requires 0.19.0, but you are using 0.19.1 right now.

"
    );
    let src = b"0.19.0 <= v < 0.19.1";
    let constraint =
        Constraint::parse(src, json::Cursor::start(Compat::V0_19_3), Compat::V0_19_3).unwrap();
    assert_eq!(
        plain(&pkg_elm(&constraint, Compat::V0_19_3), Compat::V0_19_3),
        "\
-- ELM VERSION MISMATCH ----------------------------------------------- elm.json

Your elm.json says this package needs a version of Elm in this range:

    0.19.0 <= v < 0.19.1

But you are using Elm 0.19.3 right now.

"
    );
}

/// 0.19.1 starts the rows before an error on the first row at row
/// 65,535, shows none, and dies; nomi shows what later releases do.
#[test]
fn a_parse_error_on_the_first_row_shows_it_on_every_release() {
    let src = br#"{"type" "application"}"#;
    let json = |compat| to_json(&read(src, compat), compat);
    assert_eq!(json(Compat::V0_19_1), json(Compat::V0_19_2));
    assert_eq!(
        plain(&read(src, Compat::V0_19_1), Compat::V0_19_1),
        "\
-- EXPECTING COLON ---------------------------------------------------- elm.json

I ran into a problem with your elm.json file. I was partway through parsing a
JSON object when I got stuck here:

1| {\"type\" \"application\"}
           ^
I was expecting to see a colon next.

Note: Here is an example of a valid JSON object for reference:

    {
      \"name\": \"Tom\",
      \"age\": 42
    }

Notice that (1) the field names are in double quotes and (2) there is no
trailing comma after the last entry. Both are strict requirements in JSON!

"
    );
}

/// 0.19.1 dies on a string byte that can't start UTF-8; nomi reports it
/// as later releases do.
#[test]
fn a_byte_0_19_1_dies_on_is_an_unexpected_encoding() {
    let (before, after) = APPLICATION.split_once("src").unwrap();
    let src = [before.as_bytes(), b"s\xFF", after.as_bytes()].concat();
    let report = read(&src, Compat::V0_19_1);
    assert_eq!(report.title, "UNEXPECTED ENCODING");
    assert_eq!(
        to_json(&report, Compat::V0_19_1),
        to_json(&read(&src, Compat::V0_19_2), Compat::V0_19_2),
    );
}

/// Upstream defines the region of a value on one row as itself, and
/// hangs after the first paragraph; nomi underlines the value.
#[test]
fn a_value_on_one_row_is_underlined() {
    let src = APPLICATION.replace("\"src\"", "5");
    assert_eq!(
        plain(&read(src.as_bytes(), Compat::V0_19_3), Compat::V0_19_3),
        "\
-- EXPECTING STRING --------------------------------------------------- elm.json

I ran into a problem with your elm.json file. When looking at the
\"source-directories\" field, I ran into trouble with the 0th entry:

4|         5
           ^
I was expecting to run into a STRING.

"
    );
    let src = APPLICATION.replace("\"application\"", "true");
    assert_eq!(
        plain(&read(src.as_bytes(), Compat::V0_19_1), Compat::V0_19_1),
        "\
-- EXPECTING STRING --------------------------------------------------- elm.json

I ran into a problem with your elm.json file. I ran into trouble with the value
of the \"type\" field:

2|     \"type\": true,
               ^^^^
I was expecting to run into a STRING.

"
    );
}
