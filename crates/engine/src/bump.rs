//! `elm bump` (`Bump.hs`): the version a package's API changes call for,
//! from a version it may be bumped from, written to `elm.json` once asked.

use std::path::Path;

use nomi_base::Compat;
use nomi_base::diagnostic::Report;
use nomi_base::diagnostic::doc::{Color, Doc, fill_sep, text, vcat};
use nomi_project::Outline;
use nomi_project::version::Version;
use nomi_registry::{Http, Known};

use crate::Vfs;
use crate::console::{self, Console, Personality};
use crate::diff::{self, Magnitude, Stop};
use crate::paths::{Cache, Root};
use crate::project::Project;

/// `elm bump` for the project around `dir`. Whether nothing was wrong.
pub fn run(
    vfs: &Vfs,
    console: &mut dyn Console,
    dir: &Path,
    cache: &Cache,
    http: &(impl Http + Sync),
) -> bool {
    let Some(root) = Root::find(vfs, dir) else {
        console::report(console, &no_outline(), Compat::LATEST);
        return false;
    };
    let compat = Project::read(vfs, &root, Personality::Elm).map_or(Compat::LATEST, |p| p.compat);
    match bump(vfs, console, &root, cache, http, compat) {
        Ok(done) => done,
        Err(Stop::Report(report)) => {
            console::report(console, &report, compat);
            false
        }
        Err(Stop::Build(checked)) => {
            crate::make::show_reports(console, root.dir(), &checked, compat);
            false
        }
    }
}

fn bump(
    vfs: &Vfs,
    console: &mut dyn Console,
    root: &Root,
    cache: &Cache,
    http: &(impl Http + Sync),
    compat: Compat,
) -> Result<bool, Stop> {
    let index = diff::latest(
        http,
        cache,
        compat,
        "I need the latest list of published packages before I can bump any versions",
    )?;
    let project = Project::read(vfs, root, Personality::Elm)?;
    let Outline::Pkg(pkg) = &project.outline else {
        return Err(application().into());
    };
    let Some(known) = index.versions(&pkg.name) else {
        return Ok(new_package(vfs, console, &project, compat));
    };
    let mut bumpable: Vec<Version> = possibilities(compat, known)
        .into_iter()
        .map(|(old, _, _)| old)
        .collect();
    if !bumpable.contains(&pkg.version) {
        bumpable.sort();
        bumpable.dedup();
        return Err(unexpected_version(pkg.version, &bumpable).into());
    }
    let purpose = "compute the next version number";
    let old = diff::fetch_docs(cache, http, compat, &pkg.name, pkg.version, purpose)?;
    let new = diff::local_docs(vfs, root, &project, cache, http)?;
    let changes = diff::diff(&old, &new);
    let version = diff::bump(compat, &changes, pkg.version);
    let (old_text, new_text) = (pkg.version.to_string(), version.to_string());
    let question = vcat([
        text("Based on your new API, this should be a")
            .space(text(diff::magnitude(&changes).name()).color(Color::Green))
            .space(text(format!("change ({old_text} => {new_text})"))),
        text("Bail out of this command and run 'elm diff' for a full explanation."),
        text(""),
        text(format!(
            "Should I perform the update ({old_text} => {new_text}) in elm.json? [Y/n] "
        )),
    ]);
    Ok(change_version(
        vfs, console, &project, version, &question, compat,
    ))
}

/// `Bump.getPossibilities`: each version a release may follow, with the
/// one it would be: a major bump from the newest, a minor one from the
/// last of each major version, a patch from the last of each minor one.
pub(crate) fn possibilities(compat: Compat, known: &Known) -> Vec<(Version, Version, Magnitude)> {
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
    let mut out = vec![(
        known.newest,
        known.newest.bump_major(compat),
        Magnitude::Major,
    )];
    out.extend(
        last_of(|a, b| a.major == b.major)
            .into_iter()
            .map(|v| (v, v.bump_minor(compat), Magnitude::Minor)),
    );
    out.extend(
        last_of(|a, b| a.major == b.major && a.minor == b.minor)
            .into_iter()
            .map(|v| (v, v.bump_patch(compat), Magnitude::Patch)),
    );
    out
}

/// `checkNewPackage`: how versions work, and the version put back to
/// 1.0.0 if it was changed.
fn new_package(vfs: &Vfs, console: &mut dyn Console, project: &Project, compat: Compat) -> bool {
    console.out(&format!("{NEW_PACKAGE_OVERVIEW}\n"));
    let Outline::Pkg(pkg) = &project.outline else {
        unreachable!("a package");
    };
    let one = Version::ONE;
    if pkg.version == one {
        console.out("The version number in elm.json is correct so you are all set!\n");
        return true;
    }
    let question = vcat([
        text("It looks like the version in elm.json has been changed though!"),
        text(format!(
            "Would you like me to change it back to {one}? [Y/n] "
        )),
    ]);
    change_version(vfs, console, project, one, &question, compat)
}

/// `changeVersion`: `question` asked, and on a yes the version written.
fn change_version(
    vfs: &Vfs,
    console: &mut dyn Console,
    project: &Project,
    version: Version,
    question: &Doc,
    compat: Compat,
) -> bool {
    let approved = match console::ask(console, question, compat) {
        Ok(Some(approved)) => approved,
        _ => {
            console::no_answer(console, Personality::Elm);
            return false;
        }
    };
    if !approved {
        console.out("Okay, I did not change anything!\n");
        return true;
    }
    let Outline::Pkg(pkg) = &project.outline else {
        unreachable!("a package");
    };
    let mut pkg = pkg.clone();
    pkg.version = version;
    if let Err(report) = project.write(vfs, &Outline::Pkg(pkg)) {
        console::report(console, &report, compat);
        return false;
    }
    let changed = text("Version changed to ")
        .append(text(version.to_string()).color(Color::Green))
        .append(text("!"));
    console::print(console, &changed, compat);
    console.out("\n");
    true
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
        [nomi_base::diagnostic::doc::reflow(
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
        Some("elm.json".to_owned()),
        "Your elm.json says this is an application. That means it cannot be published on \
         <https://package.elm-lang.org> and therefore has no version to bump!",
        [],
    )
}

/// `BumpUnexpectedVersion`.
fn unexpected_version(version: Version, versions: &[Version]) -> Report {
    let words = |s: &str| s.split_whitespace().map(text).collect::<Vec<_>>();
    let mut message = words("Your elm.json says I should bump relative to version");
    message.push(
        text(version.to_string())
            .color(Color::Red)
            .append(text(",")),
    );
    message.extend(words(
        "but I cannot find that version on <https://package.elm-lang.org>. That means there \
         is no API for me to diff against and figure out if these are MAJOR, MINOR, or PATCH \
         changes.",
    ));
    let mut hint = words("Try bumping again after changing the");
    hint.push(text("\"version\"").color(Color::DullYellow));
    hint.extend(words("in elm.json"));
    hint.extend(words(if versions.len() == 1 {
        "to:"
    } else {
        "to one of these:"
    }));
    Report::doc(
        "CANNOT BUMP",
        Some("elm.json".to_owned()),
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
