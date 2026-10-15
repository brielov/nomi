//! The elm workloads: nomi, run as `elm`, against each pinned elm, end to
//! end. `elm make` builds each corpus app's entries in dev and with
//! `--optimize`, and `elm make --docs` documents [`PACKAGES`]. A cold build
//! starts without `elm-stuff` or any built dependency: elm's
//! `artifacts.dat` files are removed from its `ELM_HOME`, and nomi's
//! artifact cache is emptied. A warm build follows it with nothing
//! changed. Both tools run offline over the same package sources, each in
//! its own copy of the project, and must exit alike and write the same
//! output.

use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, ensure};
use nomi_base::Compat;
use nomi_testkit::files_below;

use super::measure::{Cost, Run, measure};
use super::{Row, Setup, Table};
use crate::corpus::{apps, compat, corpus_dir};
use crate::oracle::corpus_runs::{cache_releases, copy_modules};
use crate::oracle::sandbox::{Release, elm_home};
use crate::oracle::{cache_dir, fresh_dir, link, pinned_elm, remove_tree};
use crate::references::References;

const RUNS: usize = 3;

/// The packages documented: the core library, three widely used ones, the
/// largest corpus package by modules and one of the largest by bytes,
/// among those every release documents.
const PACKAGES: &[(&str, &str)] = &[
    ("elm/core", "1.0.5"),
    ("mdgriffith/elm-ui", "1.1.8"),
    ("rtfeldman/elm-css", "18.0.0"),
    ("dillonkearns/elm-graphql", "5.0.12"),
    ("finos/morphir-elm", "22.0.2"),
    ("icidasset/elm-material-icons", "11.0.0"),
];

/// One tool's side: its binary, environment, and how to empty what it
/// built.
struct Tool {
    binary: PathBuf,
    env: Vec<(&'static str, PathBuf)>,
    clear: Box<dyn Fn() -> Result<()>>,
}

/// `elm make`'s arguments, given where the output goes.
type Args<'a> = Box<dyn Fn(&Path) -> Vec<OsString> + 'a>;

/// A project both tools build: where it comes from, its `elm.json`, and
/// `elm make`'s arguments given where the output goes.
struct Project<'a> {
    name: String,
    top: PathBuf,
    root: &'a str,
    manifest: String,
    args: Args<'a>,
    /// The output's file name.
    output: &'a str,
}

/// A cache for nomi that reads the development cache's packages and
/// registry, with nothing built.
pub(crate) fn nomi_cache(setup: &Setup) -> Result<PathBuf> {
    let store = cache_dir()?.join("nomi");
    let cache = setup.scratch.join("cache");
    fresh_dir(&cache)?;
    link(&store.join("packages"), &cache.join("packages"))?;
    link(&store.join("registry"), &cache.join("registry"))?;
    Ok(cache)
}

pub(crate) fn tables(root: &Path, refs: &References, setup: &Setup) -> Result<Vec<Table>> {
    let releases = cache_releases()?;
    let cache = nomi_cache(setup)?;
    let artifacts = cache.join("artifacts");
    let nomi = Tool {
        binary: setup.nomi("elm"),
        env: vec![("NOMI_CACHE", cache)],
        clear: Box::new(move || remove_tree(&artifacts)),
    };

    let classification = compat::load(root)?;
    let corpus = corpus_dir()?;
    let apps = apps::load(root)?.app;
    let mut make = Table {
        title: "elm make".to_owned(),
        about: format!(
            "nomi, run as `elm`, against each pinned elm (its official build, with its default \
             runtime options), building each corpus app's entries from process start to exit, \
             in dev and with `--optimize`. A cold build starts without `elm-stuff` or any built \
             dependency: elm's `artifacts.dat` files are removed from its `ELM_HOME`, and \
             nomi's artifact cache is emptied. A warm build follows it with nothing changed. \
             Both tools run offline over the same package sources and write the same \
             JavaScript. The best of {RUNS} runs counts for each measure."
        ),
        upstream: "elm".to_owned(),
        rows: Vec::new(),
        refused: Vec::new(),
    };
    let mut docs = Table {
        title: "elm make --docs".to_owned(),
        about: format!(
            "As for `elm make`, documenting packages with `--docs`: the core library, three \
             widely used packages, the largest corpus package by modules and one of the \
             largest by bytes, with the same `docs.json` from both. A package's `elm.json` \
             admits a range of releases, and nomi builds it as the newest of them does, so \
             only the newest elm is measured. The best of {RUNS} runs counts for each measure."
        ),
        upstream: "elm".to_owned(),
        rows: Vec::new(),
        refused: Vec::new(),
    };
    let versions = compat::compats(refs)?;
    let newest = versions.iter().copied().max().context("no elm is pinned")?;
    for version in versions {
        let elm = elm_tool(refs, version, &setup.scratch.join("home"), &releases)?;
        for app in &apps {
            let manifest = compat::sandbox_manifest(&classification, app, version)?;
            for optimize in [false, true] {
                let project = Project {
                    name: format!(
                        "{} {version}{}",
                        app.name,
                        if optimize { " `--optimize`" } else { "" }
                    ),
                    top: apps::app_dir(&corpus, app),
                    root: &app.root,
                    manifest: manifest.clone(),
                    args: Box::new(move |out| {
                        let mut args: Vec<OsString> = vec!["make".into()];
                        args.extend(app.entries.iter().map(OsString::from));
                        let mut output = OsString::from("--output=");
                        output.push(out);
                        args.push(output);
                        if optimize {
                            args.push("--optimize".into());
                        }
                        args
                    }),
                    output: "out.js",
                };
                compare(setup, [&elm, &nomi], &project, &mut make)?;
            }
        }
        for (name, release) in PACKAGES.iter().filter(|_| version == newest) {
            let top = corpus.join("packages").join(name).join(release);
            let manifest = fs::read_to_string(top.join("elm.json")).with_context(|| {
                format!("reading {name} {release}; run `cargo xtask corpus fetch`")
            })?;
            let project = Project {
                name: format!("{name} {release}, {version}"),
                top,
                root: ".",
                manifest,
                args: Box::new(|out| {
                    let mut docs = OsString::from("--docs=");
                    docs.push(out);
                    vec!["make".into(), docs]
                }),
                output: "docs.json",
            };
            compare(setup, [&elm, &nomi], &project, &mut docs)?;
        }
    }
    Ok(vec![make, docs])
}

/// The pinned `version` of elm, its `ELM_HOME` below `homes` holding
/// `releases`, which it builds cold once their `artifacts.dat` are gone.
fn elm_tool(
    refs: &References,
    version: Compat,
    homes: &Path,
    releases: &[Release],
) -> Result<Tool> {
    let home = homes.join(version.as_str());
    let packages = elm_home(&home, version, releases)?;
    Ok(Tool {
        binary: pinned_elm(refs, version)?,
        env: vec![("ELM_HOME", home)],
        clear: Box::new(move || {
            for file in files_below(&packages)? {
                if file.ends_with("artifacts.dat") {
                    fs::remove_file(packages.join(file))?;
                }
            }
            Ok(())
        }),
    })
}

/// Measures both tools building `project` cold and warm, adding a row for
/// each to `table`, or the project to what both refuse.
fn compare(
    setup: &Setup,
    tools: [&Tool; 2],
    project: &Project<'_>,
    table: &mut Table,
) -> Result<()> {
    let report = setup.scratch.join("time.txt");
    let mut costs = [[Cost::MAX; 2]; 2];
    let mut codes = [None; 2];
    let mut outputs = Vec::new();
    for (t, tool) in tools.into_iter().enumerate() {
        let work = setup.scratch.join("work").join(t.to_string());
        remove_tree(&work)?;
        copy_modules(&project.top, &work)?;
        let dir = work.join(project.root);
        fs::create_dir_all(&dir)?;
        fs::write(dir.join("elm.json"), &project.manifest)?;
        let output = work.join(project.output);
        let run = Run {
            binary: &tool.binary,
            args: (project.args)(&output),
            dir: &dir,
            env: &tool.env,
            stdin: None,
            report: &report,
        };
        for i in 0..RUNS {
            (tool.clear)()?;
            remove_tree(&dir.join("elm-stuff"))?;
            for (build, cost) in costs.iter_mut().enumerate() {
                let (measured, code) = measure(&run).with_context(|| {
                    let kind = ["cold", "warm"][build];
                    format!("{} {kind} on {}", tool.binary.display(), project.name)
                })?;
                ensure!(
                    (i == 0 && build == 0) || code == codes[t],
                    "{} exits with {code:?} on {}, before with {:?}",
                    tool.binary.display(),
                    project.name,
                    codes[t]
                );
                codes[t] = code;
                cost[t] = cost[t].min(measured);
            }
            if codes[t] != Some(0) {
                break;
            }
        }
        outputs.push(fs::read(&output).ok());
    }
    ensure!(
        codes[0] == codes[1],
        "elm exits with {:?} on {} but nomi with {:?}",
        codes[0],
        project.name,
        codes[1]
    );
    if codes[0] != Some(0) {
        table.refused.push(project.name.clone());
        return Ok(());
    }
    ensure!(
        outputs[0] == outputs[1],
        "elm and nomi write different output for {}",
        project.name
    );
    for (build, [upstream, nomi]) in ["cold", "warm"].into_iter().zip(costs) {
        table.rows.push(Row {
            name: format!("{} {build}", project.name),
            upstream,
            nomi,
        });
    }
    Ok(())
}
