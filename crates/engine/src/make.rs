//! `elm make` (`Make.hs`, `run`), which `nomi check` is the
//! `--output=/dev/null` of: the project is found, the flags checked, the
//! project and its dependencies loaded, and the modules the paths name, or
//! a package's exposed ones, built; then the documentation is written, or
//! the code generated.

use std::path::{Path, PathBuf};

use nomi_base::Report;
use nomi_base::diagnostic::doc::{Color, Doc, fill_sep, reflow, simple_note, text, vcat};
use nomi_project::{ModuleName, Outline};

use crate::Vfs;
use crate::artifacts::Artifacts;
use crate::check::show;
use crate::check::{self, Checked, Style, env_of, foreigns};
use crate::console::{Console, Personality};
use crate::install::one_line;
use crate::javascript;
use crate::paths::{Cache, Root};
use crate::project::{self, Project, check_source_dirs};
use crate::report;
use nomi_registry::Http;

/// What `elm make` was asked for (`Make.Flags`, with the paths).
#[derive(Clone, Debug)]
pub struct Make {
    pub paths: Vec<PathBuf>,
    pub personality: Personality,
    pub style: Style,
    pub debug: bool,
    pub optimize: bool,
    pub output: Option<Output>,
    /// Where to write a package's `docs.json`.
    pub docs: Option<PathBuf>,
}

/// `Make.Output`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Output {
    Js(PathBuf),
    Html(PathBuf),
    DevNull,
}

impl Output {
    /// `parseOutput`: `/dev/null` and its Windows spellings, or a file
    /// whose name ends in `.html` or `.js` after more.
    pub fn parse(name: &str) -> Option<Output> {
        if ["/dev/null", "NUL", "$null"].contains(&name) {
            Some(Output::DevNull)
        } else if has_ext(".html", name) {
            Some(Output::Html(name.into()))
        } else if has_ext(".js", name) {
            Some(Output::Js(name.into()))
        } else {
            None
        }
    }
}

/// `hasExt`: whether `path`'s extension is `ext`, with a name before it.
pub fn has_ext(ext: &str, path: &str) -> bool {
    take_extension(path) == ext && path.len() > ext.len()
}

/// `FilePath.takeExtension`: from the last dot of the file's name on,
/// which may be its first character.
pub fn take_extension(path: &str) -> &str {
    let name = path
        .rsplit(['/', std::path::MAIN_SEPARATOR])
        .next()
        .unwrap_or(path);
    name.rfind('.').map_or("", |at| &name[at..])
}

impl Make {
    /// What `nomi check` asks for: `elm make --output=/dev/null`.
    pub fn check(paths: &[PathBuf], style: Style) -> Make {
        Make {
            paths: paths.to_vec(),
            personality: Personality::Nomi,
            style,
            debug: false,
            optimize: false,
            output: Some(Output::DevNull),
            docs: None,
        }
    }
}

/// Makes what `make` asks for of the project around `dir`, and shows how
/// it went. Whether nothing was wrong.
pub fn run(
    vfs: &Vfs,
    console: &mut dyn Console,
    dir: &Path,
    cache: &Cache,
    http: &(impl Http + Sync),
    make: &Make,
) -> bool {
    let personality = make.personality;
    let Some(root) = Root::find(vfs, dir) else {
        let report = match personality {
            Personality::Elm => no_outline(),
            Personality::Nomi if make.output == Some(Output::DevNull) => {
                project::no_outline("to check")
            }
            Personality::Nomi => project::no_outline("to build"),
        };
        show(
            console,
            dir,
            &Checked::Project(report),
            nomi_base::Compat::LATEST,
            make.style,
        );
        return false;
    };
    if make.debug && make.optimize {
        let report = cannot_optimize_and_debug();
        show(
            console,
            dir,
            &Checked::Project(report),
            nomi_base::Compat::LATEST,
            make.style,
        );
        return false;
    }
    let mut compat = nomi_base::Compat::LATEST;
    let loaded = (|| {
        let project = Project::read(vfs, &root, personality)?;
        compat = project.compat;
        let root = root.dir().to_owned();
        check_source_dirs(&root, &project.outline)?;
        let (foreigns, interfaces, dependencies) = foreigns(cache, http, &project, personality)?;
        let env = env_of(&root, &project.outline, compat, foreigns)?.with_cwd(dir);
        Ok::<_, Report>((root, project.outline, env, interfaces, dependencies))
    })();
    let terminal = personality == Personality::Elm && make.style == Style::Terminal;
    let (root, outline, env, interfaces, dependencies) = match loaded {
        Ok(loaded) => loaded,
        Err(report) => {
            if terminal {
                details_progress(console, 0);
            }
            show(console, dir, &Checked::Project(report), compat, make.style);
            return false;
        }
    };
    if terminal {
        details_progress(console, dependencies.packages.len());
    }
    let snapshot = vfs.snapshot();
    let artifacts = Artifacts::new(cache);
    let documenting = make.docs.is_some() && make.paths.is_empty();
    let generating = !make.paths.is_empty() && make.output != Some(Output::DevNull);
    let mut optimized = None;
    let (checked, docs, compiled) = if documenting {
        check::docs(&snapshot, &env, &outline, personality, &interfaces)
    } else if generating {
        let (checked, made) = check::optimize(
            &snapshot,
            &env,
            &outline,
            &make.paths,
            personality,
            &interfaces,
            Some(&artifacts),
        );
        let compiled = made.compiled;
        optimized = Some(made);
        (checked, None, compiled)
    } else {
        let (checked, _, compiled) = check::make_with(
            &snapshot,
            &env,
            &outline,
            &make.paths,
            personality,
            &interfaces,
            Some(&artifacts),
        );
        (checked, None, compiled)
    };
    // `elm make` without files, in an application or a package exposing
    // nothing, stops before it builds.
    let no_input = make.paths.is_empty()
        && match &outline {
            Outline::App(_) => true,
            Outline::Pkg(pkg) => pkg.exposed.modules().next().is_none(),
        };
    if terminal && !no_input {
        build_progress(console, compiled, &checked);
    }
    if !matches!(checked, Checked::Fine) {
        if terminal {
            show_reports(console, &root, &checked, compat);
        } else {
            show(console, &root, &checked, compat, make.style);
        }
        return false;
    }
    let fail = |console: &mut dyn Console, report: Report| {
        show(
            console,
            &root,
            &Checked::Project(report),
            compat,
            make.style,
        );
        false
    };
    if let (Some(path), Some(docs)) = (&make.docs, docs) {
        let path = dir.join(path);
        if let Err(e) = vfs.create(&path, docs.as_bytes()) {
            let report = one_line(
                "PROBLEM WRITING DOCS",
                format!("I could not write {}: {e}", path.display()),
            );
            return fail(console, report);
        }
    }
    let mut diagram = None;
    if let Some(optimized) = optimized {
        let target = match target(make.output.as_ref(), &optimized.roots) {
            Ok(target) => target,
            Err(report) => return fail(console, report),
        };
        if let Some(target) = target {
            let kind = if make.optimize {
                let uses = javascript::debug_uses(compat, &optimized);
                if !uses.is_empty() {
                    return fail(console, report::debug_remnants(&uses));
                }
                javascript::Kind::Optimize
            } else if make.debug {
                javascript::Kind::Debug
            } else {
                javascript::Kind::Dev
            };
            let package = match &outline {
                Outline::App(_) => "author/project".to_owned(),
                Outline::Pkg(pkg) => pkg.name.to_string(),
            };
            let types = dependencies.into_graph_and_types();
            let js = javascript::program(compat, kind, &package, types, optimized);
            let bytes = match &target.html {
                Some(module) => nomi_codegen_js::html::sandwich(&module.to_string(), &js),
                None => js,
            };
            let path = dir.join(&target.path);
            let written = (path.parent().map_or(Ok(()), std::fs::create_dir_all))
                .and_then(|()| vfs.create(&path, &bytes));
            if let Err(e) = written {
                let report = one_line(
                    "PROBLEM WRITING OUTPUT",
                    format!("I could not write {}: {e}", path.display()),
                );
                return fail(console, report);
            }
            let names: Vec<String> = target.names.iter().map(|n| n.to_string()).collect();
            diagram = Some(gen_diagram(&names, &target.path.to_string_lossy()));
        }
    }
    // elm's success line came with the build's progress; nomi's comes now.
    if !terminal {
        show(console, &root, &Checked::Fine, compat, make.style);
    }
    if let Some(diagram) = diagram
        && make.style == Style::Terminal
    {
        console.out(&format!("\n{diagram}\n"));
    }
    true
}

/// What elm's terminal style shows of the dependencies' verification
/// (`Reporting.detailsLoop`): each step, then the line cleared and
/// `Dependencies ready!`. elm's steps interleave as its threads do; nomi
/// shows the dependencies found, then each built.
fn details_progress(console: &mut dyn Console, total: usize) {
    let step = |built: usize| format!("Verifying dependencies ({built}/{total})");
    if total > 0 {
        let mut steps = String::new();
        for built in 0..=total {
            steps.push('\r');
            steps.push_str(&step(built));
        }
        console.out(&steps);
    }
    let cleared = " ".repeat(step(total).chars().count());
    console.out(&format!("\r{cleared}\rDependencies ready!\n"));
}

/// What elm's terminal style shows of the build (`Reporting.buildLoop`):
/// `Compiling ...`, a count for each module compiled, then the final
/// message over them.
fn build_progress(console: &mut dyn Console, compiled: usize, checked: &Checked) {
    let mut out = String::from("Compiling ...");
    for done in 1..=compiled {
        out.push_str(&format!("\rCompiling ({done})"));
    }
    let message = match checked {
        Checked::Fine => match compiled {
            0 => "Success!".to_owned(),
            1 => "Success! Compiled 1 module.".to_owned(),
            n => format!("Success! Compiled {n} modules."),
        },
        Checked::Modules(modules) => match modules.len() {
            1 => "Detected problems in 1 module.".to_owned(),
            n => format!("Detected problems in {n} modules."),
        },
        Checked::Project(_) => "Detected a problem.".to_owned(),
    };
    let width = 12 + compiled.to_string().len();
    if message.chars().count() < width {
        out.push_str(&format!("\r{}\r{message}\n", " ".repeat(width)));
    } else {
        out.push_str(&format!("\r{message}\n"));
    }
    console.out(&out);
}

/// The reports of `checked`, without the summary [`build_progress`] has
/// shown.
pub(crate) fn show_reports(
    console: &mut dyn Console,
    root: &Path,
    checked: &Checked,
    compat: nomi_base::Compat,
) {
    match checked {
        Checked::Fine => {}
        Checked::Project(report) => crate::console::report(console, report, compat),
        Checked::Modules(modules) => {
            crate::console::report_doc(console, &report::modules_doc(root, modules), compat);
        }
    }
}

/// `toGenDiagram`: each module given, an arrow to the file they went to.
fn gen_diagram(names: &[String], output: &str) -> String {
    let (hbar, vtop, vmiddle, vbottom) = if cfg!(windows) {
        ('-', '+', '+', '+')
    } else {
        ('─', '┬', '┤', '┘')
    };
    let width = 3 + names.iter().map(|n| n.chars().count()).max().unwrap_or(0);
    let line = |name: &str, end: &str| {
        let bar: String = std::iter::repeat_n(hbar, width - name.chars().count()).collect();
        format!("    {name} {bar}{end}")
    };
    match names {
        [] => String::new(),
        [name] => line(name, &format!("> {output}\n")),
        [first, rest @ ..] => {
            let mut lines = vec![line(first, &format!("{vtop}{hbar}{hbar}> {output}"))];
            for (i, name) in rest.iter().enumerate() {
                let end = if i + 1 == rest.len() {
                    vbottom
                } else {
                    vmiddle
                };
                lines.push(line(name, &end.to_string()));
            }
            lines.iter().map(|l| format!("{l}\n")).collect()
        }
    }
}

/// Where `run` writes the code, the module an HTML page starts, and the
/// modules the terminal style says went there.
struct Target {
    path: PathBuf,
    html: Option<ModuleName>,
    names: Vec<ModuleName>,
}

/// What `Make.hs` generates for `output` from `roots`, each a root and
/// whether it defines `main`: without `--output`, `index.html` for one
/// `main` and `elm.js` for more; JavaScript only when every root has a
/// `main`; HTML only for one root, which does.
fn target(output: Option<&Output>, roots: &[(ModuleName, bool)]) -> Result<Option<Target>, Report> {
    let mains: Vec<&ModuleName> = (roots.iter())
        .filter(|(_, main)| *main)
        .map(|(name, _)| name)
        .collect();
    match output {
        Some(Output::DevNull) => Ok(None),
        None => Ok(match mains.as_slice() {
            [] => None,
            [name] => Some(Target {
                path: "index.html".into(),
                html: Some((*name).clone()),
                names: vec![(*name).clone()],
            }),
            _ => Some(Target {
                path: "elm.js".into(),
                html: None,
                names: mains.iter().map(|n| (*n).clone()).collect(),
            }),
        }),
        Some(Output::Js(path)) => {
            let no_mains: Vec<&ModuleName> = (roots.iter())
                .filter(|(_, main)| !main)
                .map(|(name, _)| name)
                .collect();
            match no_mains.as_slice() {
                [] => Ok(Some(Target {
                    path: path.clone(),
                    html: None,
                    names: roots.iter().map(|(n, _)| n.clone()).collect(),
                })),
                names => Err(non_main_files_into_javascript(names)),
            }
        }
        Some(Output::Html(path)) => match roots {
            [(name, true)] => Ok(Some(Target {
                path: path.clone(),
                html: Some(name.clone()),
                names: vec![name.clone()],
            })),
            [(_, false)] => Err(no_main()),
            _ => Err(multiple_files_into_html()),
        },
    }
}

/// `MakeMultipleFilesIntoHtml`.
fn multiple_files_into_html() -> Report {
    let words = |s: &'static str| s.split_whitespace().map(text).collect::<Vec<_>>();
    let flagged = |before: &'static str, flag: &'static str, after: &'static str| {
        let mut docs = words(before);
        docs.push(text(flag).color(Color::DullYellow));
        docs.extend(words(after));
        fill_sep(docs)
    };
    Report::new(
        "TOO MANY FILES",
        None,
        "When producing an HTML file, I can only handle one file.",
        [
            flagged(
                "Switch to",
                "--output=/dev/null",
                "if you just want to get compile errors. This skips the code gen phase, so it \
                 can be a bit faster than other options sometimes.",
            ),
            flagged(
                "Switch to",
                "--output=elm.js",
                "if you want multiple `main` values available in a single JavaScript file. \
                 Then you can make your own customized HTML file that embeds multiple Elm \
                 nodes. The generated JavaScript also shares dependencies between modules, so \
                 it should be smaller than compiling each module separately.",
            ),
        ],
    )
}

/// The few lines a `main` can be, which the `NO MAIN` reports show.
fn brief_main() -> Doc {
    vcat([
        fill_sep([text("import").color(Color::Cyan), text("Html")]),
        text(""),
        fill_sep([text("main").color(Color::Green), text("=")]),
        fill_sep([
            text("Html").color(Color::Cyan).append(text(".text")),
            text("\"Hello!\"").color(Color::DullYellow),
        ])
        .indent(2),
    ])
}

/// `MakeNoMain`.
fn no_main() -> Report {
    Report::new(
        "NO MAIN",
        None,
        "When producing an HTML file, I require that the given file has a `main` value. That \
         way I have something to show on screen!",
        [
            reflow(
                "Try adding a `main` value to your file? Or if you just want to verify that this \
                 module compiles, switch to --output=/dev/null to skip the code gen phase \
                 altogether.",
            ),
            simple_note("Adding a `main` value can be as brief as adding something like this:"),
            brief_main(),
            reflow(
                "From there I can create an HTML file that says \"Hello!\" on screen. I \
                 recommend looking through https://guide.elm-lang.org for more guidance on how \
                 to fill in the `main` value.",
            ),
        ],
    )
}

/// `MakeNonMainFilesIntoJavaScript`.
fn non_main_files_into_javascript(names: &[&ModuleName]) -> Report {
    let worker = reflow(
        "Or use https://package.elm-lang.org/packages/elm/core/latest/Platform#worker to make a \
         `main` with no user interface.",
    );
    let note = simple_note("Adding a `main` value can be as brief as adding something like this:");
    match names {
        [name] => Report::new(
            "NO MAIN",
            None,
            &format!(
                "When producing a JS file, I require that the given file has a `main` value. \
                 That way Elm.{name}.init() is definitely defined in the resulting file!"
            ),
            [
                reflow(
                    "Try adding a `main` value to your file? Or if you just want to verify that \
                     this module compiles, switch to --output=/dev/null to skip the code gen \
                     phase altogether.",
                ),
                note,
                brief_main(),
                worker,
            ],
        ),
        names => Report::new(
            "NO MAIN",
            None,
            &format!(
                "When producing a JS file, I require that given files all have `main` values. \
                 That way functions like Elm.{}.init() are definitely defined in the resulting \
                 file. I am missing `main` values in:",
                names[0]
            ),
            [
                vcat(names.iter().map(|n| text(n.to_string())))
                    .color(Color::Red)
                    .indent(4),
                reflow(
                    "Try adding a `main` value to them? Or if you just want to verify that these \
                     modules compile, switch to --output=/dev/null to skip the code gen phase \
                     altogether.",
                ),
                note,
                brief_main(),
                worker,
            ],
        ),
    }
}

/// `MakeNoOutline`.
fn no_outline() -> Report {
    Report::new(
        "NO elm.json FILE",
        None,
        "It looks like you are starting a new Elm project. Very exciting! Try running:",
        [
            text("elm init").color(Color::Green).indent(4),
            reflow("It will help you get set up. It is really simple!"),
        ],
    )
}

/// `MakeCannotOptimizeAndDebug`.
fn cannot_optimize_and_debug() -> Report {
    let words = |s: &'static str| s.split_whitespace().map(text).collect::<Vec<_>>();
    let mut first = words("I cannot compile with");
    first.push(text("--optimize").color(Color::Red));
    first.push(text("and"));
    first.push(text("--debug").color(Color::Red));
    first.extend(words("at the same time."));
    Report::doc(
        "CLASHING FLAGS",
        None,
        fill_sep(first),
        [reflow(
            "I need to take away information to optimize things, and I need to add \
             information to add the debugger. It is impossible to do both at once though! \
             Pick just one of those flags and it should work!",
        )],
    )
}
