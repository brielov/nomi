//! Finding a workspace on disk: its members from the root's patterns,
//! each read, and the problems that keep members from making one.

use nomi_base::Compat;
use nomi_base::diagnostic::render::{Styles, render};
use nomi_engine::Vfs;
use nomi_engine::workspace::discover;
use nomi_testkit::workspace::{app, package};
use nomi_testkit::write_file;

/// What discovery finds: the root and the members' directories in order,
/// none, or a report's text.
#[derive(Debug, PartialEq, Eq)]
enum Found {
    Members(&'static str, Vec<&'static str>),
    Standalone,
    Problem(&'static str),
}

struct Case {
    name: &'static str,
    files: Vec<(&'static str, String)>,
    /// Where discovery starts, relative to the temporary directory.
    from: &'static str,
    found: Found,
}

fn text(text: &str) -> String {
    text.to_owned()
}

fn cases() -> Vec<Case> {
    let workspace = |members: &str| text(&format!("[workspace]\nmembers = {members}\n"));
    vec![
        Case {
            name: "globs expand to directories with an elm.json",
            files: vec![
                (
                    "nomi.toml",
                    workspace(r#"["apps/*", "packages/*", "tools/*"]"#),
                ),
                ("apps/web/elm.json", app(&["acme/shared"], &[])),
                ("apps/api/elm.json", app(&["acme/shared"], &[])),
                ("apps/notes/README.md", text("not a project")),
                ("apps/.hidden/elm.json", app(&[], &[])),
                ("packages/shared/elm.json", package("acme/shared", &[], &[])),
            ],
            from: "",
            found: Found::Members("", vec!["packages/shared", "apps/api", "apps/web"]),
        },
        Case {
            name: "members come after what they depend on, ties by path",
            files: vec![
                (
                    "nomi.toml",
                    workspace(r#"["apps/web", "apps/api", "packages/*"]"#),
                ),
                ("apps/web/elm.json", app(&["acme/ui"], &[])),
                ("apps/api/elm.json", app(&["acme/core"], &[])),
                ("packages/core/elm.json", package("acme/core", &[], &[])),
                (
                    "packages/ui/elm.json",
                    package("acme/ui", &["acme/core"], &[]),
                ),
            ],
            from: "apps/web/src",
            found: Found::Members(
                "",
                vec!["packages/core", "apps/api", "packages/ui", "apps/web"],
            ),
        },
        Case {
            name: "the root with an elm.json is a member",
            files: vec![
                (
                    "nomi.toml",
                    text(
                        "[workspace]\nmembers = [\"packages/*\"]\n\n[[entry]]\nmain = \"src/Main.elm\"\n",
                    ),
                ),
                ("elm.json", app(&["acme/shared"], &[])),
                ("packages/shared/elm.json", package("acme/shared", &[], &[])),
            ],
            from: "src",
            found: Found::Members("", vec!["packages/shared", ""]),
        },
        Case {
            name: "a project under the root that no member names stands alone",
            files: vec![
                ("nomi.toml", workspace(r#"["apps/*"]"#)),
                ("apps/web/elm.json", app(&[], &[])),
                ("examples/demo/elm.json", app(&[], &[])),
            ],
            from: "examples/demo/src",
            found: Found::Standalone,
        },
        Case {
            name: "a directory in the workspace outside any project is in it",
            files: vec![
                ("nomi.toml", workspace(r#"["apps/*"]"#)),
                ("apps/web/elm.json", app(&[], &[])),
                ("examples/README.md", text("examples")),
            ],
            from: "examples",
            found: Found::Members("", vec!["apps/web"]),
        },
        Case {
            name: "the nearest workspace that has the project wins",
            files: vec![
                ("nomi.toml", workspace(r#"["inner/apps/*"]"#)),
                ("inner/nomi.toml", workspace(r#"["libs/*"]"#)),
                ("inner/apps/web/elm.json", app(&[], &[])),
                ("inner/libs/a/elm.json", package("acme/a", &[], &[])),
            ],
            from: "inner/libs/a",
            found: Found::Members("inner", vec!["libs/a"]),
        },
        Case {
            name: "a workspace without the project gives way to one above with it",
            files: vec![
                ("nomi.toml", workspace(r#"["inner/apps/*"]"#)),
                ("inner/nomi.toml", workspace(r#"["libs/*"]"#)),
                ("inner/apps/web/elm.json", app(&[], &[])),
                ("inner/libs/a/elm.json", package("acme/a", &[], &[])),
            ],
            from: "inner/apps/web",
            found: Found::Members("", vec!["inner/apps/web"]),
        },
        Case {
            name: "a literal member needs an elm.json",
            files: vec![
                ("nomi.toml", workspace(r#"["apps/web", "apps/api"]"#)),
                ("apps/web/elm.json", app(&[], &[])),
                ("apps/api/README.md", text("soon")),
            ],
            from: "",
            found: Found::Problem(
                "-- MISSING MEMBER ---------------------------------------------------- nomi.toml\n\
                 \n\
                 I ran into a problem with your nomi.toml file. There is no elm.json in the\n\
                 member apps/api:\n\
                 \n\
                 2| members = [\"apps/web\", \"apps/api\"]\n\
                 \x20                         ^^^^^^^^^^\n\
                 A member is an application or a package with an elm.json of its own.\n",
            ),
        },
        Case {
            name: "a broken root is named from the member it was found from",
            files: vec![
                ("nomi.toml", workspace(r#""apps/web""#)),
                ("apps/web/elm.json", app(&[], &[])),
            ],
            from: "apps/web",
            found: Found::Problem(
                "-- WRONG TYPE -------------------------------------------------- ../../nomi.toml\n",
            ),
        },
        Case {
            name: "a member can't be a workspace",
            files: vec![
                ("nomi.toml", workspace(r#"["apps/*"]"#)),
                ("apps/web/elm.json", app(&[], &[])),
                ("apps/web/nomi.toml", workspace(r#"["lib"]"#)),
            ],
            from: "",
            found: Found::Problem(
                "-- NESTED WORKSPACE ------------------------------------------------------------\n\
                 \n\
                 apps/web/nomi.toml has a [workspace] of its own, but apps/web is a member of the\n\
                 workspace in nomi.toml. A workspace can't hold another, so remove one of the two\n\
                 [workspace] tables.\n\
                 \n",
            ),
        },
        Case {
            name: "two members can't have one name",
            files: vec![
                ("nomi.toml", workspace(r#"["packages/*"]"#)),
                ("packages/a/elm.json", package("acme/shared", &[], &[])),
                ("packages/b/elm.json", package("acme/shared", &[], &[])),
            ],
            from: "",
            found: Found::Problem(
                "-- DUPLICATE MEMBER ------------------------------------------------------------\n\
                 \n\
                 The members packages/a and packages/b are both the package acme/shared, but each\n\
                 member package needs a name of its own, since that is how the others depend on\n\
                 it.\n\
                 \n",
            ),
        },
        Case {
            name: "member packages can't depend on each other in a cycle",
            files: vec![
                ("nomi.toml", workspace(r#"["packages/*", "apps/web"]"#)),
                ("apps/web/elm.json", app(&["acme/b"], &[])),
                ("packages/a/elm.json", package("acme/a", &["acme/c"], &[])),
                ("packages/b/elm.json", package("acme/b", &["acme/a"], &[])),
                ("packages/c/elm.json", package("acme/c", &["acme/b"], &[])),
            ],
            from: "",
            found: Found::Problem(
                "-- MEMBER CYCLE ----------------------------------------------------------------\n\
                 \n\
                 The member packages depend on each other in a cycle: acme/a, which depends on\n\
                 acme/c, which depends on acme/b, which depends on acme/a. A package can't depend\n\
                 on itself, even through others.\n\
                 \n",
            ),
        },
        Case {
            name: "a test dependency makes no cycle",
            files: vec![
                ("nomi.toml", workspace(r#"["packages/*"]"#)),
                ("packages/a/elm.json", package("acme/a", &[], &["acme/b"])),
                ("packages/b/elm.json", package("acme/b", &["acme/a"], &[])),
            ],
            from: "",
            found: Found::Members("", vec!["packages/a", "packages/b"]),
        },
        Case {
            name: "a member can't take a kernel author's name",
            files: vec![
                ("nomi.toml", workspace(r#"["packages/*"]"#)),
                (
                    "packages/core/elm.json",
                    package("elm-explorations/core", &[], &[]),
                ),
            ],
            from: "",
            found: Found::Problem(
                "-- RESERVED NAME ---------------------------------------------------------------\n\
                 \n\
                 The member packages/core is the package elm-explorations/core, but packages by\n\
                 elm, elm-explorations and nomi may hold kernel code, so they come only from the\n\
                 package registry and nomi. Give it a name of your own in its elm.json.\n\
                 \n",
            ),
        },
        Case {
            name: "entries at the root need its elm.json",
            files: vec![
                (
                    "nomi.toml",
                    text(
                        "[workspace]\nmembers = [\"apps/*\"]\n\n[[entry]]\nmain = \"src/Main.elm\"\n",
                    ),
                ),
                ("apps/web/elm.json", app(&[], &[])),
            ],
            from: "apps/web",
            found: Found::Problem("ENTRY WITHOUT PROJECT"),
        },
        Case {
            name: "a member's bad nomi.toml is named",
            files: vec![
                ("nomi.toml", workspace(r#"["apps/*"]"#)),
                ("apps/web/elm.json", app(&[], &[])),
                (
                    "apps/web/nomi.toml",
                    text("[[entry]]\nmian = \"src/Main.elm\"\n"),
                ),
            ],
            from: "",
            found: Found::Problem(
                "-- UNKNOWN KEY ---------------------------------------------- apps/web/nomi.toml",
            ),
        },
        Case {
            name: "a member's bad elm.json is named",
            files: vec![
                ("nomi.toml", workspace(r#"["apps/*"]"#)),
                ("apps/web/elm.json", text("{")),
            ],
            from: "",
            found: Found::Problem(
                "-- EXTRA COMMA ----------------------------------------------- apps/web/elm.json",
            ),
        },
    ]
}

#[test]
fn workspaces_on_disk() {
    for case in cases() {
        let temp = tempfile::tempdir().unwrap();
        let top = temp.path().canonicalize().unwrap();
        for (path, text) in &case.files {
            write_file(&top, path, text);
        }
        let from = top.join(case.from);
        std::fs::create_dir_all(&from).unwrap();
        let name = case.name;
        match (discover(&Vfs::new().snapshot(), &from), case.found) {
            (None, Found::Standalone) => {}
            (Some(Ok(workspace)), Found::Members(root, expected)) => {
                assert_eq!(workspace.root(), top.join(root), "{name}");
                let dirs: Vec<String> = (workspace.members().iter())
                    .map(|m| m.dir.to_string_lossy().into_owned())
                    .collect();
                assert_eq!(dirs, expected, "{name}");
            }
            (Some(Err(report)), Found::Problem(expected)) => {
                let text = render(&report.to_doc(), Compat::LATEST, 80, Styles::Plain);
                assert!(text.contains(expected), "{name}:\n{text}");
            }
            (found, expected) => panic!("{name}: found {found:?}, expected {expected:?}"),
        }
    }
}
