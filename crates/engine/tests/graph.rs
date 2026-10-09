//! The module graph against real directories. The cycles each release
//! reports, and the problems with roots, are as elm 0.19.1, 0.19.2 and
//! 0.19.3 report them for the same projects.

use std::fs;
use std::path::{Path, PathBuf};

use nomi_base::Compat;
use nomi_engine::Vfs;
use nomi_engine::graph::{
    Env, Foreigns, Graph, ImportProblem, ModuleProblem, Owner, Problem, Root, Status,
};
use nomi_project::ModuleName;
use nomi_project::Name;
use nomi_project::json::Cursor as JsonCursor;

const ALL: [Compat; 3] = [Compat::V0_19_1, Compat::V0_19_2, Compat::V0_19_3];

fn name(text: &str) -> Name {
    Name::parse(
        text.as_bytes(),
        JsonCursor::start(Compat::LATEST),
        Compat::LATEST,
    )
    .unwrap()
}

fn m(text: &str) -> ModuleName {
    ModuleName::new(text)
}

fn write(root: &Path, path: &str, text: &str) {
    let path = root.join(path);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

/// `src/<name>.elm`, importing `imports`.
fn module(root: &Path, module: &str, imports: &[&str]) {
    let mut text = format!("module {module} exposing (..)\n\n");
    for import in imports {
        text += &format!("import {import}\n");
    }
    text += "\nx = 1\n";
    write(
        root,
        &format!("src/{}.elm", module.replace('.', "/")),
        &text,
    );
}

/// An application in `src` with elm/core's modules from its dependency.
fn app(root: &Path, compat: Compat) -> Env {
    app_in(root, compat, &["src"])
}

fn app_in(root: &Path, compat: Compat, dirs: &[&str]) -> Env {
    for dir in dirs {
        fs::create_dir_all(root.join(dir)).unwrap();
    }
    let core = [
        "Basics",
        "Debug",
        "List",
        "Maybe",
        "Result",
        "String",
        "Char",
        "Tuple",
        "Platform",
        "Platform.Cmd",
        "Platform.Sub",
        "Dict",
    ];
    let foreigns = Foreigns::new([(name("elm/core"), core.map(m).to_vec())]);
    let dirs: Vec<PathBuf> = dirs.iter().map(PathBuf::from).collect();
    Env::new(compat, root, Owner::Application, &dirs, foreigns).unwrap()
}

fn from_paths(root: &Path, env: &Env, paths: &[&str]) -> Result<Graph, Problem> {
    let paths: Vec<PathBuf> = paths.iter().map(|p| root.join(p)).collect();
    Graph::from_paths(&Vfs::new().snapshot(), env, &paths)
}

fn cycle(problem: Problem) -> Vec<String> {
    let Problem::Cycle { first, rest } = problem else {
        panic!("not a cycle: {problem:?}");
    };
    std::iter::once(first)
        .chain(rest)
        .map(|n| n.to_string())
        .collect()
}

#[test]
fn a_build_crawls_what_its_roots_import() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    module(root, "Main", &["A", "Dict"]);
    module(root, "A", &["B.C"]);
    module(root, "B.C", &[]);
    write(
        root,
        "src/Unused.elm",
        "module Unused exposing (..)\n\nx = 1\n",
    );
    let graph = from_paths(root, &app(root, Compat::LATEST), &["src/Main.elm"]).unwrap();
    assert!(matches!(&graph.roots[..], [Root::Inside(n)] if *n == m("Main")));
    let Status::Local(main) = &graph.modules[&m("Main")] else {
        panic!("Main is local");
    };
    let imports: Vec<String> = main.imports.iter().map(|i| i.name.to_string()).collect();
    assert_eq!(imports[..2], ["Basics", "Debug"]);
    assert_eq!(imports[11..], ["A", "Dict"]);
    assert!(main.imports[11].range.is_some() && main.imports[0].range.is_none());
    assert!(!main.has_main);
    assert!(
        matches!(&graph.modules[&m("B.C")], Status::Local(c) if c.path.ends_with("src/B/C.elm"))
    );
    assert!(matches!(&graph.modules[&m("Dict")], Status::Foreign(p) if *p == name("elm/core")));
    assert!(!graph.modules.contains_key(&m("Unused")));
}

#[test]
fn cycles_are_reported_as_each_release_reports_them() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    module(root, "Main", &["B"]);
    module(root, "B", &["C"]);
    module(root, "C", &["Bb", "Main"]);
    module(root, "Bb", &["B"]);
    for compat in ALL {
        let problem = from_paths(root, &app(root, compat), &["src/Main.elm"]).unwrap_err();
        let want: &[&str] = match compat {
            Compat::V0_19_3 => &["B", "C", "Bb"],
            _ => &["B", "C", "Bb", "Main"],
        };
        assert_eq!(cycle(problem), want, "{compat:?}");
    }
}

#[test]
fn the_component_is_found_in_upstreams_order() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    module(root, "Main", &["Q", "D"]);
    module(root, "Q", &["Main", "D"]);
    module(root, "D", &["Q"]);
    let env = app(root, Compat::V0_19_1);
    let problem = from_paths(root, &env, &["src/Main.elm"]).unwrap_err();
    assert_eq!(cycle(problem), ["Q", "Main", "D"]);
}

#[test]
fn a_module_importing_itself_is_a_cycle() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    module(root, "Main", &["Main"]);
    // 0.19.3 crashes or hangs here; nomi reports the module alone.
    for compat in ALL {
        let problem = from_paths(root, &app(root, compat), &["src/Main.elm"]).unwrap_err();
        assert_eq!(cycle(problem), ["Main"], "{compat:?}");
    }
}

#[test]
fn imports_name_one_module_or_say_why_not() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    module(root, "Main", &["Twice", "Dict", "Missing", "Shared"]);
    write(
        root,
        "lib/Twice.elm",
        "module Twice exposing (..)\n\nx = 1\n",
    );
    write(
        root,
        "src/Twice.elm",
        "module Twice exposing (..)\n\nx = 1\n",
    );
    write(root, "src/Dict.elm", "module Dict exposing (..)\n\nx = 1\n");
    let core = name("elm/core");
    let a = name("author/a");
    let b = name("author/b");
    let env = Env::new(
        Compat::LATEST,
        root,
        Owner::Application,
        &[PathBuf::from("src"), PathBuf::from("lib")],
        Foreigns::new([
            (core.clone(), vec![m("Basics"), m("Dict")]),
            (a.clone(), vec![m("Shared")]),
            (b.clone(), vec![m("Shared")]),
        ]),
    )
    .unwrap();
    let graph = from_paths(root, &env, &["src/Main.elm"]).unwrap();
    let problem = |module: &str| match &graph.modules[&m(module)] {
        Status::BadImport(problem) => problem.clone(),
        status => panic!("{module}: {status:?}"),
    };
    assert_eq!(
        problem("Twice"),
        ImportProblem::AmbiguousLocal(vec!["src/Twice.elm".into(), "lib/Twice.elm".into()])
    );
    let ImportProblem::Ambiguous { path, dep, rivals } = problem("Dict") else {
        panic!("Dict is ambiguous");
    };
    assert!(path.ends_with("src/Dict.elm"));
    assert_eq!((dep, rivals), (core, vec![]));
    assert_eq!(problem("Missing"), ImportProblem::NotFound);
    assert_eq!(
        problem("Shared"),
        ImportProblem::AmbiguousForeign {
            dep: b,
            rivals: vec![a]
        }
    );
}

#[test]
fn modules_must_have_the_name_their_path_gives() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    module(root, "Main", &["A", "B", "C"]);
    write(root, "src/A.elm", "module Other exposing (..)\n\nx = 1\n");
    write(root, "src/B.elm", "x = 1\n");
    write(root, "src/C.elm", "module C exposing (..)\n\nx = = 1\n");
    let graph = from_paths(root, &app(root, Compat::LATEST), &["src/Main.elm"]).unwrap();
    let problem = |module: &str| match &graph.modules[&m(module)] {
        Status::BadModule(bad) => bad.problem.clone(),
        status => panic!("{module}: {status:?}"),
    };
    assert!(
        matches!(problem("A"), ModuleProblem::NameMismatch { actual, range } if actual == m("Other") && u32::from(range.start()) == 7)
    );
    assert_eq!(problem("B"), ModuleProblem::NameUnspecified);
    assert!(matches!(problem("C"), ModuleProblem::Syntax(errors) if !errors.is_empty()));
}

#[test]
fn a_broken_module_keeps_what_it_says_for_the_editor() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    module(root, "Main", &["Broken"]);
    write(
        root,
        "src/Broken.elm",
        "module Broken exposing (..)\n\nimport Gone\n\nx = = 1\n",
    );
    let graph = from_paths(root, &app(root, Compat::LATEST), &["src/Main.elm"]).unwrap();
    let Status::BadModule(bad) = &graph.modules[&m("Broken")] else {
        panic!("Broken parsed");
    };
    assert!(matches!(&bad.problem, ModuleProblem::Syntax(errors) if !errors.is_empty()));
    let parse = bad.parse.as_ref().expect("the recovered tree");
    let text = &bad.source.as_ref().unwrap().1;
    assert_eq!(parse.syntax().to_string(), **text);
    let imports: Vec<String> = bad.imports().iter().map(|i| i.name.to_string()).collect();
    assert_eq!(imports, ["Gone"]);
    // The build still stops there: `Gone` isn't looked for.
    assert!(!graph.modules.contains_key(&m("Gone")));
}

#[test]
fn kernel_code_is_for_official_packages() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    module(root, "Mine", &["Elm.Kernel.List", "Elm.Kernel.Gone"]);
    write(root, "src/Elm/Kernel/List.js", "");
    for (owner, kernel) in [(name("elm/core"), true), (name("author/core"), false)] {
        let env = Env::new(
            Compat::LATEST,
            root,
            Owner::Package(owner),
            &[PathBuf::from("src")],
            Foreigns::default(),
        )
        .unwrap();
        let graph = Graph::from_exposed(&Vfs::new().snapshot(), &env, &[m("Mine")]).unwrap();
        let status = &graph.modules[&m("Elm.Kernel.List")];
        assert_eq!(matches!(status, Status::Kernel), kernel, "{status:?}");
        assert!(matches!(
            graph.modules[&m("Elm.Kernel.Gone")],
            Status::BadImport(ImportProblem::NotFound)
        ));
        // elm/core imports nothing by default.
        let Status::Local(mine) = &graph.modules[&m("Mine")] else {
            panic!("Mine is local");
        };
        assert_eq!(mine.imports.len(), if kernel { 2 } else { 13 });
    }
}

#[test]
fn roots_are_checked_in_the_order_given() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let env = app(root, Compat::LATEST);
    module(root, "Main", &[]);
    write(root, "src/Main.js", "");
    write(
        root,
        "src/bad/Name.elm",
        "module Name exposing (..)\n\nx = 1\n",
    );
    assert_eq!(
        from_paths(root, &env, &["src/Gone.elm", "src/Main.js"]).unwrap_err(),
        Problem::PathUnknown(root.join("src/Gone.elm"))
    );
    assert_eq!(
        from_paths(root, &env, &["src/Main.js", "src/Gone.elm"]).unwrap_err(),
        Problem::WithBadExtension(root.join("src/Main.js"))
    );
    let Problem::RootNameInvalid { names, .. } =
        from_paths(root, &env, &["src/bad/Name.elm"]).unwrap_err()
    else {
        panic!("bad is no module name");
    };
    assert_eq!(names, ["bad", "Name"]);
    assert_eq!(
        from_paths(root, &env, &["src/Main.elm", "src/../src/Main.elm"]).unwrap_err(),
        Problem::MainPathDuplicate {
            first: root.join("src/../src/Main.elm"),
            second: root.join("src/Main.elm"),
        }
    );
}

#[test]
fn a_root_names_one_module() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let env = app_in(root, Compat::LATEST, &["src", "lib"]);
    write(root, "src/A.elm", "module A exposing (..)\n\nx = 1\n");
    write(root, "lib/A.elm", "module A exposing (..)\n\nx = 1\n");
    let Problem::RootNameDuplicate { name, .. } =
        from_paths(root, &env, &["src/A.elm"]).unwrap_err()
    else {
        panic!("A is in both");
    };
    assert_eq!(name, m("A"));

    write(
        root,
        "other/One.elm",
        "module Main exposing (..)\n\nmain = 1\n",
    );
    write(root, "other/Two.elm", "x = 1\n");
    assert_eq!(
        from_paths(root, &env, &["other/One.elm", "other/Two.elm"]).unwrap_err(),
        Problem::RootNameDuplicate {
            name: m("Main"),
            first: root.join("other/Two.elm"),
            second: root.join("other/One.elm"),
        }
    );
    write(
        root,
        "other/Uses.elm",
        "module C exposing (..)\n\nimport B\n\nx = 1\n",
    );
    write(
        root,
        "src/B.elm",
        "module B exposing (..)\n\nimport C\n\nx = 1\n",
    );
    write(root, "src/C.elm", "module C exposing (..)\n\nx = 1\n");
    let Problem::RootNameDuplicate { name, first, .. } =
        from_paths(root, &env, &["other/Uses.elm"]).unwrap_err()
    else {
        panic!("C is outside and in");
    };
    assert_eq!((name, first), (m("C"), root.join("other/Uses.elm")));

    let graph = from_paths(root, &env, &["other/One.elm"]).unwrap();
    let [Root::Outside(Ok(main))] = &graph.roots[..] else {
        panic!("One is outside");
    };
    assert!(main.has_main && main.name == m("Main"));
}

#[test]
fn a_graph_of_deep_modules_drops_on_a_small_stack() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let env = app(root, Compat::LATEST);
    // As deep as nomi parses: the expression's own level and one per pair.
    let depth = nomi_syntax::DEPTH_LIMIT as usize - 2;
    let x = format!("{}1{}", "(".repeat(depth), ")".repeat(depth));
    write(
        root,
        "src/Main.elm",
        &format!("module Main exposing (x)\n\n\nx =\n    {x}\n"),
    );
    let graph = from_paths(root, &env, &["src/Main.elm"]).unwrap();
    assert!(matches!(graph.modules[&m("Main")], Status::Local(_)));
    // As a caller's thread drops the graph the pool built, here as small
    // as Windows' main thread.
    std::thread::Builder::new()
        .stack_size(1 << 20)
        .spawn(move || drop(graph))
        .unwrap()
        .join()
        .unwrap();
}
