//! `cargo xtask oracle solver`: what upstream's dependency solver answers
//! for each solver scenario, stored for the solver test.
//!
//! The oracle is `oracles/solver/Main.hs` compiled against upstream's own
//! sources with the GHC on `PATH` and the packages `cabal build
//! --only-dependencies` put in its store. 0.19.1's sources no longer build
//! with a current GHC, so its oracle is 0.19.2's sources calling itself
//! 0.19.1; the solver and the outline decoder differ between the two only
//! in plumbing (`oracles/solver/README.md`).
//!
//! A scenario is a fixture registry, the metadata of its releases, a
//! project and optionally a package to add. Each runs in a fresh sandbox
//! per Compat: the releases' `elm.json` files go in the sandbox's
//! `ELM_HOME`, where upstream looks for them, and the solver runs offline.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail, ensure};
use nomi_base::Compat;
use nomi_project::Version;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::sandbox::offline;
use super::{cache_dir, cores, generated, oracles_dir, remove_tree};
use crate::corpus::compat;
use crate::corpus::parallel_on;
use crate::download;
use crate::references::References;

const SUITE: &str = "crates/registry/tests/solver";

/// The seed of `generated.json`. Changing it replaces every generated
/// scenario.
const SEED: u64 = 0x6e6f_6d69_736f_6c76;

/// How many scenarios `generated.json` holds.
const GENERATED: usize = 160;

/// What a Compat's oracle is built from.
struct Recipe {
    /// The reference sources, under `reference/`.
    sources: &'static str,
    /// The shim over the modules that moved between releases, under
    /// `oracles/solver/`.
    shim: &'static str,
}

impl Recipe {
    fn of(compat: Compat) -> Recipe {
        match compat {
            Compat::V0_19_1 | Compat::V0_19_2 => Recipe {
                sources: "compiler-0.19.2",
                shim: "compat-0.19.2",
            },
            Compat::V0_19_3 => Recipe {
                sources: "compiler",
                shim: "compat-0.19.3",
            },
        }
    }
}

/// The packages the oracle links against, all from the cabal store.
const GHC_PACKAGES: &[&str] = &[
    "base",
    "containers",
    "bytestring",
    "binary",
    "directory",
    "filepath",
    "mtl",
    "time",
    "process",
    "array",
    "text",
    "template-haskell",
    "ghc-prim",
    "parsec",
    "haskeline",
    "utf8-string",
    "zip-archive",
    "SHA",
    "filelock",
    "edit-distance",
    "http-client",
    "http-client-tls",
    "http-types",
    "HTTP",
    "network",
    "ansi-terminal",
    "ansi-wl-pprint",
    "scientific",
    "vector",
    "unordered-containers",
    "language-glsl",
    "snap-core",
    "snap-server",
    "crypton",
];

/// A solver scenario, as `scenarios/<name>.json` and each entry of
/// `generated.json` hold it.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Scenario {
    /// `universes/<name>.json`, whose registry and releases come first.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    universe: Option<String>,
    /// Each package's versions, newest first.
    #[serde(default)]
    registry: BTreeMap<String, Vec<String>>,
    /// Each release's metadata, by `<name>@<version>`. A release the
    /// registry lists without metadata isn't in the package cache.
    #[serde(default)]
    releases: BTreeMap<String, Release>,
    project: Project,
    /// The package to add, as `install` does.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    add: Option<String>,
}

/// A registry and the metadata of its releases.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Universe {
    registry: BTreeMap<String, Vec<String>>,
    releases: BTreeMap<String, Release>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Release {
    /// Its `elm-version`.
    elm: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    deps: BTreeMap<String, String>,
    /// Whether its sources are in the cache; offline, upstream skips a
    /// release without them.
    #[serde(default = "yes", skip_serializing_if = "is_yes")]
    src: bool,
    /// Whether its `elm.json` is unreadable.
    #[serde(default, skip_serializing_if = "is_no")]
    broken: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "kebab-case", deny_unknown_fields)]
enum Project {
    #[serde(rename_all = "kebab-case")]
    Application {
        /// Its `elm-version`; the Compat's own by default.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        elm: Option<String>,
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        direct: BTreeMap<String, String>,
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        indirect: BTreeMap<String, String>,
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        test_direct: BTreeMap<String, String>,
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        test_indirect: BTreeMap<String, String>,
    },
    #[serde(rename_all = "kebab-case")]
    Package {
        elm: String,
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        deps: BTreeMap<String, String>,
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        test_deps: BTreeMap<String, String>,
    },
}

impl Default for Project {
    fn default() -> Project {
        Project::Package {
            elm: String::new(),
            deps: BTreeMap::new(),
            test_deps: BTreeMap::new(),
        }
    }
}

fn yes() -> bool {
    true
}

#[expect(
    clippy::trivially_copy_pass_by_ref,
    reason = "serde's `skip_serializing_if` passes the field by reference"
)]
fn is_yes(b: &bool) -> bool {
    *b
}

#[expect(
    clippy::trivially_copy_pass_by_ref,
    reason = "serde's `skip_serializing_if` passes the field by reference"
)]
fn is_no(b: &bool) -> bool {
    !*b
}

/// A scenario, with the registry and releases it sees.
struct Resolved {
    name: String,
    universe: Universe,
    scenario: Scenario,
}

/// `cargo xtask oracle solver`.
pub(crate) fn run(root: &Path, refs: &References) -> Result<()> {
    let suite = root.join(SUITE);
    let made = generate();
    fs::write(suite.join("generated.json"), render_generated(&made)?)?;
    let mut scenarios = load_scenarios(&suite)?;
    for (name, scenario) in made {
        ensure!(
            !scenarios.contains_key(&name),
            "scenarios/{name}.json shadows a generated scenario"
        );
        scenarios.insert(name, scenario);
    }
    let scenarios: Vec<Resolved> = (scenarios.into_iter())
        .map(|(name, scenario)| {
            let universe =
                resolve(&suite, &scenario).with_context(|| format!("scenario {name}"))?;
            Ok(Resolved {
                name,
                universe,
                scenario,
            })
        })
        .collect::<Result<_>>()?;
    let oracles = build(root, refs)?;
    let sandboxes = cache_dir()?.join("sandboxes").join("solver");
    let jobs: Vec<(&Resolved, &(Compat, PathBuf))> = (scenarios.iter())
        .flat_map(|s| oracles.iter().map(move |oracle| (s, oracle)))
        .collect();
    let results = parallel_on(cores(), &jobs, |&(s, (compat, oracle))| {
        let dir = sandboxes.join(compat.as_str()).join(&s.name);
        solve(oracle, *compat, &dir, &s.universe, &s.scenario)
            .with_context(|| format!("solver {compat} on {}", s.name))
    });
    let mut out = format!(
        "{}\n#\n# Each case is `<scenario>@<compat>`, then what upstream's solver answered.\n",
        generated("solver")
    );
    for (&(s, (compat, _)), result) in jobs.iter().zip(results) {
        write!(out, "\n== {}@{compat}\n{}", s.name, result?)?;
    }
    fs::write(suite.join("oracle.txt"), out)?;
    println!(
        "recorded {} scenarios for {} solvers",
        scenarios.len(),
        oracles.len()
    );
    Ok(())
}

/// The hand-written scenarios, by name.
fn load_scenarios(suite: &Path) -> Result<BTreeMap<String, Scenario>> {
    let dir = suite.join("scenarios");
    let mut scenarios = BTreeMap::new();
    for entry in fs::read_dir(&dir).with_context(|| format!("reading {}", dir.display()))? {
        let path = entry?.path();
        let Some(name) = path
            .file_name()
            .and_then(|n| n.to_str())
            .and_then(|n| n.strip_suffix(".json"))
        else {
            continue;
        };
        let text = fs::read_to_string(&path)?;
        let scenario: Scenario =
            serde_json::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
        scenarios.insert(name.to_owned(), scenario);
    }
    Ok(scenarios)
}

/// The scenario's registry and releases, over its universe's.
fn resolve(suite: &Path, scenario: &Scenario) -> Result<Universe> {
    let mut universe = match &scenario.universe {
        Some(name) => {
            let path = suite.join("universes").join(format!("{name}.json"));
            let text =
                fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
            serde_json::from_str(&text).with_context(|| format!("parsing {}", path.display()))?
        }
        None => Universe::default(),
    };
    universe.registry.extend(scenario.registry.clone());
    universe.releases.extend(scenario.releases.clone());
    for (name, versions) in &universe.registry {
        ensure!(!versions.is_empty(), "{name} has no versions");
        let parsed = (versions.iter())
            .map(|v| {
                v.parse()
                    .ok()
                    .with_context(|| format!("`{v}` is not a version"))
            })
            .collect::<Result<Vec<Version>>>()?;
        ensure!(
            parsed.windows(2).all(|w| w[0] > w[1]),
            "{name}'s versions are not newest first"
        );
    }
    for key in universe.releases.keys() {
        let (name, version) = key
            .split_once('@')
            .with_context(|| format!("`{key}` is not `<name>@<version>`"))?;
        ensure!(
            universe
                .registry
                .get(name)
                .is_some_and(|vs| vs.iter().any(|v| v == version)),
            "release {key} is not in the registry"
        );
    }
    Ok(universe)
}

/// Builds each Compat's oracle unless it is up to date.
fn build(root: &Path, refs: &References) -> Result<Vec<(Compat, PathBuf)>> {
    let ghc = ghc_version()?;
    let db = package_db(&ghc)?;
    let main = root.join("oracles/solver/Main.hs");
    let mut oracles = Vec::new();
    for compat in compat::compats(refs)? {
        let recipe = Recipe::of(compat);
        let pin = (refs.source.iter())
            .find(|s| s.name == recipe.sources)
            .with_context(|| format!("reference/{} is not pinned", recipe.sources))?;
        let sources = root.join("reference").join(recipe.sources);
        ensure!(
            sources.join("compiler/src").is_dir(),
            "{} is missing; run `cargo xtask refs`",
            sources.display()
        );
        let shim = root.join("oracles/solver").join(recipe.shim);
        let dir = oracles_dir()?.join(format!("solver-{compat}"));
        let exe = dir.join(format!("solver-oracle{}", std::env::consts::EXE_SUFFIX));
        let stamp = nomi_testkit::sha256_hex(
            format!(
                "{ghc}\n{}\n{compat}\n{}\n{}",
                pin.commit,
                nomi_testkit::sha256_hex(&fs::read(&main)?),
                nomi_testkit::sha256_hex(&fs::read(shim.join("Compat.hs"))?),
            )
            .as_bytes(),
        );
        let stamp_path = dir.join("stamp");
        if exe.exists() && download::stamped(&stamp_path, &stamp) {
            eprintln!("  solver oracle {compat} is built");
            oracles.push((compat, exe));
            continue;
        }
        eprintln!(
            "  building the solver oracle {compat} from reference/{}",
            recipe.sources
        );
        let paths = dir.join("paths");
        fs::create_dir_all(&paths)?;
        let Version {
            major,
            minor,
            patch,
        } = Version::compiler(compat);
        fs::write(
            paths.join("Paths_elm.hs"),
            format!(
                "-- Generated by `cargo xtask oracle solver`: the release the oracle is.\n\
                 module Paths_elm (version) where\n\
                 import Data.Version (Version, makeVersion)\n\
                 version :: Version\n\
                 version = makeVersion [{major},{minor},{patch}]\n"
            ),
        )?;
        let status = ghc_command(&db, &sources, &[&shim, &paths])
            .arg("-outputdir")
            .arg(dir.join("out"))
            .arg("-o")
            .arg(&exe)
            .arg(&main)
            .status()
            .context("running ghc")?;
        ensure!(
            status.success(),
            "ghc failed to build the solver oracle {compat}"
        );
        download::write_atomic(&stamp_path, stamp.as_bytes())?;
        oracles.push((compat, exe));
    }
    Ok(oracles)
}

/// GHC making a program of upstream's `sources` with the packages of the
/// database `db`, finding modules in `more` too.
fn ghc_command(db: &Path, sources: &Path, more: &[&Path]) -> Command {
    let mut command = Command::new("ghc");
    command
        .args(["--make", "-O1", "-j", "-v0", "-package-db"])
        .arg(db)
        .args(["-hide-all-packages", "-XHaskell2010"]);
    for package in GHC_PACKAGES {
        command.args(["-package", package]);
    }
    for dir in [
        "compiler/src",
        "builder/src",
        "src/utils",
        "terminal/impl",
        "terminal/src",
    ] {
        command.arg(format!("-i{}", sources.join(dir).display()));
    }
    for dir in more {
        command.arg(format!("-i{}", dir.display()));
    }
    command
}

fn ghc_version() -> Result<String> {
    let output = Command::new("ghc")
        .arg("--numeric-version")
        .output()
        .context("running ghc; the solver oracle needs GHC on PATH")?;
    ensure!(output.status.success(), "ghc --numeric-version failed");
    Ok(String::from_utf8(output.stdout)?.trim().to_owned())
}

/// `$NOMI_GHC_PACKAGE_DB`, or the cabal store's database for `ghc`.
fn package_db(ghc: &str) -> Result<PathBuf> {
    if let Some(db) = std::env::var_os("NOMI_GHC_PACKAGE_DB") {
        return Ok(PathBuf::from(db));
    }
    let home = std::env::var_os("HOME").context("HOME is not set")?;
    let store = PathBuf::from(home).join(".local/state/cabal/store");
    let prefix = format!("ghc-{ghc}");
    let mut found = fs::read_dir(&store)
        .with_context(|| format!("reading {}", store.display()))?
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.file_name().to_string_lossy().starts_with(&prefix))
        .map(|entry| entry.path().join("package.db"))
        .filter(|db| db.is_dir())
        .collect::<Vec<_>>();
    found.sort();
    match found.into_iter().next() {
        Some(db) => Ok(db),
        None => bail!(
            "no cabal store for GHC {ghc} in {}; run `cabal build --only-dependencies` in \
             reference/compiler, or set NOMI_GHC_PACKAGE_DB",
            store.display()
        ),
    }
}

/// Runs the oracle on a fresh sandbox holding the scenario.
fn solve(
    oracle: &Path,
    compat: Compat,
    dir: &Path,
    universe: &Universe,
    scenario: &Scenario,
) -> Result<String> {
    remove_tree(dir)?;
    let home = dir.join("home");
    let packages = home.join(compat.as_str()).join("packages");
    for (key, release) in &universe.releases {
        let (name, vsn) = key.split_once('@').expect("checked by resolve");
        let release_dir = packages.join(name).join(vsn);
        fs::create_dir_all(&release_dir)?;
        if release.src {
            fs::create_dir_all(release_dir.join("src"))?;
        }
        let text = if release.broken {
            "{}".to_owned()
        } else {
            json!({
                "type": "package",
                "name": name,
                "summary": "fixture",
                "license": "BSD-3-Clause",
                "version": vsn,
                "exposed-modules": ["Fixture"],
                "elm-version": release.elm,
                "dependencies": release.deps,
                "test-dependencies": {},
            })
            .to_string()
        };
        fs::write(release_dir.join("elm.json"), text)?;
    }
    let mut registry = String::new();
    for (name, versions) in &universe.registry {
        writeln!(registry, "{name} {}", versions.join(" "))?;
    }
    fs::write(dir.join("registry"), registry)?;
    let project = dir.join("project");
    fs::create_dir_all(project.join("src"))?;
    fs::write(
        project.join("elm.json"),
        outline(&scenario.project, compat).to_string(),
    )?;
    let mut command = Command::new(oracle);
    command
        .arg(dir.join("registry"))
        .args(&scenario.add)
        .current_dir(&project)
        .env("ELM_HOME", &home)
        .stdin(Stdio::null());
    let output = offline(&mut command)
        .output()
        .with_context(|| format!("running {}", oracle.display()))?;
    ensure!(
        output.status.success(),
        "the oracle failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(String::from_utf8(output.stdout)?)
}

/// The project's `elm.json`.
fn outline(project: &Project, compat: Compat) -> serde_json::Value {
    match project {
        Project::Application {
            elm,
            direct,
            indirect,
            test_direct,
            test_indirect,
        } => json!({
            "type": "application",
            "source-directories": ["src"],
            "elm-version": elm.as_deref().unwrap_or(compat.as_str()),
            "dependencies": { "direct": direct, "indirect": indirect },
            "test-dependencies": { "direct": test_direct, "indirect": test_indirect },
        }),
        Project::Package {
            elm,
            deps,
            test_deps,
        } => json!({
            "type": "package",
            "name": "nomi/root",
            "summary": "fixture",
            "license": "BSD-3-Clause",
            "version": "1.0.0",
            "exposed-modules": ["Main"],
            "elm-version": elm,
            "dependencies": deps,
            "test-dependencies": test_deps,
        }),
    }
}

/// `generated.json`: one scenario a line, so a change diffs by scenario.
fn render_generated(generated: &BTreeMap<String, Scenario>) -> Result<String> {
    let mut out = String::from("{\n");
    for (i, (name, scenario)) in generated.iter().enumerate() {
        let comma = if i + 1 < generated.len() { "," } else { "" };
        writeln!(
            out,
            "{}: {}{comma}",
            serde_json::to_string(name)?,
            serde_json::to_string(scenario)?
        )?;
    }
    out.push_str("}\n");
    Ok(out)
}

/// `SplitMix64`, so the scenarios never change with a dependency.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    /// A number below `n`.
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }

    /// True `percent` times in a hundred.
    fn chance(&mut self, percent: usize) -> bool {
        self.below(100) < percent
    }

    fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.below(items.len())]
    }
}

/// The versions a generated package draws from.
const POOL: &[&str] = &[
    "1.0.0", "1.0.1", "1.1.0", "1.2.0", "2.0.0", "2.0.1", "2.1.0", "3.0.0",
];

/// Small registries whose packages depend on one another, each with a
/// project to verify or a package to add. Most releases are fine; some
/// want another elm, lack sources or metadata, so the search backtracks.
fn generate() -> BTreeMap<String, Scenario> {
    let mut rng = Rng(SEED);
    (0..GENERATED)
        .map(|i| (format!("gen-{i:03}"), generate_one(&mut rng)))
        .collect()
}

fn generate_one(rng: &mut Rng) -> Scenario {
    let mut scenario = Scenario {
        universe: Some("core".to_owned()),
        ..Scenario::default()
    };
    let names: Vec<String> = ["alpha", "beta", "gamma", "delta", "epsilon", "zeta"]
        [..3 + rng.below(4)]
        .iter()
        .map(|n| format!("gen/{n}"))
        .collect();
    let mut versions: Vec<Vec<&str>> = Vec::new();
    for _ in &names {
        let mut chosen: Vec<&str> = POOL.iter().copied().filter(|_| rng.chance(40)).collect();
        if chosen.is_empty() {
            chosen.push(rng.pick(POOL));
        }
        chosen.reverse();
        versions.push(chosen);
    }
    for (i, name) in names.iter().enumerate() {
        scenario.registry.insert(
            name.clone(),
            versions[i].iter().map(|v| (*v).to_owned()).collect(),
        );
        for &vsn in &versions[i] {
            // Missing metadata: offline, upstream skips the release.
            if rng.chance(4) {
                continue;
            }
            let release = release(rng, &names[..i], &versions);
            scenario.releases.insert(format!("{name}@{vsn}"), release);
        }
    }
    // An unreadable release ends the search wherever it's met, so only a
    // few scenarios have one.
    if rng.chance(6) {
        let keys: Vec<String> = scenario.releases.keys().cloned().collect();
        if !keys.is_empty() {
            let key = rng.pick(&keys).clone();
            scenario.releases.get_mut(&key).expect("a release").broken = true;
        }
    }
    let (project, add) = if rng.chance(40) {
        (package(rng, &names, &versions), 50)
    } else {
        (application(rng, &names, &versions), 75)
    };
    if rng.chance(add) {
        scenario.add = Some(addition(rng, &names, &project));
    }
    scenario.project = project;
    scenario
}

/// A release of a package after `earlier` in the list, which it may
/// depend on, as published packages never form a cycle.
fn release(rng: &mut Rng, earlier: &[String], versions: &[Vec<&str>]) -> Release {
    let mut deps = BTreeMap::new();
    deps.insert("elm/core".to_owned(), "1.0.0 <= v < 2.0.0".to_owned());
    if rng.chance(30) {
        deps.insert("elm/json".to_owned(), json_constraint(rng));
    }
    for (j, name) in earlier.iter().enumerate() {
        if rng.chance(35) {
            deps.insert(name.clone(), constraint(rng, &versions[j]));
        }
    }
    let elm = match rng.below(100) {
        0..=2 => "0.18.0 <= v < 0.19.0",
        3..=7 => "0.19.2 <= v < 0.20.0",
        8..=11 => "0.19.0 <= v < 0.19.2",
        _ => "0.19.0 <= v < 0.20.0",
    };
    Release {
        elm: elm.to_owned(),
        deps,
        src: !rng.chance(3),
        broken: false,
    }
}

/// A package depending on some of `names`, a few only for its tests.
fn package(rng: &mut Rng, names: &[String], versions: &[Vec<&str>]) -> Project {
    let mut deps = BTreeMap::new();
    deps.insert("elm/core".to_owned(), "1.0.0 <= v < 2.0.0".to_owned());
    let mut test_deps = BTreeMap::new();
    for (i, name) in names.iter().enumerate() {
        match rng.below(100) {
            0..=39 => {
                deps.insert(name.clone(), constraint(rng, &versions[i]));
            }
            40..=49 => {
                test_deps.insert(name.clone(), constraint(rng, &versions[i]));
            }
            _ => {}
        }
    }
    Project::Package {
        elm: "0.19.0 <= v < 0.20.0".to_owned(),
        deps,
        test_deps,
    }
}

/// An application depending on some of `names`, directly or not, some
/// only for its tests.
fn application(rng: &mut Rng, names: &[String], versions: &[Vec<&str>]) -> Project {
    let mut maps: [BTreeMap<String, String>; 4] = Default::default();
    maps[0].insert(
        "elm/core".to_owned(),
        rng.pick(&["1.0.5", "1.0.4"]).to_string(),
    );
    maps[0].insert(
        "elm/json".to_owned(),
        rng.pick(&["1.1.3", "1.1.2"]).to_string(),
    );
    for (i, name) in names.iter().enumerate() {
        let which = match rng.below(100) {
            0..=24 => 0,
            25..=39 => 1,
            40..=44 => 2,
            45..=47 => 3,
            _ => continue,
        };
        maps[which].insert(name.clone(), (*rng.pick(&versions[i])).to_owned());
    }
    let [direct, indirect, test_direct, test_indirect] = maps;
    Project::Application {
        elm: None,
        direct,
        indirect,
        test_direct,
        test_indirect,
    }
}

/// The package to add: mostly one of `names` the project doesn't have
/// yet, so `install` solves.
fn addition(rng: &mut Rng, names: &[String], project: &Project) -> String {
    let has = |name: &String| match project {
        Project::Application {
            direct,
            indirect,
            test_direct,
            test_indirect,
            ..
        } => [direct, indirect, test_direct, test_indirect]
            .iter()
            .any(|deps| deps.contains_key(name)),
        Project::Package {
            deps, test_deps, ..
        } => deps.contains_key(name) || test_deps.contains_key(name),
    };
    let new: Vec<&String> = names.iter().filter(|name| !has(name)).collect();
    if rng.chance(4) {
        "gen/unknown".to_owned()
    } else if new.is_empty() || rng.chance(15) {
        rng.pick(names).clone()
    } else {
        (*rng.pick(&new)).clone()
    }
}

/// A constraint on a package with these versions: usually up to the next
/// major from one of them, sometimes exact or wide.
fn constraint(rng: &mut Rng, versions: &[&str]) -> String {
    let lower: Version = rng.pick(versions).parse().expect("a pool version");
    let Version {
        major,
        minor,
        patch,
    } = lower;
    match rng.below(10) {
        // `a <= v <= a` is no constraint to upstream: its bounds must differ.
        0 => format!("{lower} <= v < {major}.{minor}.{}", patch + 1),
        1 => format!("{major}.{minor}.{patch} <= v < {major}.{}.0", minor + 1),
        2 => format!("1.0.0 <= v < {}.0.0", major + 2),
        _ => format!("{lower} <= v < {}.0.0", major + 1),
    }
}

fn json_constraint(rng: &mut Rng) -> String {
    rng.pick(&[
        "1.0.0 <= v < 2.0.0",
        "1.1.3 <= v < 2.0.0",
        "1.1.0 <= v < 2.0.0",
    ])
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_generated_scenarios_are_the_recorded_ones() {
        let recorded = include_str!("../../../crates/registry/tests/solver/generated.json");
        assert_eq!(render_generated(&generate()).unwrap(), recorded);
    }
}
