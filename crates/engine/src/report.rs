//! Upstream's reports on the modules a build reads: an import that names
//! no one module (`Reporting/Error/Import.hs`), a module whose header
//! disagrees with its path (`Reporting/Error/Syntax.hs`), and what keeps
//! the project from building at all (`Reporting/Exit.hs`,
//! `toProjectProblemReport`). Which modules `elm make` shows, and in what
//! order, is the reporting policy's to decide.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use nomi_base::Compat;
use nomi_base::diagnostic::code::{Position, Region, Source};
use nomi_base::diagnostic::doc::{
    Color, Doc, fill_sep, hint, reflow, reflow_link, simple_hint, simple_note, stack, text, vcat,
};
use nomi_base::diagnostic::json::{self, Json};
use nomi_base::diagnostic::{Problem, Report};
use nomi_project::ModuleName;
use nomi_syntax::elm;

use crate::graph::{
    BadModule, Env, Graph, ImportProblem, Module, ModuleProblem, Problem as GraphProblem, Root,
    Status,
};

/// One module's problems, as `--report=json` lists them.
#[derive(Clone, Debug)]
pub struct ModuleErrors {
    pub path: PathBuf,
    /// Its name as the build knows it, or `???` for a file outside the
    /// source directories that doesn't parse.
    pub name: String,
    pub problems: Vec<Problem>,
}

impl ModuleErrors {
    /// `Error.toJson`'s module entry.
    pub fn to_json(&self, compat: Compat) -> Json {
        json::module(
            &self.path.to_string_lossy(),
            &self.name,
            self.problems.iter().map(|p| p.to_json(compat)).collect(),
        )
    }
}

/// Every module of `graph` with problems of its own: a header or syntax
/// that is wrong, or imports that name no one module (`checkModule`'s
/// `RProblem`s). A module that only imports a broken one is blocked, and
/// not here. In no particular order.
pub fn module_errors(env: &Env, graph: &Graph) -> Vec<ModuleErrors> {
    let mut errors = Vec::new();
    for (name, status) in &graph.modules {
        match status {
            Status::Local(module) => errors.extend(imports(env, graph, module)),
            Status::BadModule(bad) => errors.extend(bad_module(env, bad, &name.to_string())),
            _ => {}
        }
    }
    for root in &graph.roots {
        match root {
            Root::Outside(Ok(module)) => errors.extend(imports(env, graph, module)),
            Root::Outside(Err(bad)) => errors.extend(bad_module(env, bad, "???")),
            Root::Inside(_) => {}
        }
    }
    errors
}

fn imports(env: &Env, graph: &Graph, module: &Module) -> Option<ModuleErrors> {
    let problems = import_problems(env, graph, module);
    (!problems.is_empty()).then(|| ModuleErrors {
        path: module.path.clone(),
        name: module.name.to_string(),
        problems,
    })
}

fn bad_module(env: &Env, bad: &BadModule, name: &str) -> Option<ModuleErrors> {
    let problem = module_problem(env, bad, &ModuleName::new(name))?;
    Some(ModuleErrors {
        path: bad.path.clone(),
        name: name.to_owned(),
        problems: vec![problem],
    })
}

/// The report on a module that can't be read as the one its path names:
/// elm's syntax error, or `MODULE NAME MISSING` or `MODULE NAME MISMATCH`
/// against `expected`. `None` for a file that can't be read, which
/// upstream doesn't report.
pub fn module_problem(env: &Env, bad: &BadModule, expected: &ModuleName) -> Option<Problem> {
    let compat = env.compat();
    let (_, contents) = bad.source.as_ref()?;
    let source = Source::new(contents);
    match &bad.problem {
        ModuleProblem::Unreadable(_) => None,
        ModuleProblem::Syntax(_) => {
            let error = elm::parse(contents.as_bytes(), env.source(), compat).err()?;
            Some(elm::to_problem(&source, &error, compat))
        }
        ModuleProblem::NameUnspecified => Some(name_unspecified(expected, compat)),
        ModuleProblem::NameMismatch { actual, range } => {
            let region = source.region(*range, compat);
            let message = source.snippet(
                compat,
                text("It looks like this module name is out of sync:"),
                region,
                None,
                stack([
                    reflow(&format!(
                        "I need it to match the file path, so I was expecting to see `{expected}` \
                         here. Make the following change, and you should be all set!"
                    )),
                    text(actual.to_string())
                        .color(Color::DullYellow)
                        .append(text(" -> "))
                        .append(text(expected.to_string()).color(Color::Green))
                        .indent(4),
                    simple_note(
                        "I require that module names correspond to file paths. This makes it \
                         much easier to explore unfamiliar codebases! So if you want to keep the \
                         current module name, try renaming the file instead.",
                    ),
                ]),
            );
            Some(Problem::new("MODULE NAME MISMATCH", region, message))
        }
    }
}

fn name_unspecified(name: &ModuleName, compat: Compat) -> Problem {
    // `toRegion 1 1` in 0.19.1, `A.zero` after.
    let start = Position {
        row: compat.first_position(),
        col: compat.first_position(),
    };
    let message = stack([
        reflow("I need the module name to be declared at the top of this file, like this:"),
        fill_sep([
            text("module").color(Color::Cyan),
            text(name.to_string()),
            text("exposing").color(Color::Cyan),
            text("(..)"),
        ])
        .indent(4),
        reflow("Try adding that as the first line of your file!"),
        simple_note(
            "It is best to replace (..) with an explicit list of types and functions you want \
             to expose. When you know a value is only used within this module, you can \
             refactor without worrying about uses elsewhere. Limiting exposed values can also \
             speed up compilation because I can skip a bunch of work if I see that the exposed \
             API has not changed.",
        ),
    ]);
    Problem::new("MODULE NAME MISSING", Region { start, end: start }, message)
}

/// `checkDeps`'s import problems and `toImportErrors`: one report for each
/// of `module`'s imports, default ones included, that names no one
/// module, in the order it imports them.
pub fn import_problems(env: &Env, graph: &Graph, module: &Module) -> Vec<Problem> {
    let compat = env.compat();
    // `Map.fromList`: an import written twice is where it was written last.
    let ranges: HashMap<&ModuleName, _> = (module.imports.iter())
        .filter_map(|import| Some((&import.name, import.range?)))
        .collect();
    let mut unimported: Vec<&ModuleName> = (env.foreigns().names())
        .chain(graph.modules.keys())
        .filter(|name| !module.imports.iter().any(|i| &i.name == *name))
        .collect();
    unimported.sort_by(|a, b| a.compare(b, compat));
    unimported.dedup();
    let source = Source::new(&module.text);
    let mut problems = Vec::new();
    for import in &module.imports {
        let Some(Status::BadImport(problem)) = graph.modules.get(&import.name) else {
            continue;
        };
        // Upstream has no region for a default import, and crashes.
        let Some(range) = ranges.get(&import.name) else {
            continue;
        };
        let region = source.region(*range, compat);
        problems.push(import_problem(
            &source,
            compat,
            region,
            &import.name,
            &unimported,
            problem,
        ));
    }
    problems
}

/// `Import.toReport`.
pub fn import_problem(
    source: &Source<'_>,
    compat: Compat,
    region: Region,
    name: &ModuleName,
    unimported: &[&ModuleName],
    problem: &ImportProblem,
) -> Problem {
    let before = reflow(&format!("You are trying to import a `{name}` module:"));
    let (title, after) = match problem {
        ImportProblem::NotFound => {
            let mut suggestions = unimported.to_vec();
            let codes = |name: &ModuleName| -> Vec<u32> {
                name.to_string().chars().map(u32::from).collect()
            };
            nomi_base::suggest::sort(&codes(name), &mut suggestions, |n| codes(n));
            suggestions.truncate(4);
            let hint = match package_for(name) {
                None => simple_hint(
                    "If it is not a typo, check the \"dependencies\" and \"source-directories\" \
                     of your elm.json to make sure all the packages you need are listed there!",
                ),
                Some(package) => hint(
                    ["Maybe", "you", "want", "the"]
                        .map(text)
                        .into_iter()
                        .chain([text(format!("`{name}`"))])
                        .chain(["module", "defined", "in", "the"].map(text))
                        .chain([text(package)])
                        .chain(["package?", "Running"].map(text))
                        .chain([text(format!("elm install {package}")).color(Color::Green)])
                        .chain(["should", "make", "it", "available!"].map(text)),
                ),
            };
            let after = stack([
                reflow(
                    "I checked the \"dependencies\" and \"source-directories\" listed in your \
                     elm.json, but I cannot find it! Maybe it is a typo for one of these names?",
                ),
                vcat(suggestions.iter().map(|n| text(n.to_string())))
                    .indent(4)
                    .color(Color::DullYellow),
                hint,
            ]);
            ("MODULE NOT FOUND", after)
        }
        ImportProblem::Ambiguous { path, dep, .. } => {
            let words = |s: &str| s.split(' ').map(text).collect::<Vec<_>>();
            let after = stack([
                fill_sep(
                    words("But I found multiple modules with that name. One in the")
                        .into_iter()
                        .chain([text(dep.to_string()).color(Color::DullYellow)])
                        .chain(words("package, and another defined locally in the"))
                        .chain([text(display(path)).color(Color::DullYellow)])
                        .chain(words("file. I do not have a way to choose between them.")),
                ),
                reflow(
                    "Try changing the name of the locally defined module to clear up the \
                     ambiguity?",
                ),
            ]);
            ("AMBIGUOUS IMPORT", after)
        }
        ImportProblem::AmbiguousLocal(paths) => {
            let after = stack([
                reflow("But I found multiple files in your \"source-directories\" with that name:"),
                vcat(paths.iter().map(|p| text(display(p))))
                    .indent(4)
                    .color(Color::DullYellow),
                reflow("Change the module names to be distinct!"),
            ]);
            ("AMBIGUOUS IMPORT", after)
        }
        ImportProblem::AmbiguousForeign { dep, rivals } => {
            let after = stack([
                reflow(
                    "But multiple packages in your \"dependencies\" that expose a module that \
                     name:",
                ),
                vcat(
                    std::iter::once(dep)
                        .chain(rivals)
                        .map(|p| text(p.to_string())),
                )
                .indent(4)
                .color(Color::DullYellow),
                reflow(
                    "There is no way to disambiguate in cases like this right now. Of the known \
                     name clashes, they are usually for packages with similar purposes, so the \
                     current recommendation is to pick just one of them.",
                ),
                simple_note(
                    "It seems possible to resolve this with new syntax in imports, but that is \
                     more complicated than it sounds. Right now, our module names are tied to \
                     GitHub repos, but we may want to get rid of that dependency for a variety \
                     of reasons. That would in turn have implications for our package \
                     infrastructure, hosting costs, and possibly on how package names are \
                     specified. The particular syntax chosen seems like it would interact with \
                     all these factors in ways that are difficult to predict, potentially \
                     leading to harder problems later on. So more design work and planning is \
                     needed on these topics.",
                ),
            ]);
            ("AMBIGUOUS IMPORT", after)
        }
    };
    let message = source.snippet(compat, before, region, None, after);
    Problem::new(title, region, message)
}

/// `Pkg.suggestions`: the package to install for a well-known module.
fn package_for(name: &ModuleName) -> Option<&'static str> {
    Some(match name.to_str()? {
        "Browser" => "elm/browser",
        "File" | "File.Download" | "File.Select" => "elm/file",
        "Html" | "Html.Attributes" | "Html.Events" => "elm/html",
        "Http" => "elm/http",
        "Json.Decode" | "Json.Encode" => "elm/json",
        "Random" => "elm/random",
        "Time" => "elm/time",
        "Url.Parser" | "Url" => "elm/url",
        _ => return None,
    })
}

fn display(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

/// `toProjectProblemReport` for the problems the crawl finds.
pub fn project_problem(problem: &GraphProblem, compat: Compat) -> Report {
    let red = |path: &Path| text(display(path)).color(Color::Red).indent(4);
    let red_lines = |paths: &[&Path]| {
        vcat(paths.iter().map(|p| text(display(p))))
            .color(Color::Red)
            .indent(4)
    };
    match problem {
        GraphProblem::PathUnknown(path) => Report::new(
            "FILE NOT FOUND",
            None,
            "I cannot find this file:",
            [
                red(path),
                reflow("Is there a typo?"),
                simple_note(
                    "If you are just getting started, try working through the examples in the \
                     official guide https://guide.elm-lang.org to get an idea of the kinds of \
                     things that typically go in a src/Main.elm file.",
                ),
            ],
        ),
        GraphProblem::WithBadExtension(path) => Report::new(
            "UNEXPECTED FILE EXTENSION",
            None,
            "I can only compile Elm files (with a .elm extension) but you want me to compile:",
            [
                red(path),
                reflow("Is there a typo? Can the file extension be changed?"),
            ],
        ),
        GraphProblem::WithAmbiguousSrcDir {
            path,
            first,
            second,
        } => Report::new(
            "CONFUSING FILE",
            None,
            "I am getting confused when I try to compile this file:",
            [
                red(path),
                reflow(
                    "I always check if files appear in any of the \"source-directories\" listed \
                     in your elm.json to see if there might be some cached information about \
                     them. That can help me compile faster! But in this case, it looks like \
                     this file may be in either of these directories:",
                ),
                red_lines(&[first, second]),
                reflow("Try to make it so no source directory contains another source directory!"),
            ],
        ),
        GraphProblem::MainPathDuplicate { first, second } => Report::new(
            "CONFUSING FILES",
            None,
            "You are telling me to compile these two files:",
            [
                red_lines(&[first, second]),
                reflow(if first == second {
                    "Why are you telling me twice? Is something weird going on with a script? I \
                     figured I would let you know about it just in case something is wrong. Only \
                     list it once and you should be all set!"
                } else {
                    "But seem to be the same file though... It makes me think something tricky \
                     is going on with symlinks in your project, so I figured I would let you \
                     know about it just in case. Remove one of these files from your command to \
                     get unstuck!"
                }),
            ],
        ),
        GraphProblem::RootNameDuplicate {
            name,
            first,
            second,
        } => Report::new(
            "MODULE NAME CLASH",
            None,
            "These two files are causing a module name clash:",
            [
                red_lines(&[first, second]),
                reflow(&format!(
                    "They both say `module {name} exposing (..)` up at the top, but they cannot \
                     have the same name!"
                )),
                reflow("Try changing to a different module name in one of them!"),
            ],
        ),
        GraphProblem::RootNameInvalid { path, src_dir, .. } => {
            let mut dir = display(src_dir);
            if !dir.ends_with(std::path::MAIN_SEPARATOR) {
                dir.push(std::path::MAIN_SEPARATOR);
            }
            Report::new(
                "UNEXPECTED FILE NAME",
                None,
                "I am having trouble with this file name:",
                [
                    red(path),
                    reflow(&format!(
                        "I found it in your {dir} directory which is good, but I expect all of \
                         the files in there to use the following module naming convention:"
                    )),
                    naming_table(src_dir, &["Main", "HomePage", "Http.Helpers"]),
                    reflow(
                        "Notice that the names always start with capital letters! Can you make \
                         your file use this naming convention?",
                    ),
                    simple_note(
                        "Having a strict naming convention like this makes it a lot easier to \
                         find things in large projects. If you see a module imported, you know \
                         where to look for the corresponding file every time!",
                    ),
                ],
            )
        }
        GraphProblem::Cycle { first, rest } => Report::doc(
            "IMPORT CYCLE",
            None,
            reflow("Your module imports form a cycle:"),
            [
                cycle(std::iter::once(first).chain(rest)),
                learn_about_cycles(compat),
            ],
        ),
    }
}

/// `D.cycle`.
fn cycle<'n>(names: impl Iterator<Item = &'n ModuleName>) -> Doc {
    let (top, line, mid, end) = if cfg!(windows) {
        ("+-----+", "|    ", "|     |", "+-<---+")
    } else {
        ("┌─────┐", "│    ", "│     ↓", "└─────┘")
    };
    let mut rows = vec![text(top)];
    for (i, name) in names.enumerate() {
        if i > 0 {
            rows.push(text(mid));
        }
        rows.push(text(line).append(text(name.to_string()).color(Color::DullYellow)));
    }
    rows.push(text(end));
    vcat(rows).indent(4)
}

/// Before 0.19.3 the link follows `here:` with no space, so the two wrap
/// as one word.
fn learn_about_cycles(compat: Compat) -> Doc {
    const BEFORE: &str = "Learn more about why this is disallowed and how to break cycles here:";
    match compat {
        Compat::V0_19_1 | Compat::V0_19_2 => {
            let mut words: Vec<Doc> = BEFORE.split(' ').map(text).collect();
            let last = words.pop().expect("words");
            fill_sep(
                words
                    .into_iter()
                    .chain([last.append(Doc::link("import-cycles"))]),
            )
        }
        Compat::V0_19_3 => reflow_link(BEFORE, "import-cycles", ""),
    }
}

/// `toModuleNameConventionTable`.
fn naming_table(src_dir: &Path, names: &[&str]) -> Doc {
    let pairs: Vec<(String, String)> = names
        .iter()
        .map(|name| {
            let file: PathBuf = name.split('.').collect();
            let path = src_dir.join(file).with_extension("elm");
            ((*name).to_owned(), display(&path))
        })
        .collect();
    let width = |s: &String| s.chars().count();
    let name_width = pairs.iter().map(|(n, _)| width(n)).fold(11, usize::max);
    let path_width = pairs.iter().map(|(_, p)| width(p)).fold(9, usize::max);
    let row =
        |name: &str, path: &str| text(format!("| {name:<name_width$} | {path:<path_width$} |",));
    let bar = text(format!(
        "+-{}-+-{}-+",
        "-".repeat(name_width),
        "-".repeat(path_width)
    ));
    let mut rows = vec![bar.clone(), row("Module Name", "File Path"), bar.clone()];
    rows.extend(pairs.iter().map(|(n, p)| row(n, p)));
    rows.push(bar);
    vcat(rows).indent(4)
}

/// `finalizeExposed`'s `BP_MissingExposed`: the first exposed module, in
/// the order `elm.json` lists them, that names no one module.
pub fn missing_exposed(graph: &Graph, exposed: &[ModuleName]) -> Option<Report> {
    let (name, problem) = exposed
        .iter()
        .find_map(|name| match graph.modules.get(name) {
            Some(Status::BadImport(problem)) => Some((name, problem)),
            _ => None,
        })?;
    let module = text(name.to_string()).color(Color::Red).indent(4);
    let start = "The  \"exposed-modules\" of your elm.json lists the following module:";
    let path = Some("elm.json".to_owned());
    let not_found = "But I cannot find it in your src/ directory. Is there a typo? Was it renamed?";
    Some(match problem {
        ImportProblem::NotFound => {
            Report::new("MISSING MODULE", path, start, [module, reflow(not_found)])
        }
        ImportProblem::Ambiguous { dep, .. } => Report::new(
            "AMBIGUOUS MODULE NAME",
            path,
            start,
            [
                module,
                reflow(&format!(
                    "But a module from {dep} already uses that name. Try choosing a different \
                     name for your local file."
                )),
            ],
        ),
        ImportProblem::AmbiguousLocal(paths) => Report::new(
            "AMBIGUOUS MODULE NAME",
            path,
            start,
            [
                module,
                reflow("But I found multiple files with that name:"),
                vcat(paths.iter().map(|p| text(display(p))))
                    .indent(4)
                    .color(Color::DullYellow),
                reflow("Change the module names to be distinct!"),
            ],
        ),
        ImportProblem::AmbiguousForeign { .. } => Report::new(
            "MISSING MODULE",
            path,
            start,
            [
                module,
                reflow(not_found),
                simple_note(
                    "It is not possible to \"re-export\" modules from other packages. You can \
                     only expose modules that you define in your own code.",
                ),
            ],
        ),
    })
}

#[cfg(test)]
mod tests {
    use nomi_base::diagnostic::render::{self, Styles};
    use nomi_base::{Compat, TextRange};
    use nomi_project::Name;

    use super::*;

    fn plain(doc: &Doc) -> String {
        render::render(doc, Compat::LATEST, 80, Styles::Plain)
    }

    fn name(text: &str) -> Name {
        Name::parse(
            text.as_bytes(),
            nomi_project::json::Cursor::start(Compat::LATEST),
            Compat::LATEST,
        )
        .unwrap()
    }

    // `elm make` can't reach these: its argument parser takes only `.elm`
    // files, and the oracle's projects have no two packages exposing one
    // module.

    #[test]
    fn a_root_that_isnt_elm() {
        let report = project_problem(
            &GraphProblem::WithBadExtension(PathBuf::from("src/main.js")),
            Compat::LATEST,
        );
        assert_eq!(report.title, "UNEXPECTED FILE EXTENSION");
        assert_eq!(
            plain(&report.message),
            "I can only compile Elm files (with a .elm extension) but you want me to \
             compile:\n\n    src/main.js\n\n\
             Is there a typo? Can the file extension be changed?"
        );
    }

    #[test]
    fn packages_exposing_one_module() {
        let text = "module Main exposing (main)\n\nimport Url\n";
        let source = Source::new(text);
        let region = source.region(TextRange::new(36.into(), 39.into()), Compat::LATEST);
        let problem = ImportProblem::AmbiguousForeign {
            dep: name("elm/url"),
            rivals: vec![name("author/url")],
        };
        let problem = import_problem(
            &source,
            Compat::LATEST,
            region,
            &ModuleName::new("Url"),
            &[],
            &problem,
        );
        assert_eq!(problem.title, "AMBIGUOUS IMPORT");
        let plain = plain(&problem.message);
        assert!(plain.contains("3| import Url\n          ^^^"), "{plain}");
        assert!(plain.contains("\n    elm/url\n    author/url\n"), "{plain}");
    }
}
