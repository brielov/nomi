//! `elm make` (`Make.hs`, `run`), which `nomi check` is the
//! `--output=/dev/null` of: the project is found, the flags checked, the
//! project and its dependencies loaded, and the modules the paths name, or
//! a package's exposed ones, built; then the documentation is written, or
//! the code generated, which waits for M8.

use std::path::{Path, PathBuf};

use nomi_base::Report;
use nomi_base::diagnostic::doc::{Color, Doc, fill_sep, reflow, simple_note, text, vcat};
use nomi_project::ModuleName;

use crate::Vfs;
use crate::artifacts::Artifacts;
use crate::check::show;
use crate::check::{self, Checked, Style, env_of, foreigns};
use crate::console::{Console, Personality};
use crate::install::one_line;
use crate::paths::{Cache, Root};
use crate::project::{self, Project, check_source_dirs};
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
            Personality::Nomi => project::no_outline("to check"),
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
        let (foreigns, interfaces) = foreigns(cache, http, &project, personality)?;
        let env = env_of(&root, &project.outline, compat, foreigns)?.with_cwd(dir);
        Ok::<_, Report>((root, project.outline, env, interfaces))
    })();
    let (root, outline, env, interfaces) = match loaded {
        Ok(loaded) => loaded,
        Err(report) => {
            show(console, dir, &Checked::Project(report), compat, make.style);
            return false;
        }
    };
    let snapshot = vfs.snapshot();
    let artifacts = Artifacts::new(cache);
    let documenting = make.docs.is_some() && make.paths.is_empty();
    let mut mains = Vec::new();
    let (checked, docs) = if documenting {
        check::docs(&snapshot, &env, &outline, personality, &interfaces)
    } else {
        let (checked, roots) = check::make_with(
            &snapshot,
            &env,
            &outline,
            &make.paths,
            personality,
            &interfaces,
            Some(&artifacts),
        );
        mains = roots;
        (checked, None)
    };
    if !matches!(checked, Checked::Fine) {
        show(console, &root, &checked, compat, make.style);
        return false;
    }
    if let (Some(path), Some(docs)) = (&make.docs, docs) {
        let path = dir.join(path);
        if let Err(e) = vfs.create(&path, docs.as_bytes()) {
            let report = one_line(
                "PROBLEM WRITING DOCS",
                format!("I could not write {}: {e}", path.display()),
            );
            show(
                console,
                &root,
                &Checked::Project(report),
                compat,
                make.style,
            );
            return false;
        }
    }
    // A package's exposed modules generate nothing.
    if !make.paths.is_empty()
        && let Err(report) = generate(make.output.as_ref(), &mains)
    {
        if let Some(report) = report {
            show(
                console,
                &root,
                &Checked::Project(report),
                compat,
                make.style,
            );
        } else {
            console.err("error: nomi doesn't generate JavaScript yet; pass --output=/dev/null\n");
        }
        return false;
    }
    show(console, &root, &Checked::Fine, compat, make.style);
    true
}

/// What `run` does past the build: nothing for `/dev/null`; the reports
/// on roots that can't make the file asked for; and otherwise the code,
/// which waits for M8, `Err(None)` until then.
fn generate(output: Option<&Output>, mains: &[(ModuleName, bool)]) -> Result<(), Option<Report>> {
    match output {
        Some(Output::DevNull) => Ok(()),
        None if mains.iter().all(|(_, main)| !main) => Ok(()),
        None => Err(None),
        Some(Output::Js(_)) => {
            let no_mains: Vec<&ModuleName> = (mains.iter())
                .filter(|(_, main)| !main)
                .map(|(name, _)| name)
                .collect();
            match no_mains.as_slice() {
                [] => Err(None),
                names => Err(Some(non_main_files_into_javascript(names))),
            }
        }
        Some(Output::Html(_)) => match mains {
            [(_, true)] => Err(None),
            [(_, false)] => Err(Some(no_main())),
            _ => Err(Some(multiple_files_into_html())),
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
