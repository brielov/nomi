//! `elm init` and `nomi new` (`Init.hs`, the same in every supported
//! release): an application that depends on elm/core, elm/browser and
//! elm/html, at the newest releases that go together.

use std::collections::BTreeMap;
use std::io;
use std::path::Path;

use nomi_base::diagnostic::doc::{Color, Doc, fill_sep, reflow, stack, text, vcat};
use nomi_base::{Compat, Report};
use nomi_project::outline::{AppOutline, SrcDir};
use nomi_project::{Constraint, Name, Outline, Version};
use nomi_registry::{Connection, Http, Store, report, solver};

use crate::console::{self, Console, Personality};
use crate::paths::{Cache, ELM_JSON};
use crate::{Vfs, env};

/// The source directory a new project starts with.
const SRC: &str = "src";

/// Creates a project in `dir`, once the user agrees. Whether it
/// succeeded; a decline is a success.
pub fn run(
    vfs: &Vfs,
    console: &mut dyn Console,
    personality: Personality,
    dir: &Path,
    cache: &Cache,
    http: &impl Http,
) -> bool {
    // Upstream's reports name the release's own pages.
    let compat = Compat::LATEST;
    if vfs.is_file(&dir.join(ELM_JSON)) {
        console::report(console, &already_exists(), compat);
        return false;
    }
    match console::ask(console, &question(), compat) {
        Err(e) => {
            console.err(&format!("error: {e}\n"));
            false
        }
        Ok(None) => {
            console::no_answer(console, personality);
            false
        }
        Ok(Some(false)) => {
            console.out("Okay, I did not make any changes!\n");
            true
        }
        Ok(Some(true)) => match init(vfs, dir, cache, http, compat) {
            Ok(()) => {
                console.out("Okay, I created it. Now read that link!\n");
                true
            }
            Err(problem) => {
                console::report(console, &problem, compat);
                false
            }
        },
    }
}

/// `init`: solves for the defaults and writes the application.
fn init(
    vfs: &Vfs,
    dir: &Path,
    cache: &Cache,
    http: &impl Http,
    compat: Compat,
) -> Result<(), Report> {
    let store = Store::new(cache.packages());
    let mut registry = env::registry(
        cache,
        &store,
        http,
        compat,
        "I need the list of published packages before I can start initializing projects",
    )?;
    let defaults: BTreeMap<Name, Constraint> = [Name::core(), Name::browser(), Name::html()]
        .into_iter()
        .map(|name| (name, Constraint::anything(compat)))
        .collect();
    let solution = match solver::verify(&mut registry, compat, &defaults) {
        Ok(Some(solution)) => solution,
        Ok(None) => return Err(no_solution(&defaults, registry.connection)),
        Err(problem) => return Err(report::solver_problem(&problem)),
    };
    let (direct, indirect) = solution
        .into_iter()
        .map(|(name, details)| (name, details.version))
        .partition(|(name, _)| defaults.contains_key(name));
    let outline = Outline::App(AppOutline {
        elm: Version::compiler(compat),
        source_dirs: vec![SrcDir::new(SRC)],
        direct,
        indirect,
        test_direct: BTreeMap::new(),
        test_indirect: BTreeMap::new(),
    });
    write(vfs, dir, &outline).map_err(|e| {
        Report::new(
            "PROBLEM CREATING PROJECT",
            None,
            &format!("I could not create the project in {}: {e}", dir.display()),
            [],
        )
    })
}

fn write(vfs: &Vfs, dir: &Path, outline: &Outline) -> io::Result<()> {
    std::fs::create_dir_all(dir.join(SRC))?;
    vfs.create(&dir.join(ELM_JSON), outline.write())
}

fn question() -> Doc {
    stack([
        fill_sep(
            ["Hello!", "Elm", "projects", "always", "start", "with", "an"]
                .into_iter()
                .map(text)
                .chain([text("elm.json").color(Color::Green)])
                .chain(
                    ["file.", "I", "can", "create", "them!"]
                        .into_iter()
                        .map(text),
                ),
        ),
        reflow(
            "Now you may be wondering, what will be in this file? How do I add Elm files to my \
             project? How do I see it in the browser? How will my code grow? Do I need more \
             directories? What about tests? Etc.",
        ),
        fill_sep(
            ["Check", "out"]
                .into_iter()
                .map(text)
                .chain([Doc::link("init").color(Color::Cyan)])
                .chain(["for", "all", "the", "answers!"].into_iter().map(text)),
        ),
        text("Knowing all that, would you like me to create an elm.json file now? [Y/n]: "),
    ])
}

/// `InitAlreadyExists`.
fn already_exists() -> Report {
    Report::new(
        "EXISTING PROJECT",
        None,
        "You already have an elm.json file, so there is nothing for me to initialize!",
        [fill_sep(
            std::iter::once(text("Maybe"))
                .chain([Doc::link("init").color(Color::Green)])
                .chain(
                    [
                        "can", "help", "you", "figure", "out", "what", "to", "do", "next?",
                    ]
                    .into_iter()
                    .map(text),
                ),
        )],
    )
}

/// `InitNoSolution` and `InitNoOfflineSolution`.
fn no_solution(defaults: &BTreeMap<Name, Constraint>, connection: Connection) -> Report {
    let names = vcat(
        defaults
            .keys()
            .map(|name| text(name.to_string()).color(Color::DullYellow)),
    )
    .indent(4);
    let (title, advice) = match connection {
        Connection::Online => (
            "NO SOLUTION",
            "I could not find compatible versions though! This should not happen, so please ask \
             around one of the community forums at https://elm-lang.org/community to learn what \
             is going on!",
        ),
        Connection::Offline => (
            "NO OFFLINE SOLUTION",
            "I could not find compatible versions though, but that may be because I could not \
             connect to https://package.elm-lang.org to get the latest list of packages. Are you \
             able to connect to the internet? Please ask around one of the community forums at \
             https://elm-lang.org/community for help!",
        ),
    };
    Report::new(
        title,
        None,
        "I tried to create an elm.json with the following direct dependencies:",
        [names, reflow(advice)],
    )
}
