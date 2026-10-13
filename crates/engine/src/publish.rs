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
use nomi_base::diagnostic::doc::{Color, Doc, fill_sep, reflow, simple_note, text, vcat};
use nomi_project::version::Version;
use nomi_project::{Name, Outline};
use nomi_registry::http::DOMAIN;
use nomi_registry::{Http, HttpError, Known, Part};

use crate::Vfs;
use crate::console::{self, Console, Personality};
use crate::diff::{self, Magnitude, Stop};
use crate::paths::{Cache, Root};
use crate::project::Project;

/// `elm publish` for the project around `dir`. Whether it published.
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
    match publish(vfs, console, &root, cache, http, compat) {
        Ok(()) => true,
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

fn publish(
    vfs: &Vfs,
    console: &mut dyn Console,
    root: &Root,
    cache: &Cache,
    http: &(impl Http + Sync),
    compat: Compat,
) -> Result<(), Stop> {
    let index = diff::latest(
        http,
        cache,
        compat,
        "I need the latest list of published packages to make sure this is safe to publish",
    )?;
    let project = Project::read(vfs, root, Personality::Elm)?;
    let Outline::Pkg(pkg) = &project.outline else {
        return Err(Report::new(
            "UNPUBLISHABLE",
            None,
            "I cannot publish applications, only packages!",
            [],
        )
        .into());
    };
    let (name, version) = (pkg.name.clone(), pkg.version);
    let known = index.versions(&name);
    match known {
        None => console.out(&format!(
            "{}\nI will now verify that everything is in order...\n\n",
            crate::bump::NEW_PACKAGE_OVERVIEW
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
    let dir = root.dir();
    check(
        console,
        compat,
        Check::fixed(
            "Looking for README.md",
            "Found README.md",
            "Problem with your README.md",
        ),
        || {
            match std::fs::metadata(dir.join("README.md")) {
            Err(_) => Err(bad_readme("NO README", "Every published package must have a helpful README.md file, but I do not see one in your project.").into()),
            Ok(meta) if meta.len() < 300 => Err(bad_readme("SHORT README", "This README.md is too short. Having more details will help people assess your package quickly and fairly.").into()),
            Ok(_) => Ok(()),
        }
        },
    )?;
    check(
        console,
        compat,
        Check::fixed(
            "Looking for LICENSE",
            "Found LICENSE",
            "Problem with your LICENSE",
        ),
        || {
            if dir.join("LICENSE").exists() {
                Ok(())
            } else {
                Err(no_license().into())
            }
        },
    )?;
    let (docs, docs_json) = check(
        console,
        compat,
        Check::fixed(
            "Verifying documentation...",
            "Verified documentation",
            "Problem with documentation",
        ),
        || diff::local_docs_json(vfs, root, &project, cache, http),
    )?;
    verify_version(console, cache, http, compat, &name, version, &docs, known)?;
    if !git_available() {
        return Err(no_git().into());
    }
    let tagged = format!("Is version {version} tagged on GitHub?");
    let hash = check(
        console,
        compat,
        Check::fixed(
            &tagged,
            &format!("Version {version} is tagged on GitHub"),
            &format!("Version {version} is not tagged on GitHub!"),
        ),
        || verify_tag(dir, http, &name, version),
    )?;
    check(
        console,
        compat,
        Check::fixed(
            "Checking for uncommitted changes...",
            "No uncommitted changes in local code",
            "Your local code is different than the code tagged on GitHub",
        ),
        || {
            if git(dir, &["diff-index", "--quiet", &hash, "--"]) {
                Ok(())
            } else {
                Err(local_changes(version).into())
            }
        },
    )?;
    let zip_hash = verify_zip(console, compat, vfs, cache, http, &name, version)?;
    console.out("\n");
    let url = format!("{DOMAIN}/register?name={name}&version={version}&commit-hash={hash}");
    let file = |name: &str| -> Result<Vec<u8>, Stop> {
        std::fs::read(dir.join(name)).map_err(|e| {
            Stop::Report(Report::new(
                "PROBLEM PUBLISHING PACKAGE",
                None,
                &format!("I could not read {name}: {e}"),
                [],
            ))
        })
    };
    let parts = [
        Part {
            name: "elm.json".to_owned(),
            file_name: Some("elm.json".to_owned()),
            bytes: file("elm.json")?,
        },
        Part {
            name: "docs.json".to_owned(),
            file_name: Some("docs.json".to_owned()),
            bytes: docs_json.into_bytes(),
        },
        Part {
            name: "README.md".to_owned(),
            file_name: Some("README.md".to_owned()),
            bytes: file("README.md")?,
        },
        Part {
            name: "github-hash".to_owned(),
            file_name: None,
            bytes: zip_hash.into_bytes(),
        },
    ];
    http.upload(&url, &parts).map_err(|e| {
        nomi_registry::report::http_error(
            "PROBLEM PUBLISHING PACKAGE",
            &url,
            &e,
            "I need to send information about your package to the package website",
        )
    })?;
    console.out("Success!\n");
    Ok(())
}

/// A check's messages: while it runs, when it passes, when it fails.
struct Check {
    waiting: String,
    success: String,
    failure: String,
}

impl Check {
    fn fixed(waiting: &str, success: &str, failure: &str) -> Check {
        Check {
            waiting: waiting.to_owned(),
            success: success.to_owned(),
            failure: failure.to_owned(),
        }
    }
}

/// `reportCheck`: the waiting line, the work, then the line overwritten
/// with how it went, padded to cover the waiting message.
fn check<T>(
    console: &mut dyn Console,
    compat: Compat,
    messages: Check,
    work: impl FnOnce() -> Result<T, Stop>,
) -> Result<T, Stop> {
    custom_check(
        console,
        compat,
        &messages.waiting,
        |_| messages.success,
        &messages.failure,
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
    let (good, bad, wait) = marks();
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
        console::print(
            console,
            &text("\r  ")
                .append(good)
                .space(text(padded(&success(value)))),
            compat,
        );
        console.out("\n");
    } else {
        console::print(
            console,
            &text("\r  ").append(bad).space(text(padded(failure))),
            compat,
        );
        console.out("\n\n");
    }
    result
}

/// The good, bad and waiting marks.
fn marks() -> (Doc, Doc, Doc) {
    if cfg!(windows) {
        (
            text("+").color(Color::Green),
            text("X").color(Color::Red),
            text("-").color(Color::DullYellow),
        )
    } else {
        (
            text("●").color(Color::Green),
            text("✗").color(Color::Red),
            text("→").color(Color::DullYellow),
        )
    }
}

/// `verifyVersion`: 1.0.0 for a new package; otherwise not published yet,
/// and the bump the API changes call for from a version it may follow.
#[expect(
    clippy::too_many_arguments,
    reason = "upstream's `verifyVersion` takes these, with its environment spread out"
)]
fn verify_version(
    console: &mut dyn Console,
    cache: &Cache,
    http: &impl Http,
    compat: Compat,
    name: &Name,
    version: Version,
    docs: &diff::Documentation,
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
        let found = crate::bump::possibilities(compat, known)
            .into_iter()
            .find(|(_, new, _)| *new == version);
        let Some((old, new, magnitude)) = found else {
            return Err(invalid_bump(version, known.newest).into());
        };
        let purpose = format!("verify that {new} really does come next");
        let old_docs = diff::fetch_docs(cache, http, compat, name, old, &purpose)?;
        let changes = diff::diff(&old_docs, docs);
        let real = diff::bump(compat, &changes, old);
        if real == new {
            Ok(Good::Bump(old, magnitude))
        } else {
            Err(bad_bump(old, new, magnitude, real, diff::magnitude(&changes)).into())
        }
    })
    .map(|_| ())
}

/// Whether `git` is on the path.
fn git_available() -> bool {
    Command::new("git")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok()
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
fn verify_tag(dir: &Path, http: &impl Http, name: &Name, version: Version) -> Result<String, Stop> {
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
}

/// `verifyZip`: GitHub's zipball of the tag downloaded and built, and its
/// hash.
fn verify_zip(
    console: &mut dyn Console,
    compat: Compat,
    vfs: &Vfs,
    cache: &Cache,
    http: &(impl Http + Sync),
    name: &Name,
    version: Version,
) -> Result<String, Stop> {
    let url = format!("https://github.com/{name}/zipball/{version}/");
    let context = "I need to check that folks can download and build the source code when they install this package";
    let archive = check(
        console,
        compat,
        Check::fixed(
            "Downloading code from GitHub...",
            "Code downloaded successfully from GitHub",
            "Could not download code from GitHub!",
        ),
        || {
            http.get(&url).map_err(|e| {
                nomi_registry::report::http_error("PROBLEM DOWNLOADING CODE", &url, &e, context)
                    .into()
            })
        },
    )?;
    let hash = nomi_registry::store::sha1(&archive);
    let prepublish = tempfile::tempdir().map_err(|e| {
        Stop::Report(Report::new(
            "PROBLEM DOWNLOADING CODE",
            None,
            &e.to_string(),
            [],
        ))
    })?;
    if nomi_registry::store::unpack(&archive, prepublish.path()).is_err() {
        return Err(cannot_decode_zip(&url).into());
    }
    check(
        console,
        compat,
        Check::fixed(
            "Verifying downloaded code...",
            "Downloaded code compiles successfully",
            "Cannot compile downloaded code!",
        ),
        || {
            let root = Root::new(prepublish.path().to_owned());
            let built = Project::read(vfs, &root, Personality::Elm)
                .ok()
                .and_then(|project| diff::local_docs(vfs, &root, &project, cache, http).ok());
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

fn words(s: &str) -> Vec<Doc> {
    s.split_whitespace().map(text).collect()
}

/// `PublishNotInitialVersion`.
fn not_initial_version(version: Version) -> Report {
    let mut message = words("I cannot publish");
    message.push(text(version.to_string()).color(Color::Red));
    message.extend(words("as the initial version."));
    let mut hint = words("Change it to");
    hint.push(text("1.0.0").color(Color::Green));
    hint.extend(words("which is the initial version for all Elm packages."));
    Report::doc("INVALID VERSION", None, fill_sep(message), [fill_sep(hint)])
}

/// `PublishAlreadyPublished`.
fn already_published(version: Version) -> Report {
    let mut first = words("Version");
    first.push(text(version.to_string()).color(Color::Green));
    first.extend(words(
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
    let mut message = words("Your elm.json says the next version should be");
    message.push(text(stated.to_string()).color(Color::Red).append(text(",")));
    message.extend(words(
        "but that is not valid based on the previously published versions.",
    ));
    let mut hint = words("Change the version back to");
    hint.push(text(latest.to_string()).color(Color::Green));
    hint.extend(words("which is the most recently published version. From there, have Elm bump the version by running:"));
    Report::doc(
        "INVALID VERSION",
        Some("elm.json".to_owned()),
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
    let mut message = words("Your elm.json says the next version should be");
    message.push(text(new.to_string()).color(Color::Red).append(text(",")));
    message.extend(words("indicating a"));
    message.push(text(magnitude.name()));
    message.extend(words(
        "change to the public API. This does not match the API diff given by:",
    ));
    let mut says = words("This command says this is a");
    says.push(text(real_magnitude.name()));
    says.extend(words("change, so the next version should be"));
    says.push(text(real.to_string()).color(Color::Green).append(text(".")));
    says.extend(words(
        "Double check everything to make sure you are publishing what you want!",
    ));
    Report::doc(
        "INVALID VERSION",
        Some("elm.json".to_owned()),
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
    let mut message = words("To publish a package, your elm.json must have a");
    message.push(text("\"summary\"").color(Color::DullYellow));
    message.extend(words(
        "field that gives a consice overview of your project.",
    ));
    Report::doc(
        "NO SUMMARY",
        Some("elm.json".to_owned()),
        fill_sep(message),
        [reflow(
            "The summary must be less than 80 characters. It should describe the concrete use of your package as clearly and as plainly as possible.",
        )],
    )
}

/// `PublishNoExposed`.
fn no_exposed() -> Report {
    let mut message = words("To publish a package, the");
    message.push(text("\"exposed-modules\"").color(Color::DullYellow));
    message.extend(words(
        "field of your elm.json must list at least one module.",
    ));
    Report::doc(
        "NO EXPOSED MODULES",
        Some("elm.json".to_owned()),
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
        Some("README.md".to_owned()),
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
        Some("LICENSE".to_owned()),
        "By publishing a package you are inviting the Elm community to build upon your work. But without knowing your license, we have no idea if that is legal!",
        [reflow(
            "Once you pick an OSI approved license from <https://spdx.org/licenses/>, you must share that choice in two places. First, the license identifier must appear in your elm.json file. Second, the full license text must appear in the root of your project in a file named LICENSE. Add that file and you will be all set!",
        )],
    )
}

/// `PublishMissingTag`.
fn missing_tag(version: Version) -> Report {
    let mut message = words("Packages must be tagged in git, but I cannot find a");
    message.push(text(version.to_string()).color(Color::Green));
    message.push(text("tag."));
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
    let shown = String::from_utf8_lossy(body);
    let (whole, shown) = if body.len() <= 76 {
        ("whole thing:", shown.into_owned())
    } else {
        (
            "beginning:",
            format!("{}...", shown.chars().take(73).collect::<String>()),
        )
    };
    Report::new(
        "PROBLEM VERIFYING TAG",
        None,
        &format!("I need to check that version {version} is tagged on GitHub, so I fetched:"),
        [
            text(url).color(Color::DullYellow).indent(4),
            reflow(&format!(
                "I got the data back, but it was not what I was expecting. The response body contains {} bytes. Here is the {whole}",
                body.len()
            )),
            text(shown).color(Color::DullYellow).indent(4),
            reflow(
                "Does this error keep showing up? Maybe there is something weird with your internet connection. We have gotten reports that schools, businesses, airports, etc. sometimes intercept requests and add things to the body or change its contents entirely. Could that be the problem?",
            ),
        ],
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
            reflow(
                "I was unable to unzip the archive though. Maybe there is something weird with your internet connection. We have gotten reports that schools, businesses, airports, etc. sometimes intercept requests and add things to the body or change its contents entirely. Could that be the problem?",
            ),
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
    let mut message = words("The code tagged as");
    message.push(text(version.to_string()).color(Color::Green));
    message.extend(words(
        "in git does not match the code in your working directory. This means you have commits or local changes that are not going to be published!",
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
