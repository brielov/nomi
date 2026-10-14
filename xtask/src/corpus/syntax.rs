//! `cargo xtask corpus syntax`: the corpus side of the syntax ratchet.
//! Each corpus project is a case. It passes when nomi accepts and rejects
//! its modules as every tool in `corpus/syntax/oracle.tsv` did, the table
//! `cargo xtask oracle syntax` stores, and its tree gives back every
//! module's text.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail, ensure};
use nomi_syntax::{Source, Tool};
use nomi_testkit::ratchet::Suite;
use nomi_testkit::sha256_hex as sha256;

use super::{apps, corpus_dir, files_below, packages, parallel};

/// The suite's directory: the oracle's table and `passing.txt`.
pub(crate) const DIR: &str = "corpus/syntax";

/// The cell of a tool that crashed, which gives no verdict to match.
pub(crate) const CRASH: &str = "crash";

/// One corpus module.
#[derive(Debug)]
pub(crate) struct Module {
    /// The project: `author/project@version`, or `app/<name>`.
    pub case: String,
    /// Relative to the project.
    pub path: String,
    pub file: PathBuf,
    pub source: Source,
}

/// Every module of every pinned package and app, in a stable order.
pub(crate) fn modules(root: &Path) -> Result<Vec<Module>> {
    let corpus = corpus_dir()?;
    let mut modules = Vec::new();
    for package in packages::load(root)?.packages {
        let dir = packages::package_dir(&corpus, &package);
        ensure!(
            dir.join(packages::STAMP).exists(),
            "{} {} is not fetched; run `cargo xtask corpus fetch`",
            package.name,
            package.version
        );
        // Upstream's `Pkg.isKernel`.
        let kernel =
            package.name.starts_with("elm/") || package.name.starts_with("elm-explorations/");
        let source = if kernel {
            Source::Kernel
        } else {
            Source::Package
        };
        let case = format!("{}@{}", package.name, package.version);
        push_modules(&mut modules, &case, &dir, source)?;
    }
    for app in apps::load(root)?.app {
        let dir = apps::app_dir(&corpus, &app);
        ensure!(
            dir.join(".git").exists(),
            "app {} is not fetched; run `cargo xtask corpus fetch`",
            app.name
        );
        push_modules(
            &mut modules,
            &format!("app/{}", app.name),
            &dir,
            Source::Application,
        )?;
    }
    Ok(modules)
}

fn push_modules(modules: &mut Vec<Module>, case: &str, dir: &Path, source: Source) -> Result<()> {
    for path in files_below(dir)? {
        let skipped = path.starts_with(".git/")
            || path
                .split('/')
                .any(|part| part == "elm-stuff" || part == "node_modules");
        if path.ends_with(".elm") && !skipped {
            modules.push(Module {
                case: case.to_owned(),
                file: dir.join(&path),
                path,
                source,
            });
        }
    }
    Ok(())
}

/// The SHA-256 of a `sha256sum`-style listing of every module, so the
/// stored table can tell when the corpus changed under it.
pub(crate) fn digest(modules: &[Module]) -> Result<String> {
    let mut listing = String::new();
    for m in modules {
        let bytes =
            std::fs::read(&m.file).with_context(|| format!("reading {}", m.file.display()))?;
        listing.push_str(&format!("{}  {}/{}\n", sha256(&bytes), m.case, m.path));
    }
    Ok(sha256(listing.as_bytes()))
}

/// The tools of the table's columns, and each module some tool rejects.
struct Table {
    tools: Vec<String>,
    modules: usize,
    sources: String,
    rejected: BTreeMap<(String, String), Vec<Option<String>>>,
}

fn parse_table(text: &str) -> Result<Table> {
    let mut lines = text.lines().filter(|l| !l.starts_with("# @generated"));
    let summary = lines.next().context("the table is empty")?;
    let words: Vec<&str> = summary.split_whitespace().collect();
    let ["#", "modules", modules, "sources", sources] = words[..] else {
        bail!("malformed summary `{summary}`");
    };
    let columns: Vec<&str> = lines
        .next()
        .context("no column header")?
        .split('\t')
        .collect();
    ensure!(
        columns.starts_with(&["case", "path"]),
        "malformed column header"
    );
    let tools: Vec<String> = columns[2..].iter().map(|c| c.to_string()).collect();
    let mut rejected = BTreeMap::new();
    for line in lines {
        let cells: Vec<&str> = line.split('\t').collect();
        ensure!(cells.len() == columns.len(), "malformed row `{line}`");
        let outcomes = cells[2..]
            .iter()
            .map(|c| (*c != "ok").then(|| c.to_string()))
            .collect();
        rejected.insert((cells[0].to_owned(), cells[1].to_owned()), outcomes);
    }
    Ok(Table {
        tools,
        modules: modules.parse()?,
        sources: sources.to_owned(),
        rejected,
    })
}

/// The nomi tool a column stands for.
fn tool(column: &str, source: Source) -> Result<Tool> {
    match column {
        "elm-format" => Ok(Tool::ElmFormat),
        _ => match column.strip_prefix("elm ") {
            Some(version) => Ok(Tool::Elm(source, version.parse()?)),
            None => bail!("unknown column `{column}`"),
        },
    }
}

/// Whether nomi reads `text` as `tool` does.
fn accepts(text: &[u8], tool: Tool) -> bool {
    let Ok(text) = std::str::from_utf8(text) else {
        return false;
    };
    let parse = nomi_syntax::parse_module(text);
    parse.errors().is_empty() && nomi_syntax::validate(&parse, tool).is_empty()
}

pub(crate) fn check(root: &Path, record: bool) -> Result<()> {
    let dir = root.join(DIR);
    let text = std::fs::read_to_string(dir.join("oracle.tsv"))
        .context("reading corpus/syntax/oracle.tsv; run `cargo xtask oracle syntax corpus`")?;
    let table = parse_table(&text)?;
    let modules = modules(root)?;
    ensure!(
        table.modules == modules.len() && table.sources == digest(&modules)?,
        "the corpus changed since oracle.tsv was made; run `cargo xtask oracle syntax corpus`"
    );
    let mismatches = parallel(&modules, |m| -> Result<Vec<String>> {
        let text = std::fs::read(&m.file)?;
        let expected = table.rejected.get(&(m.case.clone(), m.path.clone()));
        let mut mismatches = Vec::new();
        if let Ok(text) = std::str::from_utf8(&text)
            && nomi_syntax::parse_module(text).syntax().to_string() != text
        {
            mismatches.push(format!("{}: the tree does not give back the text", m.path));
        }
        for (i, column) in table.tools.iter().enumerate() {
            let rejection = expected.and_then(|row| row[i].as_ref());
            if rejection.is_some_and(|why| why == CRASH) {
                continue;
            }
            if accepts(&text, tool(column, m.source)?) == rejection.is_some() {
                mismatches.push(match rejection {
                    Some(why) => format!("{}: {column} rejects ({why}), nomi accepts", m.path),
                    None => format!("{}: {column} accepts, nomi rejects", m.path),
                });
            }
        }
        Ok(mismatches)
    });
    let mut cases: BTreeMap<&str, Vec<String>> = BTreeMap::new();
    for (m, found) in modules.iter().zip(mismatches) {
        cases.entry(&m.case).or_default().extend(found?);
    }
    let outcomes = cases.into_iter().map(|(case, mismatches)| {
        let result = match mismatches.as_slice() {
            [] => Ok(()),
            [only] => Err(only.clone()),
            [first, rest @ ..] => Err(format!("{first} (and {} more)", rest.len())),
        };
        (case.to_owned(), result)
    });
    Suite::open(dir)
        .run_by("cargo xtask corpus syntax --record", record)
        .finish(outcomes);
    Ok(())
}

#[cfg(test)]
mod tests {
    use nomi_base::Compat;

    use super::*;

    #[test]
    fn tables_parse() {
        let text = "# @generated by `cargo xtask oracle syntax`; regenerate, never edit.\n\
                    # modules 3 sources abc\n\
                    case\tpath\telm 0.19.1\telm-format\n\
                    a/b@1.0.0\tsrc/A.elm\tUNFINISHED LIST 1:2-1:2\tok\n";
        let table = parse_table(text).unwrap();
        assert_eq!(table.tools, ["elm 0.19.1", "elm-format"]);
        assert_eq!((table.modules, table.sources.as_str()), (3, "abc"));
        let row = &table.rejected[&("a/b@1.0.0".to_owned(), "src/A.elm".to_owned())];
        assert_eq!(row, &[Some("UNFINISHED LIST 1:2-1:2".to_owned()), None]);
    }

    #[test]
    fn columns_name_tools() {
        assert_eq!(
            tool("elm 0.19.3", Source::Kernel).unwrap(),
            Tool::Elm(Source::Kernel, Compat::V0_19_3)
        );
        assert!(tool("elm 0.19.0", Source::Kernel).is_err());
        assert_eq!(tool("elm-format", Source::Kernel).unwrap(), Tool::ElmFormat);
        assert!(tool("elm-review", Source::Kernel).is_err());
    }
}
