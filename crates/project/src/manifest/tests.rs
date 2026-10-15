use nomi_base::Compat;
use nomi_base::diagnostic::render::{Styles, render};

use super::*;
use crate::report;

fn plain(src: &[u8]) -> String {
    let error = Manifest::parse(src).expect_err("a bad nomi.toml");
    let report = report::manifest(&error, src, Compat::LATEST);
    render(&report.to_doc(), Compat::LATEST, 80, Styles::Plain)
}

#[test]
fn entries_with_every_key() {
    let src = r#"
[[entry]]
main = "src/Api.elm"
output = "dist/api.mjs"
platform = "node"
ffi = "src/ffi.mjs"

[[entry]]
main = "src/Web.elm"
output = "dist/web.js"
platform = "browser"
"#;
    assert_eq!(
        Manifest::parse(src.as_bytes()),
        Ok(Manifest {
            entries: vec![
                Entry {
                    main: "src/Api.elm".to_owned(),
                    output: Some("dist/api.mjs".to_owned()),
                    platform: Some(Platform::Node),
                    ffi: Some("src/ffi.mjs".to_owned()),
                },
                Entry {
                    main: "src/Web.elm".to_owned(),
                    output: Some("dist/web.js".to_owned()),
                    platform: Some(Platform::Browser),
                    ffi: None,
                },
            ],
        })
    );
}

#[test]
fn an_entry_with_main_alone() {
    let entry = Entry {
        main: "src/Main.elm".to_owned(),
        output: None,
        platform: None,
        ffi: None,
    };
    for src in [
        "[[entry]]\nmain = \"src/Main.elm\"\n",
        "entry = [{ main = \"src/Main.elm\" }]\n",
    ] {
        assert_eq!(
            Manifest::parse(src.as_bytes()),
            Ok(Manifest {
                entries: vec![entry.clone()],
            })
        );
    }
    assert_eq!(Manifest::parse(b""), Ok(Manifest::default()));
}

/// A bad file, by what is wrong with it, and its report.
const BAD: &[(&str, &str, &str)] = &[
    (
        "an unknown key in the file",
        r#"[[entry]]
main = "src/Main.elm"

[workspace]
members = []
"#,
        r#"-- UNKNOWN KEY ------------------------------------------------------- nomi.toml

I ran into a problem with your nomi.toml file. I do not know the `workspace` key
here:

4| [workspace]
    ^^^^^^^^^
I know `entry` here.

"#,
    ),
    (
        "an unknown key in an entry",
        r#"[[entry]]
main = "src/Main.elm"
outptu = "dist/main.mjs"
"#,
        r#"-- UNKNOWN KEY ------------------------------------------------------- nomi.toml

I ran into a problem with your nomi.toml file. I do not know the `outptu` key
here:

3| outptu = "dist/main.mjs"
   ^^^^^^
I know `main`, `output`, `platform` and `ffi` here.

"#,
    ),
    (
        "a wrong platform",
        r#"[[entry]]
main = "src/Main.elm"
platform = "deno"
"#,
        r#"-- UNKNOWN PLATFORM -------------------------------------------------- nomi.toml

I ran into a problem with your nomi.toml file. I do not know the "deno"
platform:

3| platform = "deno"
              ^^^^^^
The platforms are "node" and "browser". Without one, an entry runs on Node when
its `main` is a Nomi.Program, and in the browser otherwise.

"#,
    ),
    (
        "a missing main",
        r#"[[entry]]
main = "src/Api.elm"

[[entry]]
output = "dist/web.js"
"#,
        r#"-- MISSING MAIN ------------------------------------------------------ nomi.toml

I ran into a problem with your nomi.toml file. This entry has no `main`:

4| [[entry]]
   ^^^^^^^^^
Every entry names the Elm module it starts from, as in main = "src/Main.elm".

"#,
    ),
    (
        "a path that isn't a string",
        r#"[[entry]]
main = 3
"#,
        r#"-- WRONG TYPE -------------------------------------------------------- nomi.toml

I ran into a problem with your nomi.toml file. The `main` key needs a string,
but this is an integer:

2| main = 3
          ^
For example: main = "src/Main.elm"

"#,
    ),
    (
        "a platform that isn't a string",
        r#"[[entry]]
main = "src/Main.elm"
platform = true
"#,
        r#"-- WRONG TYPE -------------------------------------------------------- nomi.toml

I ran into a problem with your nomi.toml file. The `platform` key needs a
string, but this is a boolean:

3| platform = true
              ^^^^
For example: platform = "node"

"#,
    ),
    (
        "entries that aren't an array",
        r#"entry = "src/Main.elm"
"#,
        r#"-- WRONG TYPE -------------------------------------------------------- nomi.toml

I ran into a problem with your nomi.toml file. The `entry` key needs an array of
tables, but this is a string:

1| entry = "src/Main.elm"
           ^^^^^^^^^^^^^^
Each entry is a table of its own, under an [[entry]] header.

"#,
    ),
    (
        "entries written as one table",
        r#"[entry]
main = "src/Main.elm"
"#,
        r#"-- WRONG TYPE -------------------------------------------------------- nomi.toml

I ran into a problem with your nomi.toml file. The `entry` key needs an array of
tables, but this is a table:

1| [entry]
   ^^^^^^^
Each entry is a table of its own, under an [[entry]] header.

"#,
    ),
    (
        "an entry that isn't a table",
        r#"entry = [{ main = "src/Main.elm" }, 1]
"#,
        r#"-- WRONG TYPE -------------------------------------------------------- nomi.toml

I ran into a problem with your nomi.toml file. The `entry` key needs a table,
but this is an integer:

1| entry = [{ main = "src/Main.elm" }, 1]
                                       ^
Each entry is a table of its own, under an [[entry]] header.

"#,
    ),
    (
        "TOML that doesn't parse",
        r#"[[entry]
main = "src/Main.elm"
"#,
        r#"-- BAD TOML ---------------------------------------------------------- nomi.toml

I ran into a problem with your nomi.toml file. It is not valid TOML here:

1| [[entry]
           ^
The TOML reader says: unclosed array table, expected `]`.

"#,
    ),
    (
        "a key given twice",
        r#"[[entry]]
main = "src/Main.elm"
main = "src/Other.elm"
"#,
        r#"-- BAD TOML ---------------------------------------------------------- nomi.toml

I ran into a problem with your nomi.toml file. It is not valid TOML here:

3| main = "src/Other.elm"
   ^^^^
The TOML reader says: duplicate key.

"#,
    ),
];

#[test]
fn bad_files_are_reported_at_their_span() {
    let failures: Vec<String> = (BAD.iter())
        .filter_map(|&(name, src, expected)| {
            let found = plain(src.as_bytes());
            (found != expected).then(|| format!("{name}:\n{found}"))
        })
        .collect();
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn bytes_that_are_not_utf8_are_reported_where_they_start() {
    assert_eq!(
        plain(b"[[entry]]\nmain = \"src/\xffMain.elm\"\n"),
        "\
-- BAD TOML ---------------------------------------------------------- nomi.toml

I ran into a problem with your nomi.toml file. It is not valid TOML here:

2| main = \"src/\u{FFFD}Main.elm\"
               ^
The TOML reader says: invalid UTF-8.

"
    );
}
