//! `cargo xtask corpus docs`: whether nomi documents every corpus package
//! as each elm did (plan 7.4a): the same `docs.json`, byte for byte, or
//! the same `--report=json` where elm rejected the package. The oracle is
//! `corpus/docs/oracle.tsv` (`cargo xtask oracle corpus-docs`). Each
//! package's dependencies are solved and downloaded as `corpus names`
//! does, so this runs by hand, never in CI. A package elm couldn't build
//! for want of a release the cache lacked is skipped.

use std::collections::{BTreeMap, HashMap};
use std::io;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use nomi_base::Compat;
use nomi_compiler::interface::Interface;
use nomi_engine::check::{self, Style};
use nomi_engine::console::{Console, Personality};
use nomi_engine::graph::Foreigns;
use nomi_engine::{Vfs, names};
use nomi_project::{ModuleName, Name, Outline};

use super::names::{Project, solve};
use super::{compat, parallel};
use crate::download::sha256;
use crate::oracle::corpus_docs::{self, EXCLUDED};

pub fn check(root: &Path, only: Option<&str>) -> Result<()> {
    let oracle = load(root)?;
    let classification = compat::load(root)?;
    let mut matching = 0;
    let mut skipped: Vec<String> = Vec::new();
    let mut failures: Vec<(String, String)> = Vec::new();
    for v in &classification.compats {
        let c = (Compat::ALL.into_iter())
            .find(|c| c.to_string() == *v)
            .with_context(|| format!("nomi doesn't emulate elm {v}"))?;
        let solving = solve(root, &classification, c, v, only, false)?;
        // Dependencies without a solution stop elm before any module too.
        for (case, why) in solving.failures {
            let elm = oracle.get(&(package_case(&case), format!("elm {v}")));
            if elm.is_some_and(|cell| cell.starts_with("error ")) {
                matching += 1;
            } else {
                failures.push((case, why));
            }
        }
        skipped.extend(solving.unavailable);
        let mut memo = HashMap::new();
        for chunk in solving.solved.chunks(64) {
            let mut ready = Vec::new();
            for (project, sources) in chunk {
                let key = package_case(&project.case);
                let Some(expected) = oracle.get(&(key.clone(), format!("elm {v}"))) else {
                    skipped.push(format!("{}: not in the oracle", project.case));
                    continue;
                };
                if expected == EXCLUDED || expected.starts_with("error ") || expected == "crash" {
                    skipped.push(format!("{}: elm said {expected}", project.case));
                    continue;
                }
                match names::package_interfaces_with(c, sources, &mut memo) {
                    Ok(interfaces) => ready.push((project, sources, interfaces, expected)),
                    Err(why) => {
                        let why: String = why.chars().take(300).collect();
                        failures.push((project.case.clone(), why));
                    }
                }
            }
            let results = parallel(&ready, |(project, sources, interfaces, _)| {
                document(c, project, sources, interfaces)
            });
            for ((project, _, _, expected), result) in ready.iter().zip(results) {
                match result {
                    Ok(cell) if &cell == *expected => matching += 1,
                    Ok(cell) => failures.push((
                        project.case.clone(),
                        format!("nomi: {cell}\n  elm:  {expected}"),
                    )),
                    Err(why) => failures.push((project.case.clone(), why)),
                }
            }
            eprintln!("  {c}: {matching} matching, {} failing", failures.len());
        }
    }
    for why in &skipped {
        eprintln!("skipped: {why}");
    }
    for (case, why) in &failures {
        eprintln!("FAIL {case}: {why}");
    }
    println!(
        "docs: {matching} packages documented as elm documents them, {} not, {} skipped",
        failures.len(),
        skipped.len()
    );
    if !failures.is_empty() {
        bail!("{} packages differ", failures.len());
    }
    Ok(())
}

/// The oracle's cells, by package case and elm.
fn load(root: &Path) -> Result<BTreeMap<(String, String), String>> {
    let path = root.join(corpus_docs::FILE);
    let text = std::fs::read_to_string(&path).with_context(|| {
        format!(
            "reading {}; run `cargo xtask oracle corpus-docs`",
            path.display()
        )
    })?;
    let mut lines = text.lines().filter(|l| !l.starts_with('#'));
    let header: Vec<&str> = lines
        .next()
        .context("an empty oracle")?
        .split('\t')
        .collect();
    let mut cells = BTreeMap::new();
    for line in lines {
        let row: Vec<&str> = line.split('\t').collect();
        for (elm, cell) in header.iter().zip(&row).skip(1) {
            cells.insert((row[0].to_owned(), (*elm).to_owned()), (*cell).to_owned());
        }
    }
    Ok(cells)
}

/// `author/project@version` of a case `author/project@version on 0.19.x`.
fn package_case(case: &str) -> String {
    case.split(" on ").next().unwrap_or(case).to_owned()
}

/// What `elm make --docs` makes of `project`, as an oracle cell.
fn document(
    compat: Compat,
    project: &Project,
    sources: &[(Name, PathBuf)],
    interfaces: &names::PackageInterfaces,
) -> Result<String, String> {
    let Outline::Pkg(pkg) = &project.outline else {
        return Err("not a package".to_owned());
    };
    let direct: Vec<Name> = pkg.deps.keys().cloned().collect();
    let dirs: BTreeMap<&Name, &PathBuf> = sources.iter().map(|(n, d)| (n, d)).collect();
    let mut exposing = Vec::new();
    for name in &direct {
        let dir = dirs
            .get(name)
            .ok_or_else(|| format!("{name} wasn't downloaded"))?;
        let bytes = std::fs::read(dir.join("elm.json")).map_err(|e| e.to_string())?;
        let Ok(Outline::Pkg(dep)) = Outline::decode(&bytes, compat) else {
            return Err(format!("{name}'s elm.json"));
        };
        exposing.push((
            name.clone(),
            dep.exposed.modules().cloned().collect::<Vec<ModuleName>>(),
        ));
    }
    let visible: HashMap<String, Interface> = names::visible(interfaces, &direct);
    let root = std::fs::canonicalize(&project.root).map_err(|e| e.to_string())?;
    let env = check::env_of(&root, &project.outline, compat, Foreigns::new(exposing))
        .map_err(|r| r.title.to_string())?
        .with_cwd(&root);
    let (checked, docs) = check::docs(
        &Vfs::new().snapshot(),
        &env,
        &project.outline,
        Personality::Elm,
        &visible,
    );
    if let Some(docs) = docs {
        return Ok(format!("docs {}", sha256(docs.as_bytes())));
    }
    let mut console = Captured::default();
    check::show(&mut console, &root, &checked, compat, Style::Json);
    let root = root.to_str().ok_or("a path that isn't UTF-8")?;
    let report = console.err.trim_end().replace(root, "$ROOT");
    Ok(corpus_docs::cell(&report))
}

#[derive(Default)]
struct Captured {
    err: String,
}

impl Console for Captured {
    fn out(&mut self, _: &str) {}

    fn err(&mut self, text: &str) {
        self.err.push_str(text);
    }

    fn read_line(&mut self) -> io::Result<Option<String>> {
        Ok(None)
    }

    fn read_all(&mut self) -> io::Result<Vec<u8>> {
        Ok(Vec::new())
    }
}
