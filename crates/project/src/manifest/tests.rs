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
                    args: Vec::new(),
                    static_dir: None,
                    port: None,
                },
                Entry {
                    main: "src/Web.elm".to_owned(),
                    output: Some("dist/web.js".to_owned()),
                    platform: Some(Platform::Browser),
                    ffi: None,
                    args: Vec::new(),
                    static_dir: None,
                    port: None,
                },
            ],
            workspace: None,
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
        args: Vec::new(),
        static_dir: None,
        port: None,
    };
    for src in [
        "[[entry]]\nmain = \"src/Main.elm\"\n",
        "entry = [{ main = \"src/Main.elm\" }]\n",
    ] {
        assert_eq!(
            Manifest::parse(src.as_bytes()),
            Ok(Manifest {
                workspace: None,
                entries: vec![entry.clone()],
            })
        );
    }
    assert_eq!(Manifest::parse(b""), Ok(Manifest::default()));
}

#[test]
fn a_workspace_with_entries_and_every_key() {
    let src = r#"
[workspace]
members = ["apps/web", "./apps/api", "packages/*"]

[[entry]]
main = "src/Api.elm"
platform = "node"
args = ["--port", "3000"]

[[entry]]
main = "src/Web.elm"
platform = "browser"
static = "public"
port = 8080
"#;
    let manifest = Manifest::parse(src.as_bytes()).expect("a good nomi.toml");
    let members: Vec<(String, &str)> = (manifest.workspace.as_ref())
        .expect("a workspace")
        .members
        .iter()
        .map(|member| (member.pattern.to_string(), &src[member.range]))
        .collect();
    assert_eq!(
        members,
        [
            ("apps/web".to_owned(), "\"apps/web\""),
            ("./apps/api".to_owned(), "\"./apps/api\""),
            ("packages/*".to_owned(), "\"packages/*\""),
        ]
    );
    assert_eq!(
        manifest.entries,
        [
            Entry {
                main: "src/Api.elm".to_owned(),
                output: None,
                platform: Some(Platform::Node),
                ffi: None,
                args: vec!["--port".to_owned(), "3000".to_owned()],
                static_dir: None,
                port: None,
            },
            Entry {
                main: "src/Web.elm".to_owned(),
                output: None,
                platform: Some(Platform::Browser),
                ffi: None,
                args: Vec::new(),
                static_dir: Some("public".to_owned()),
                port: Some(8080),
            },
        ]
    );
}

#[test]
fn platform_keys_wait_for_the_platform() {
    let src =
        "[[entry]]\nmain = \"src/Main.elm\"\nargs = [\"-v\"]\nstatic = \"public\"\nport = 1\n";
    let manifest = Manifest::parse(src.as_bytes()).expect("a good nomi.toml");
    assert_eq!(manifest.entries[0].platform, None);
    assert_eq!(manifest.entries[0].port, Some(1));
}

#[test]
fn a_workspace_alone() {
    for (src, members) in [
        ("[workspace]\n", 0),
        ("workspace.members = [\"a\", \"b/*\"]\n", 2),
    ] {
        let manifest = Manifest::parse(src.as_bytes()).expect("a good nomi.toml");
        assert_eq!(manifest.entries, []);
        assert_eq!(manifest.workspace.map(|w| w.members.len()), Some(members));
    }
}

/// A bad file, by what is wrong with it, and its report.
const BAD: &[(&str, &str, &str)] = &[
    (
        "an unknown key in the file",
        r#"[[entry]]
main = "src/Main.elm"

[serve]
port = 8000
"#,
        r#"-- UNKNOWN KEY ------------------------------------------------------- nomi.toml

I ran into a problem with your nomi.toml file. I do not know the `serve` key
here:

4| [serve]
    ^^^^^
I know `workspace` and `entry` here.

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
I know `main`, `output`, `platform`, `ffi`, `args`, `static` and `port` here.

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
    (
        "members that aren't an array",
        r#"[workspace]
members = "apps/*"
"#,
        r#"-- WRONG TYPE -------------------------------------------------------- nomi.toml

I ran into a problem with your nomi.toml file. The `members` key needs an array
of strings, but this is a string:

2| members = "apps/*"
             ^^^^^^^^
For example: members = ["apps/web", "packages/*"]

"#,
    ),
    (
        "a member that isn't a string",
        r#"[workspace]
members = ["apps/web", 3]
"#,
        r#"-- WRONG TYPE -------------------------------------------------------- nomi.toml

I ran into a problem with your nomi.toml file. The `members` key needs a string,
but this is an integer:

2| members = ["apps/web", 3]
                          ^
For example: members = ["apps/web", "packages/*"]

"#,
    ),
    (
        "an empty member",
        r#"[workspace]
members = ["apps/web", ""]
"#,
        r#"-- BAD MEMBER -------------------------------------------------------- nomi.toml

I ran into a problem with your nomi.toml file. I cannot use "" as a workspace
member:

2| members = ["apps/web", ""]
                          ^^
A member is a directory under this one, as in "apps/web", or a `*` standing for
any one directory, as in "packages/*". It is not empty or absolute, has no `..`,
and a `*` is a whole part of the path, between slashes.

"#,
    ),
    (
        "a member outside the workspace",
        r#"[workspace]
members = ["../x"]
"#,
        r#"-- BAD MEMBER -------------------------------------------------------- nomi.toml

I ran into a problem with your nomi.toml file. I cannot use "../x" as a
workspace member:

2| members = ["../x"]
              ^^^^^^
A member is a directory under this one, as in "apps/web", or a `*` standing for
any one directory, as in "packages/*". It is not empty or absolute, has no `..`,
and a `*` is a whole part of the path, between slashes.

"#,
    ),
    (
        "an absolute member",
        r#"[workspace]
members = ["/abs"]
"#,
        r#"-- BAD MEMBER -------------------------------------------------------- nomi.toml

I ran into a problem with your nomi.toml file. I cannot use "/abs" as a
workspace member:

2| members = ["/abs"]
              ^^^^^^
A member is a directory under this one, as in "apps/web", or a `*` standing for
any one directory, as in "packages/*". It is not empty or absolute, has no `..`,
and a `*` is a whole part of the path, between slashes.

"#,
    ),
    (
        "a star in part of a member",
        r#"[workspace]
members = ["packages/a*b"]
"#,
        r#"-- BAD MEMBER -------------------------------------------------------- nomi.toml

I ran into a problem with your nomi.toml file. I cannot use "packages/a*b" as a
workspace member:

2| members = ["packages/a*b"]
              ^^^^^^^^^^^^^^
A member is a directory under this one, as in "apps/web", or a `*` standing for
any one directory, as in "packages/*". It is not empty or absolute, has no `..`,
and a `*` is a whole part of the path, between slashes.

"#,
    ),
    (
        "an unknown key in the workspace",
        r#"[workspace]
members = ["apps/*"]
exclude = ["apps/old"]
"#,
        r#"-- UNKNOWN KEY ------------------------------------------------------- nomi.toml

I ran into a problem with your nomi.toml file. I do not know the `exclude` key
here:

3| exclude = ["apps/old"]
   ^^^^^^^
I know `members` here.

"#,
    ),
    (
        "a workspace that isn't a table",
        r#"workspace = ["apps/*"]
"#,
        r#"-- WRONG TYPE -------------------------------------------------------- nomi.toml

I ran into a problem with your nomi.toml file. The `workspace` key needs a
table, but this is an array:

1| workspace = ["apps/*"]
               ^^^^^^^^^^
The workspace is one table, under a [workspace] header.

"#,
    ),
    (
        "args that aren't an array",
        r#"[[entry]]
main = "src/Main.elm"
args = "--verbose"
"#,
        r#"-- WRONG TYPE -------------------------------------------------------- nomi.toml

I ran into a problem with your nomi.toml file. The `args` key needs an array of
strings, but this is a string:

3| args = "--verbose"
          ^^^^^^^^^^^
For example: args = ["--port", "3000"]

"#,
    ),
    (
        "args that aren't strings",
        r#"[[entry]]
main = "src/Main.elm"
args = ["--port", 3000]
"#,
        r#"-- WRONG TYPE -------------------------------------------------------- nomi.toml

I ran into a problem with your nomi.toml file. The `args` key needs a string,
but this is an integer:

3| args = ["--port", 3000]
                     ^^^^
For example: args = ["--port", "3000"]

"#,
    ),
    (
        "port 0",
        r#"[[entry]]
main = "src/Main.elm"
port = 0
"#,
        r#"-- BAD PORT ---------------------------------------------------------- nomi.toml

I ran into a problem with your nomi.toml file. This port is out of range:

3| port = 0
          ^
A port is a number from 1 to 65535, as in port = 8000.

"#,
    ),
    (
        "port 70000",
        r#"[[entry]]
main = "src/Main.elm"
port = 70000
"#,
        r#"-- BAD PORT ---------------------------------------------------------- nomi.toml

I ran into a problem with your nomi.toml file. This port is out of range:

3| port = 70000
          ^^^^^
A port is a number from 1 to 65535, as in port = 8000.

"#,
    ),
    (
        "a port that isn't an integer",
        r#"[[entry]]
main = "src/Main.elm"
port = "8000"
"#,
        r#"-- WRONG TYPE -------------------------------------------------------- nomi.toml

I ran into a problem with your nomi.toml file. The `port` key needs an integer,
but this is a string:

3| port = "8000"
          ^^^^^^
For example: port = 8000

"#,
    ),
    (
        "static on a node entry",
        r#"[[entry]]
main = "src/Api.elm"
static = "public"
platform = "node"
"#,
        r#"-- WRONG PLATFORM ---------------------------------------------------- nomi.toml

I ran into a problem with your nomi.toml file. The `static` key is for browser
entries, but this entry runs on Node:

3| static = "public"
            ^^^^^^^^
The `static` directory and the `port` are for the server `nomi run` starts for a
browser program. A Node program serves itself.

"#,
    ),
    (
        "port on a node entry",
        r#"[[entry]]
main = "src/Api.elm"
platform = "node"
port = 8000
"#,
        r#"-- WRONG PLATFORM ---------------------------------------------------- nomi.toml

I ran into a problem with your nomi.toml file. The `port` key is for browser
entries, but this entry runs on Node:

4| port = 8000
          ^^^^
The `static` directory and the `port` are for the server `nomi run` starts for a
browser program. A Node program serves itself.

"#,
    ),
    (
        "args on a browser entry",
        r#"[[entry]]
main = "src/Web.elm"
platform = "browser"
args = ["-v"]
"#,
        r#"-- WRONG PLATFORM ---------------------------------------------------- nomi.toml

I ran into a problem with your nomi.toml file. The `args` key is for Node
entries, but this entry runs in the browser:

4| args = ["-v"]
          ^^^^^^
The `args` are the command-line arguments `nomi run` gives a Node program, and
the `ffi` module is the JavaScript its `Ffi.task`s call on Node. A browser
program has neither.

"#,
    ),
    (
        "ffi on a browser entry",
        r#"[[entry]]
main = "src/Web.elm"
ffi = "src/ffi.mjs"
platform = "browser"
"#,
        r#"-- WRONG PLATFORM ---------------------------------------------------- nomi.toml

I ran into a problem with your nomi.toml file. The `ffi` key is for Node
entries, but this entry runs in the browser:

3| ffi = "src/ffi.mjs"
         ^^^^^^^^^^^^^
The `args` are the command-line arguments `nomi run` gives a Node program, and
the `ffi` module is the JavaScript its `Ffi.task`s call on Node. A browser
program has neither.

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
