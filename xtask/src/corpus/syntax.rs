//! `cargo xtask corpus syntax`: the corpus side of the syntax ratchet.
//! Each corpus project is a case. It passes when nomi accepts and rejects
//! its modules as every tool in `corpus/syntax/oracle.tsv` did, the table
//! `cargo xtask oracle syntax` stores, and its tree gives back every
//! module's text.

use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use anyhow::{Context, Result, bail, ensure};
use nomi_base::Compat;
use nomi_syntax::{Source, Tool};
use nomi_testkit::ratchet::Suite;
use nomi_testkit::{files_below, release_column, sha256_hex as sha256};
use sha2::{Digest as _, Sha256};

use super::{apps, corpus_dir, is_module, packages, parallel};

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

/// What a tool made of one module's syntax, as the tables write it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Outcome {
    /// `ok`.
    Accepted,
    /// The title and region elm reports, or where elm-format stopped.
    Rejected(String),
    /// The tool died without a verdict; the ratchets skip it.
    Crashed,
}

impl fmt::Display for Outcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Outcome::Accepted => "ok",
            Outcome::Rejected(detail) => detail,
            Outcome::Crashed => CRASH,
        })
    }
}

impl From<&str> for Outcome {
    fn from(cell: &str) -> Outcome {
        match cell {
            "ok" => Outcome::Accepted,
            CRASH => Outcome::Crashed,
            detail => Outcome::Rejected(detail.to_owned()),
        }
    }
}

/// What a table of the corpus's modules was made from, as its summary
/// line says: `# modules <count> sources <digest>`, the digest of a
/// `sha256sum`-style listing of the modules, so a table can tell when
/// the corpus changed under it.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Sources {
    modules: usize,
    digest: String,
}

impl Sources {
    /// What `modules` are now.
    pub(crate) fn of(modules: &[Module]) -> Result<Sources> {
        let mut listing = Sha256::new();
        for m in modules {
            let bytes =
                fs::read(&m.file).with_context(|| format!("reading {}", m.file.display()))?;
            listing.update(format!("{}  {}/{}\n", sha256(&bytes), m.case, m.path));
        }
        Ok(Sources {
            modules: modules.len(),
            digest: nomi_base::hex(&listing.finalize()),
        })
    }

    /// Fails unless the table `cargo xtask oracle <oracle> corpus` made
    /// from these sources was made from `modules` as they are now.
    pub(crate) fn ensure_current(&self, modules: &[Module], oracle: &str) -> Result<()> {
        ensure!(
            *self == Sources::of(modules)?,
            "the corpus changed since oracle.tsv was made; run `cargo xtask oracle {oracle} corpus`"
        );
        Ok(())
    }
}

impl fmt::Display for Sources {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "# modules {} sources {}", self.modules, self.digest)
    }
}

impl FromStr for Sources {
    type Err = anyhow::Error;

    fn from_str(line: &str) -> Result<Sources> {
        let words: Vec<&str> = line.split_whitespace().collect();
        let ["#", "modules", modules, "sources", digest] = words[..] else {
            bail!("malformed summary `{line}`");
        };
        Ok(Sources {
            modules: modules.parse()?,
            digest: digest.to_owned(),
        })
    }
}

/// Every module of every pinned package and app, in a stable order.
pub(crate) fn modules(root: &Path) -> Result<Vec<Module>> {
    let corpus = corpus_dir()?;
    let mut modules = Vec::new();
    for package in packages::load(root)?.packages {
        let dir = packages::fetched_dir(&corpus, &package)?;
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
    for path in files_below(dir)?.into_iter().filter(|p| is_module(p)) {
        modules.push(Module {
            case: case.to_owned(),
            file: dir.join(&path),
            path,
            source,
        });
    }
    Ok(())
}

/// A column of the table: an elm release's verdicts, or elm-format's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Column {
    Elm(Compat),
    ElmFormat,
}

impl Column {
    /// The nomi tool that reads a module of a `source` project as the
    /// column's tool does.
    fn tool(self, source: Source) -> Tool {
        match self {
            Column::Elm(compat) => Tool::Elm(source, compat),
            Column::ElmFormat => Tool::ElmFormat,
        }
    }
}

impl fmt::Display for Column {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Column::Elm(compat) => write!(f, "elm {compat}"),
            Column::ElmFormat => f.write_str("elm-format"),
        }
    }
}

impl FromStr for Column {
    type Err = anyhow::Error;

    fn from_str(header: &str) -> Result<Column> {
        match header {
            "elm-format" => Ok(Column::ElmFormat),
            _ => match release_column(header) {
                Some(compat) => Ok(Column::Elm(compat)),
                None => bail!("unknown column `{header}`"),
            },
        }
    }
}

/// The table: what it was made from, its columns, and each module some
/// tool rejects, by case and path.
struct Table<'a> {
    sources: Sources,
    columns: Vec<Column>,
    rejected: BTreeMap<(&'a str, &'a str), Vec<Outcome>>,
}

fn parse_table(text: &str) -> Result<Table<'_>> {
    let mut lines = text.lines().filter(|l| !l.starts_with("# @generated"));
    let sources = lines.next().context("the table is empty")?.parse()?;
    let header = lines.next().context("no column header")?;
    let Some(columns) = header.strip_prefix("case\tpath\t") else {
        bail!("malformed column header");
    };
    let columns = columns
        .split('\t')
        .map(str::parse)
        .collect::<Result<Vec<_>>>()?;
    let mut rejected = BTreeMap::new();
    for line in lines {
        let cells: Vec<&str> = line.split('\t').collect();
        let [case, path, outcomes @ ..] = &cells[..] else {
            bail!("malformed row `{line}`");
        };
        ensure!(outcomes.len() == columns.len(), "malformed row `{line}`");
        let outcomes = outcomes.iter().map(|&cell| Outcome::from(cell)).collect();
        let fresh = rejected.insert((*case, *path), outcomes).is_none();
        ensure!(fresh, "{case} {path} is in the table twice");
    }
    Ok(Table {
        sources,
        columns,
        rejected,
    })
}

pub(crate) fn check(root: &Path, record: bool) -> Result<()> {
    let dir = root.join(DIR);
    let text = fs::read_to_string(dir.join("oracle.tsv"))
        .context("reading corpus/syntax/oracle.tsv; run `cargo xtask oracle syntax corpus`")?;
    let table = parse_table(&text)?;
    let modules = modules(root)?;
    table.sources.ensure_current(&modules, "syntax")?;
    let mismatches = parallel(&modules, |m| -> Result<Vec<String>> {
        let bytes = fs::read(&m.file)?;
        let rejected = table.rejected.get(&(m.case.as_str(), m.path.as_str()));
        let mut mismatches = Vec::new();
        let parsed = std::str::from_utf8(&bytes)
            .ok()
            .map(|text| (text, nomi_syntax::parse_module(text)));
        if let Some((text, parse)) = &parsed
            && parse.syntax().to_string() != *text
        {
            mismatches.push(format!("{}: the tree does not give back the text", m.path));
        }
        // Text that isn't UTF-8 is rejected.
        let accepts = |column: Column| {
            parsed.as_ref().is_some_and(|(_, parse)| {
                parse.errors().is_empty()
                    && nomi_syntax::validate(parse, column.tool(m.source)).is_empty()
            })
        };
        for (i, &column) in table.columns.iter().enumerate() {
            let outcome = rejected.map_or(&Outcome::Accepted, |row| &row[i]);
            let why = match outcome {
                Outcome::Accepted if !accepts(column) => "accepts, nomi rejects".to_owned(),
                Outcome::Rejected(why) if accepts(column) => {
                    format!("rejects ({why}), nomi accepts")
                }
                Outcome::Accepted | Outcome::Rejected(_) | Outcome::Crashed => continue,
            };
            mismatches.push(format!("{}: {column} {why}", m.path));
        }
        Ok(mismatches)
    });
    finish(
        &dir,
        "cargo xtask corpus syntax --record",
        record,
        &modules,
        mismatches,
    )
}

/// Finishes the ratchet in `dir`, run by `command`, over the corpus's
/// projects, with `found` what each of `modules` has wrong: a project
/// passes where none of its modules has anything.
pub(crate) fn finish<F: IntoIterator<Item = String>>(
    dir: &Path,
    command: &str,
    record: bool,
    modules: &[Module],
    found: Vec<Result<F>>,
) -> Result<()> {
    let mut cases: BTreeMap<&str, Vec<String>> = BTreeMap::new();
    for (m, found) in modules.iter().zip(found) {
        cases.entry(&m.case).or_default().extend(found?);
    }
    let outcomes = cases.into_iter().map(|(case, mismatches)| {
        let result = match &mismatches[..] {
            [] => Ok(()),
            [only] => Err(only.clone()),
            [first, rest @ ..] => Err(format!("{first} (and {} more)", rest.len())),
        };
        (case.to_owned(), result)
    });
    Suite::open(dir).run_by(command, record).finish(outcomes);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tables_parse() {
        let text = "# @generated by `cargo xtask oracle syntax`; regenerate, never edit.\n\
                    # modules 3 sources abc\n\
                    case\tpath\telm 0.19.1\telm-format\n\
                    a/b@1.0.0\tsrc/A.elm\tUNFINISHED LIST 1:2-1:2\tok\n";
        let table = parse_table(text).unwrap();
        assert_eq!(
            table.columns,
            [Column::Elm(Compat::V0_19_1), Column::ElmFormat]
        );
        assert_eq!(table.sources, "# modules 3 sources abc".parse().unwrap());
        let row = &table.rejected[&("a/b@1.0.0", "src/A.elm")];
        assert_eq!(
            row,
            &[
                Outcome::Rejected("UNFINISHED LIST 1:2-1:2".to_owned()),
                Outcome::Accepted
            ]
        );
    }

    #[test]
    fn columns_name_tools() {
        let column: Column = "elm 0.19.3".parse().unwrap();
        assert_eq!(
            column.tool(Source::Kernel),
            Tool::Elm(Source::Kernel, Compat::V0_19_3)
        );
        assert_eq!(column.to_string(), "elm 0.19.3");
        assert!("elm 0.19.0".parse::<Column>().is_err());
        let column: Column = "elm-format".parse().unwrap();
        assert_eq!(column.tool(Source::Kernel), Tool::ElmFormat);
        assert_eq!(column.to_string(), "elm-format");
        assert!("elm-review".parse::<Column>().is_err());
    }

    #[test]
    fn summaries_read_back_as_written() {
        let sources = Sources {
            modules: 3,
            digest: "abc".to_owned(),
        };
        assert_eq!(sources.to_string(), "# modules 3 sources abc");
        assert_eq!(sources.to_string().parse::<Sources>().unwrap(), sources);
        assert!("# modules x sources abc".parse::<Sources>().is_err());
    }

    #[test]
    fn outcomes_read_back_as_written() {
        for outcome in [
            Outcome::Accepted,
            Outcome::Rejected("UNFINISHED LIST 1:2-1:2".to_owned()),
            Outcome::Crashed,
        ] {
            assert_eq!(Outcome::from(outcome.to_string().as_str()), outcome);
        }
    }
}
