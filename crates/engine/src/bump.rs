//! `elm bump` (`Bump.hs`): the version a package's API changes call for,
//! from a version it may be bumped from, written to `elm.json` once asked.

use nomi_base::Compat;
use nomi_base::diagnostic::Report;
use nomi_base::diagnostic::doc::{Color, Doc, fill_sep, reflow, text, vcat, words};
use nomi_project::Outline;
use nomi_project::outline::PkgOutline;
use nomi_project::report::elm_json_path;
use nomi_project::version::Version;
use nomi_registry::{Http, Known};

use crate::console::{self, Answer, Console, Personality};
use crate::diff::{self, Magnitude, Stop};
use crate::packages::Resolve;
use crate::paths::Root;
use crate::project::Project;
use crate::{Context, Vfs};

/// `elm bump` for the project around the directory, its documentation
/// built with its dependencies resolved as `personality` resolves them.
/// Whether nothing was wrong.
pub fn run(
    cx: &Context<'_, impl Http + Sync>,
    console: &mut dyn Console,
    personality: Personality,
) -> bool {
    let Some(root) = Root::find(cx.vfs, cx.dir) else {
        console::report(console, &no_outline(), Compat::LATEST);
        return false;
    };
    let project = Project::read(cx.vfs, &root, Personality::Elm);
    let compat = project.as_ref().map_or(Compat::LATEST, |p| p.compat);
    bump(cx, console, project, compat, Resolve::of(personality)).unwrap_or_else(|stop| {
        stop.show(console, root.dir(), compat);
        false
    })
}

fn bump(
    cx: &Context<'_, impl Http + Sync>,
    console: &mut dyn Console,
    project: Result<Project, Report>,
    compat: Compat,
    resolve: Resolve,
) -> Result<bool, Stop> {
    let index = diff::latest(
        cx,
        compat,
        "I need the latest list of published packages before I can bump any versions",
    )?;
    let project = project?;
    let Outline::Pkg(pkg) = &project.outline else {
        return Err(application().into());
    };
    let Some(known) = index.versions(&pkg.name) else {
        return Ok(new_package(cx.vfs, console, &project, pkg)?);
    };
    let mut bumpable: Vec<Version> = (possibilities(compat, known).into_iter())
        .map(|possibility| possibility.old)
        .collect();
    if !bumpable.contains(&pkg.version) {
        bumpable.sort();
        bumpable.dedup();
        return Err(unexpected_version(pkg.version, &bumpable).into());
    }
    let purpose = "compute the next version number";
    let old = diff::fetch_docs(cx, compat, &pkg.name, pkg.version, purpose)?;
    let new = diff::local_docs(cx, &project, resolve)?;
    let magnitude = diff::magnitude(&diff::diff(&old, &new));
    let version = magnitude.bump(pkg.version, compat);
    let (old_text, new_text) = (pkg.version.to_string(), version.to_string());
    let question = vcat([
        text("Based on your new API, this should be a")
            .space(text(magnitude.name()).color(Color::Green))
            .space(text(format!("change ({old_text} => {new_text})"))),
        text("Bail out of this command and run 'elm diff' for a full explanation."),
        text(""),
        text(format!(
            "Should I perform the update ({old_text} => {new_text}) in elm.json? [Y/n] "
        )),
    ]);
    Ok(change_version(
        cx.vfs, console, &project, pkg, version, &question,
    )?)
}

/// A version a release may follow, the one it would be, and how much
/// that changes.
pub(crate) struct Possibility {
    pub(crate) old: Version,
    pub(crate) new: Version,
    pub(crate) magnitude: Magnitude,
}

/// `Bump.getPossibilities`: a major bump from the newest, a minor one
/// from the last of each major version, a patch from the last of each
/// minor one.
pub(crate) fn possibilities(compat: Compat, known: &Known) -> Vec<Possibility> {
    let mut all: Vec<Version> = known.all().collect();
    all.reverse();
    let last_of = |same: fn(&Version, &Version) -> bool| {
        let mut points: Vec<Version> = Vec::new();
        for v in &all {
            match points.last_mut() {
                Some(last) if same(last, v) => *last = *v,
                _ => points.push(*v),
            }
        }
        points
    };
    let possibility = |old: Version, magnitude: Magnitude| Possibility {
        old,
        new: magnitude.bump(old, compat),
        magnitude,
    };
    let mut out = vec![possibility(known.newest, Magnitude::Major)];
    out.extend(
        last_of(|a, b| a.major == b.major)
            .into_iter()
            .map(|v| possibility(v, Magnitude::Minor)),
    );
    out.extend(
        last_of(|a, b| a.major == b.major && a.minor == b.minor)
            .into_iter()
            .map(|v| possibility(v, Magnitude::Patch)),
    );
    out
}

/// `checkNewPackage`: how versions work, and the version put back to
/// 1.0.0 if it was changed.
fn new_package(
    vfs: &Vfs,
    console: &mut dyn Console,
    project: &Project,
    pkg: &PkgOutline,
) -> Result<bool, Report> {
    console.out(&format!("{NEW_PACKAGE_OVERVIEW}\n"));
    let one = Version::ONE;
    if pkg.version == one {
        console.out("The version number in elm.json is correct so you are all set!\n");
        return Ok(true);
    }
    let question = vcat([
        text("It looks like the version in elm.json has been changed though!"),
        text(format!(
            "Would you like me to change it back to {one}? [Y/n] "
        )),
    ]);
    change_version(vfs, console, project, pkg, one, &question)
}

/// `changeVersion`: `question` asked, and on a yes the version written.
fn change_version(
    vfs: &Vfs,
    console: &mut dyn Console,
    project: &Project,
    pkg: &PkgOutline,
    version: Version,
    question: &Doc,
) -> Result<bool, Report> {
    let compat = project.compat;
    let answer = console::ask(console, question, compat).unwrap_or(Answer::Ended);
    console::answered(console, Personality::Elm, answer, |console| {
        let pkg = PkgOutline {
            version,
            ..pkg.clone()
        };
        project.write(vfs, Outline::Pkg(pkg).write())?;
        let changed = text("Version changed to ")
            .append(text(version.to_string()).color(Color::Green))
            .append(text("!"));
        console::print(console, &changed, compat);
        console.out("\n");
        Ok(())
    })
}

/// `Exit.newPackageOverview`, which `unlines` ends with an empty line;
/// `putStrLn` adds the last newline.
pub(crate) const NEW_PACKAGE_OVERVIEW: &str =
    "This package has never been published before. Here's how things work:

  - Versions all have exactly three parts: MAJOR.MINOR.PATCH

  - All packages start with initial version 1.0.0

  - Versions are incremented based on how the API changes:

        PATCH = the API is the same, no risk of breaking code
        MINOR = values have been added, existing values are unchanged
        MAJOR = existing values have been changed or removed

  - I will bump versions for you, automatically enforcing these rules

";

/// `BumpNoOutline`.
fn no_outline() -> Report {
    Report::new(
        "BUMP WHAT?",
        None,
        "I cannot find an elm.json so I am not sure what you want me to bump.",
        [reflow(
            "Elm packages always have an elm.json that says current the version number. If you \
             run this command from a directory with an elm.json file, I will try to bump the \
             version in there based on the API changes.",
        )],
    )
}

/// `BumpApplication`.
fn application() -> Report {
    Report::new(
        "CANNOT BUMP APPLICATIONS",
        elm_json_path(),
        "Your elm.json says this is an application. That means it cannot be published on \
         <https://package.elm-lang.org> and therefore has no version to bump!",
        [],
    )
}

/// `BumpUnexpectedVersion`.
fn unexpected_version(version: Version, versions: &[Version]) -> Report {
    let message = words("Your elm.json says I should bump relative to version")
        .chain([text(version.to_string())
            .color(Color::Red)
            .append(text(","))])
        .chain(words(
            "but I cannot find that version on <https://package.elm-lang.org>. That means \
             there is no API for me to diff against and figure out if these are MAJOR, MINOR, \
             or PATCH changes.",
        ));
    let hint = words("Try bumping again after changing the")
        .chain([text("\"version\"").color(Color::DullYellow)])
        .chain(words("in elm.json"))
        .chain(words(if versions.len() == 1 {
            "to:"
        } else {
            "to one of these:"
        }));
    Report::doc(
        "CANNOT BUMP",
        elm_json_path(),
        fill_sep(message),
        [
            fill_sep(hint),
            vcat(
                versions
                    .iter()
                    .map(|v| text(v.to_string()).color(Color::Green)),
            ),
        ],
    )
}
