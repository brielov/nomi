//! `elm publish` (`Publish.hs`): a package checked, then registered. The
//! checks, each shown as it runs: a summary and exposed modules, a README
//! of 300 bytes or more, a LICENSE, documentation that builds, a version
//! the API changes allow, a git tag that GitHub has too, no changes since
//! it, and GitHub's zipball of it building. Then `elm.json`, the docs, the
//! README and the zipball's hash go to the package website.

use std::path::Path;
use std::process::{Command, Stdio};

use nomi_base::Compat;
use nomi_base::diagnostic::Report;
use nomi_base::diagnostic::doc::{Color, fill_sep, reflow, simple_note, text, vcat, words};
use nomi_project::report::elm_json_path;
use nomi_project::version::Version;
use nomi_project::{Name, Outline};
use nomi_registry::http::DOMAIN;
use nomi_registry::report::INTERCEPTED;
use nomi_registry::{Http, HttpError, Known, Part};

use crate::Context;
use crate::bump::{NEW_PACKAGE_OVERVIEW, Possibility, possibilities};
use crate::console::{self, Console, Personality};
use crate::diff::{self, Magnitude, Stop};
use crate::packages::Resolve;
use crate::paths::{ELM_JSON, LICENSE, README, Root};
use crate::project::Project;
use crate::report::one_line;

/// `elm publish` for the project around the directory. Whether it
/// published.
pub fn run(cx: &Context<'_, impl Http + Sync>, console: &mut dyn Console) -> bool {
    let Some(root) = Root::find(cx.vfs, cx.dir) else {
        console::report(console, &no_outline(), Compat::LATEST);
        return false;
    };
    let project = Project::read(cx.vfs, &root, Personality::Elm);
    let compat = project.as_ref().map_or(Compat::LATEST, |p| p.compat);
    let published = publish(cx, console, project, compat);
    if let Err(stop) = &published {
        stop.show(console, root.dir(), compat);
    }
    published.is_ok()
}

fn publish(
    cx: &Context<'_, impl Http + Sync>,
    console: &mut dyn Console,
    project: Result<Project, Report>,
    compat: Compat,
) -> Result<(), Stop> {
    let index = diff::latest(
        cx,
        compat,
        "I need the latest list of published packages to make sure this is safe to publish",
    )?;
    let project = project?;
    let Outline::Pkg(pkg) = &project.outline else {
        return Err(one_line(
            "UNPUBLISHABLE",
            "I cannot publish applications, only packages!",
        )
        .into());
    };
    let (name, version) = (&pkg.name, pkg.version);
    let known = index.versions(name);
    match known {
        None => console.out(&format!(
            "{NEW_PACKAGE_OVERVIEW}\nI will now verify that everything is in order...\n\n"
        )),
        Some(_) => console.out(&format!("Verifying {name} {version} ...\n\n")),
    }
    if pkg.exposed.modules().next().is_none() {
        return Err(no_exposed().into());
    }
    let summary = String::from_utf8_lossy(&pkg.summary);
    if summary.is_empty() || summary == "helpful summary of your project, less than 80 characters" {
        return Err(no_summary().into());
    }
    let dir = project.root.dir();
    verify_readme(console, compat, dir)?;
    verify_license(console, compat, dir)?;
    let docs_json = check(
        console,
        compat,
        Check {
            waiting: "Verifying documentation...",
            success: "Verified documentation",
            failure: "Problem with documentation",
        },
        || diff::local_docs_json(cx, &project, Resolve::Upstream),
    )?;
    verify_version(cx, console, compat, name, version, &docs_json, known)?;
    if !git(dir, &["--version"]) {
        return Err(no_git().into());
    }
    let hash = verify_tag(cx.http, console, compat, dir, name, version)?;
    verify_no_changes(console, compat, dir, &hash, version)?;
    let zip_hash = verify_zip(cx, console, compat, name, version)?;
    console.out("\n");
    let url = format!("{DOMAIN}/register?name={name}&version={version}&commit-hash={hash}");
    register(cx.http, &url, dir, docs_json, zip_hash)?;
    console.out("Success!\n");
    Ok(())
}

/// `verifyReadme`: a README of 300 bytes or more.
fn verify_readme(console: &mut dyn Console, compat: Compat, dir: &Path) -> Result<(), Stop> {
    let messages = Check {
        waiting: "Looking for README.md",
        success: "Found README.md",
        failure: "Problem with your README.md",
    };
    check(console, compat, messages, || {
        match std::fs::metadata(dir.join(README)) {
            Err(_) => Err(bad_readme(
                "NO README",
                "Every published package must have a helpful README.md file, but I do not see \
                 one in your project.",
            )
            .into()),
            Ok(meta) if meta.len() < 300 => Err(bad_readme(
                "SHORT README",
                "This README.md is too short. Having more details will help people assess your \
                 package quickly and fairly.",
            )
            .into()),
            Ok(_) => Ok(()),
        }
    })
}

/// `verifyLicense`.
fn verify_license(console: &mut dyn Console, compat: Compat, dir: &Path) -> Result<(), Stop> {
    let messages = Check {
        waiting: "Looking for LICENSE",
        success: "Found LICENSE",
        failure: "Problem with your LICENSE",
    };
    check(console, compat, messages, || {
        if dir.join(LICENSE).exists() {
            Ok(())
        } else {
            Err(no_license().into())
        }
    })
}

/// `register`: `elm.json`, the documentation, the README and the hash of
/// GitHub's zipball, sent to the package website's `url`.
fn register(
    http: &impl Http,
    url: &str,
    dir: &Path,
    docs_json: String,
    zip_hash: String,
) -> Result<(), Stop> {
    let file = |name: &str, bytes: Vec<u8>| Part {
        name: name.to_owned(),
        file_name: Some(name.to_owned()),
        bytes,
    };
    let read = |name: &str| match std::fs::read(dir.join(name)) {
        Ok(bytes) => Ok(file(name, bytes)),
        Err(e) => Err(one_line(
            "PROBLEM PUBLISHING PACKAGE",
            &format!("I could not read {name}: {e}"),
        )),
    };
    let parts = [
        read(ELM_JSON)?,
        file("docs.json", docs_json.into_bytes()),
        read(README)?,
        Part {
            name: "github-hash".to_owned(),
            file_name: None,
            bytes: zip_hash.into_bytes(),
        },
    ];
    http.upload(url, &parts).map_err(|e| {
        nomi_registry::report::http_error(
            "PROBLEM PUBLISHING PACKAGE",
            url,
            &e,
            "I need to send information about your package to the package website",
        )
    })?;
    Ok(())
}

/// A check's messages: while it runs, when it passes, when it fails.
#[derive(Clone, Copy)]
struct Check<'a> {
    waiting: &'a str,
    success: &'a str,
    failure: &'a str,
}

/// `reportCheck`: the waiting line, the work, then the line overwritten
/// with how it went, padded to cover the waiting message.
fn check<T>(
    console: &mut dyn Console,
    compat: Compat,
    messages: Check,
    work: impl FnOnce() -> Result<T, Stop>,
) -> Result<T, Stop> {
    let success = |_: &T| messages.success.to_owned();
    custom_check(
        console,
        compat,
        messages.waiting,
        success,
        messages.failure,
        work,
    )
}

/// `reportCustomCheck`.
fn custom_check<T>(
    console: &mut dyn Console,
    compat: Compat,
    waiting: &str,
    success: impl FnOnce(&T) -> String,
    failure: &str,
    work: impl FnOnce() -> Result<T, Stop>,
) -> Result<T, Stop> {
    let [good, bad, wait] = if cfg!(windows) {
        ["+", "X", "-"]
    } else {
        ["●", "✗", "→"]
    };
    let wait = text(wait).color(Color::DullYellow);
    console::print(
        console,
        &text("  ").append(wait).space(text(waiting)),
        compat,
    );
    let result = work();
    let padded = |message: &str| {
        let pad = waiting
            .chars()
            .count()
            .saturating_sub(message.chars().count());
        format!("{message}{}", " ".repeat(pad))
    };
    if let Ok(value) = &result {
        let good = text(good).color(Color::Green);
        let line = text("\r  ")
            .append(good)
            .space(text(padded(&success(value))));
        console::print(console, &line, compat);
        console.out("\n");
    } else {
        let bad = text(bad).color(Color::Red);
        let line = text("\r  ").append(bad).space(text(padded(failure)));
        console::print(console, &line, compat);
        console.out("\n\n");
    }
    result
}

/// `verifyVersion`: 1.0.0 for a new package; otherwise not published yet,
/// and the bump the API changes call for from a version it may follow.
fn verify_version(
    cx: &Context<'_, impl Http>,
    console: &mut dyn Console,
    compat: Compat,
    name: &Name,
    version: Version,
    docs_json: &str,
    known: Option<&Known>,
) -> Result<(), Stop> {
    enum Good {
        Start,
        Bump(Version, Magnitude),
    }
    let waiting = format!("Checking semantic versioning rules. Is {version} correct?");
    let failure = format!("Version {version} is not correct!");
    let success = |good: &Good| match good {
        Good::Start => "All packages start at version 1.0.0".to_owned(),
        Good::Bump(old, magnitude) => format!(
            "Version number {version} verified ({} change, {old} => {version})",
            magnitude.name()
        ),
    };
    custom_check(console, compat, &waiting, success, &failure, || {
        let Some(known) = known else {
            return if version == Version::ONE {
                Ok(Good::Start)
            } else {
                Err(not_initial_version(version).into())
            };
        };
        if known.all().any(|v| v == version) {
            return Err(already_published(version).into());
        }
        let found = (possibilities(compat, known).into_iter()).find(|p| p.new == version);
        let Some(Possibility {
            old,
            new,
            magnitude,
        }) = found
        else {
            return Err(invalid_bump(version, known.newest).into());
        };
        let purpose = format!("verify that {new} really does come next");
        let old_docs = diff::fetch_docs(cx, compat, name, old, &purpose)?;
        let docs = diff::decode_built(compat, docs_json);
        let real_magnitude = diff::magnitude(&diff::diff(&old_docs, &docs));
        let real = real_magnitude.bump(old, compat);
        if real == new {
            Ok(Good::Bump(old, magnitude))
        } else {
            Err(bad_bump(old, new, magnitude, real, real_magnitude).into())
        }
    })
    .map(|_| ())
}

/// Whether `git args` succeeds in `dir`.
fn git(dir: &Path, args: &[&str]) -> bool {
    Command::new("git")
        .args(args)
        .current_dir(dir)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// `verifyTag`: the tag in git, then on GitHub, whose commit it names.
fn verify_tag(
    http: &impl Http,
    console: &mut dyn Console,
    compat: Compat,
    dir: &Path,
    name: &Name,
    version: Version,
) -> Result<String, Stop> {
    let messages = Check {
        waiting: &format!("Is version {version} tagged on GitHub?"),
        success: &format!("Version {version} is tagged on GitHub"),
        failure: &format!("Version {version} is not tagged on GitHub!"),
    };
    check(console, compat, messages, || {
        if !git(dir, &["show", "--name-only", &version.to_string(), "--"]) {
            return Err(missing_tag(version).into());
        }
        let url = format!("https://api.github.com/repos/{name}/git/refs/tags/{version}");
        let body = http.get(&url).map_err(|e| match e {
            HttpError::Status(404) => no_tag_on_github(version),
            e => nomi_registry::report::http_error(
                "PROBLEM VERIFYING TAG",
                &url,
                &e,
                "I need to check that the version tag is registered on GitHub",
            ),
        })?;
        let hash = serde_json::from_slice::<serde_json::Value>(&body)
            .ok()
            .and_then(|json| json["object"]["sha"].as_str().map(str::to_owned));
        hash.ok_or_else(|| Stop::Report(cannot_get_tag_data(version, &url, &body)))
    })
}

/// `verifyNoChanges`: the working directory holds the tagged commit,
/// `hash`.
fn verify_no_changes(
    console: &mut dyn Console,
    compat: Compat,
    dir: &Path,
    hash: &str,
    version: Version,
) -> Result<(), Stop> {
    let messages = Check {
        waiting: "Checking for uncommitted changes...",
        success: "No uncommitted changes in local code",
        failure: "Your local code is different than the code tagged on GitHub",
    };
    check(console, compat, messages, || {
        if git(dir, &["diff-index", "--quiet", hash, "--"]) {
            Ok(())
        } else {
            Err(local_changes(version).into())
        }
    })
}

/// `verifyZip`: GitHub's zipball of the tag downloaded and built, and its
/// hash.
fn verify_zip(
    cx: &Context<'_, impl Http + Sync>,
    console: &mut dyn Console,
    compat: Compat,
    name: &Name,
    version: Version,
) -> Result<String, Stop> {
    let url = format!("https://github.com/{name}/zipball/{version}/");
    let context = "I need to check that folks can download and build the source code when they install this package";
    let archive = check(
        console,
        compat,
        Check {
            waiting: "Downloading code from GitHub...",
            success: "Code downloaded successfully from GitHub",
            failure: "Could not download code from GitHub!",
        },
        || {
            cx.http.get(&url).map_err(|e| {
                nomi_registry::report::http_error("PROBLEM DOWNLOADING CODE", &url, &e, context)
                    .into()
            })
        },
    )?;
    let hash = nomi_registry::store::sha1(&archive);
    let prepublish = tempfile::tempdir()
        .map_err(|e| Stop::Report(one_line("PROBLEM DOWNLOADING CODE", &e.to_string())))?;
    if nomi_registry::store::unpack(&archive, prepublish.path()).is_err() {
        return Err(cannot_decode_zip(&url).into());
    }
    check(
        console,
        compat,
        Check {
            waiting: "Verifying downloaded code...",
            success: "Downloaded code compiles successfully",
            failure: "Cannot compile downloaded code!",
        },
        || {
            let root = Root::new(prepublish.path().to_owned());
            let built = Project::read(cx.vfs, &root, Personality::Elm)
                .ok()
                .and_then(|project| diff::local_docs_json(cx, &project, Resolve::Upstream).ok());
            built.map(|_| ()).ok_or_else(|| bad_zip().into())
        },
    )?;
    Ok(hash)
}

/// `PublishNoOutline`.
fn no_outline() -> Report {
    Report::new(
        "PUBLISH WHAT?",
        None,
        "I cannot find an elm.json so I am not sure what you want me to publish.",
        [reflow(
            "Elm packages always have an elm.json that states the version number, dependencies, exposed modules, etc.",
        )],
    )
}

/// `PublishNotInitialVersion`.
fn not_initial_version(version: Version) -> Report {
    let message = words("I cannot publish")
        .chain([text(version.to_string()).color(Color::Red)])
        .chain(words("as the initial version."));
    let hint = words("Change it to")
        .chain([text("1.0.0").color(Color::Green)])
        .chain(words("which is the initial version for all Elm packages."));
    Report::doc("INVALID VERSION", None, fill_sep(message), [fill_sep(hint)])
}

/// `PublishAlreadyPublished`.
fn already_published(version: Version) -> Report {
    let first = words("Version")
        .chain([text(version.to_string()).color(Color::Green)])
        .chain(words(
            "has already been published. You cannot publish it again!",
        ));
    Report::doc(
        "ALREADY PUBLISHED",
        None,
        vcat([fill_sep(first), text("Try using the `bump` command:")]),
        [
            text("elm bump").indent(4).color(Color::DullYellow),
            reflow(
                "It computes the version number based on API changes, ensuring that no breaking changes end up in PATCH releases!",
            ),
        ],
    )
}

/// `PublishInvalidBump`.
fn invalid_bump(stated: Version, latest: Version) -> Report {
    let message = words("Your elm.json says the next version should be")
        .chain([text(stated.to_string()).color(Color::Red).append(text(","))])
        .chain(words(
            "but that is not valid based on the previously published versions.",
        ));
    let hint = words("Change the version back to")
        .chain([text(latest.to_string()).color(Color::Green)])
        .chain(words(
            "which is the most recently published version. From there, have Elm bump the \
             version by running:",
        ));
    Report::doc(
        "INVALID VERSION",
        elm_json_path(),
        fill_sep(message),
        [
            fill_sep(hint),
            text("elm bump").color(Color::Green).indent(4),
            reflow(
                "If you want more insight on the API changes Elm detects, you can run `elm diff` at this point as well.",
            ),
        ],
    )
}

/// `PublishBadBump`.
fn bad_bump(
    old: Version,
    new: Version,
    magnitude: Magnitude,
    real: Version,
    real_magnitude: Magnitude,
) -> Report {
    let message = words("Your elm.json says the next version should be")
        .chain([text(new.to_string()).color(Color::Red).append(text(","))])
        .chain(words("indicating a"))
        .chain([text(magnitude.name())])
        .chain(words(
            "change to the public API. This does not match the API diff given by:",
        ));
    let says = words("This command says this is a")
        .chain([text(real_magnitude.name())])
        .chain(words("change, so the next version should be"))
        .chain([text(real.to_string()).color(Color::Green).append(text("."))])
        .chain(words(
            "Double check everything to make sure you are publishing what you want!",
        ));
    Report::doc(
        "INVALID VERSION",
        elm_json_path(),
        fill_sep(message),
        [
            text(format!("elm diff {old}")).indent(4),
            fill_sep(says),
            reflow("Also, next time use `elm bump` and I'll figure all this out for you!"),
        ],
    )
}

/// `PublishNoSummary`.
fn no_summary() -> Report {
    let message = words("To publish a package, your elm.json must have a")
        .chain([text("\"summary\"").color(Color::DullYellow)])
        .chain(words(
            "field that gives a consice overview of your project.",
        ));
    Report::doc(
        "NO SUMMARY",
        elm_json_path(),
        fill_sep(message),
        [reflow(
            "The summary must be less than 80 characters. It should describe the concrete use of your package as clearly and as plainly as possible.",
        )],
    )
}

/// `PublishNoExposed`.
fn no_exposed() -> Report {
    let message = words("To publish a package, the")
        .chain([text("\"exposed-modules\"").color(Color::DullYellow)])
        .chain(words(
            "field of your elm.json must list at least one module.",
        ));
    Report::doc(
        "NO EXPOSED MODULES",
        elm_json_path(),
        fill_sep(message),
        [reflow(
            "Which modules do you want users of the package to have access to? Add their names to the \"exposed-modules\" list.",
        )],
    )
}

/// `toBadReadmeReport`.
fn bad_readme(title: &'static str, summary: &str) -> Report {
    Report::new(
        title,
        Some(README.to_owned()),
        summary,
        [
            reflow("When people look at your README, they are wondering:"),
            vcat([
                text("  - What does this package even do?"),
                text("  - Will it help me solve MY problems?"),
            ]),
            reflow(
                "So I recommend starting your README with a small example of the most common usage scenario. Show people what they can expect if they learn more!",
            ),
            simple_note(
                "By publishing your package, you are inviting people to invest time in understanding your work. Spending an hour on your README to communicate your knowledge more clearly can save the community days or weeks of time in aggregate, and saving time in aggregate is the whole point of publishing packages! People really appreciate it, and it makes the whole ecosystem feel nicer!",
            ),
        ],
    )
}

/// `PublishNoLicense`.
fn no_license() -> Report {
    Report::new(
        "NO LICENSE FILE",
        Some(LICENSE.to_owned()),
        "By publishing a package you are inviting the Elm community to build upon your work. But without knowing your license, we have no idea if that is legal!",
        [reflow(
            "Once you pick an OSI approved license from <https://spdx.org/licenses/>, you must share that choice in two places. First, the license identifier must appear in your elm.json file. Second, the full license text must appear in the root of your project in a file named LICENSE. Add that file and you will be all set!",
        )],
    )
}

/// `PublishMissingTag`.
fn missing_tag(version: Version) -> Report {
    let message = words("Packages must be tagged in git, but I cannot find a")
        .chain([text(version.to_string()).color(Color::Green), text("tag.")]);
    Report::doc(
        "NO TAG",
        None,
        fill_sep(message),
        [
            vcat([
                text("These tags make it possible to find this specific version on GitHub."),
                text("To tag the most recent commit and push it to GitHub, run this:"),
            ]),
            vcat([
                text(format!("git tag -a {version} -m \"new release\"")),
                text(format!("git push origin {version}")),
            ])
            .color(Color::DullYellow)
            .indent(4),
            text("The -m flag is for a helpful message. Try to make it more informative!"),
        ],
    )
}

/// `PublishCannotGetTag` for a 404.
fn no_tag_on_github(version: Version) -> Report {
    Report::new(
        "NO TAG ON GITHUB",
        None,
        &format!("You have version {version} tagged locally, but not on GitHub."),
        [
            reflow("Run the following command to make this tag available on GitHub:"),
            text(format!("git push origin {version}"))
                .color(Color::DullYellow)
                .indent(4),
            reflow(
                "This will make it possible to find your code online based on the version number.",
            ),
        ],
    )
}

/// `PublishCannotGetTagData`.
fn cannot_get_tag_data(version: Version, url: &str, body: &[u8]) -> Report {
    nomi_registry::report::unexpected_body(
        "PROBLEM VERIFYING TAG",
        url,
        body,
        &format!("I need to check that version {version} is tagged on GitHub"),
    )
}

/// `PublishCannotDecodeZip`.
fn cannot_decode_zip(url: &str) -> Report {
    Report::new(
        "PROBLEM DOWNLOADING CODE",
        None,
        "I need to check that folks can download and build the source code when they install this package, so I downloaded the code from:",
        [
            text(url).color(Color::DullYellow).indent(4),
            reflow(&format!(
                "I was unable to unzip the archive though. Maybe there is something weird with \
                 your internet connection. {INTERCEPTED}"
            )),
        ],
    )
}

/// `PublishNoGit`.
fn no_git() -> Report {
    Report::new(
        "NO GIT",
        None,
        "I searched your PATH environment variable for `git` and could not find it. Is it available through your PATH?",
        [
            reflow(
                "Who cares about this? Well, I currently use `git` to check if there are any local changes in your code. Local changes are a good sign that some important improvements have gotten mistagged, so this check can be extremely helpful for package authors!",
            ),
            simple_note("We plan to do this without the `git` binary in a future release."),
        ],
    )
}

/// `PublishLocalChanges`.
fn local_changes(version: Version) -> Report {
    let message = words("The code tagged as")
        .chain([text(version.to_string()).color(Color::Green)])
        .chain(words(
            "in git does not match the code in your working directory. This means you have \
             commits or local changes that are not going to be published!",
        ));
    Report::doc(
        "LOCAL CHANGES",
        None,
        fill_sep(message),
        [simple_note(&format!(
            "If you are sure everything is in order, you can run `git checkout {version}` and publish your code from there."
        ))],
    )
}

/// `badZipReport`.
fn bad_zip() -> Report {
    Report::new(
        "PROBLEM VERIFYING PACKAGE",
        None,
        "Before publishing packages, I download the code from GitHub and try to build it from scratch. That way I can be more confident that it will work for other people too. But I am not able to build it!",
        [reflow(
            "I was just able to build your local copy though. Is there some way the version on GitHub could be different?",
        )],
    )
}
