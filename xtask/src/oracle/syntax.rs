//! `cargo xtask oracle syntax`: whether each pinned elm release and
//! elm-format accept each mutation of the syntax suite's seeds and each
//! corpus module, stored for the syntax ratchets.
//!
//! For the mutations elm rejects it also stores a hash of each release's
//! `--report=json`, byte for byte, in `reports/<seed>.tsv`: the reports
//! themselves would take over a hundred megabytes, since each quotes its
//! mutation. `--show <case>` runs one mutation again and prints them.
//!
//! elm runs in the [`sandbox`](super::sandbox)es, one module at a time:
//! batching several modules in one `elm make` breaks as soon as two parse
//! with the same name (`Build.hs`, `checkUniqueRoots`).

use std::borrow::Cow;
use std::fmt::{self, Write as _};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use anyhow::{Context, Result, bail};
use nomi_base::Compat;
use nomi_syntax::Source;
use nomi_testkit::mutation::{self, Seed};
use nomi_testkit::sha256_hex as sha256;

use super::format::{Formatted, elm_format};
use super::sandbox::{Sandbox, Verdict};
use super::{cache_dir, elm_header, generated, pinned_binary, pinned_elm};
use crate::corpus::compat;
use crate::corpus::syntax as corpus_syntax;
use crate::references::References;

/// The mutation suite: seeds, stored verdicts and `passing.txt`.
const MUTATIONS: &str = "crates/syntax/tests/syntax_mutations";

/// Hex digits of SHA-256 a report's hash keeps: 64 bits, so a nomi report
/// that differs from elm's matches its hash with odds of 1 in 2^64.
const REPORT_HASH: usize = 16;

/// Which inputs to run.
#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub(crate) enum Inputs {
    Mutations,
    Corpus,
    Both,
}

/// What a tool made of one module's syntax, as the tables write it.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Outcome {
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
            Outcome::Crashed => corpus_syntax::CRASH,
        })
    }
}

/// `cargo xtask oracle syntax`.
pub(crate) fn run(root: &Path, refs: &References, inputs: Inputs, jobs: usize) -> Result<()> {
    let tools = Tools::new(root, refs)?;
    if inputs != Inputs::Corpus {
        mutations(root, &tools, jobs)?;
    }
    if inputs != Inputs::Mutations {
        corpus(root, &tools, jobs)?;
    }
    Ok(())
}

/// The mutation suite's seeds, each the kind of project it belongs to.
fn seeds(root: &Path) -> Result<Vec<Seed<Source>>> {
    mutation::seeds(&root.join(MUTATIONS)).map_err(anyhow::Error::msg)
}

fn mutations(root: &Path, tools: &Tools, jobs: usize) -> Result<()> {
    let out = root.join(MUTATIONS).join("oracle");
    let reports_dir = root.join(MUTATIONS).join("reports");
    std::fs::create_dir_all(&out)?;
    std::fs::create_dir_all(&reports_dir)?;
    for seed in seeds(root)? {
        let mutations = mutation::suite(&seed.name, &seed.text);
        eprintln!("==> {} ({} mutations)", seed.name, mutations.len());
        let rows = tools.run(&mutations, jobs, |m| {
            Ok(Job {
                name: format!("{} / {}", seed.name, m.name),
                source: seed.kind,
                text: Cow::Borrowed(m.source.as_bytes()),
            })
        })?;
        let digest = mutation::digest(&mutations);
        let header = |columns| format!("{}\n# sources {digest}\n{columns}", generated("syntax"));
        let mut table = header(tools.columns("case"));
        let mut reports = header(tools.elm_columns("case"));
        for (m, row) in mutations.iter().zip(&rows) {
            writeln!(table, "{}\t{}", m.name, row.cells())?;
            // Only mutations some elm rejects.
            if row.reports.iter().any(Option::is_some) {
                let cells: Vec<&str> = (row.reports.iter())
                    .map(|report| report.as_deref().unwrap_or("-"))
                    .collect();
                writeln!(reports, "{}\t{}", m.name, cells.join("\t"))?;
            }
        }
        let file = format!("{}.tsv", seed.name);
        std::fs::write(out.join(&file), table)?;
        std::fs::write(reports_dir.join(&file), reports)?;
    }
    Ok(())
}

fn corpus(root: &Path, tools: &Tools, jobs: usize) -> Result<()> {
    let modules = corpus_syntax::modules(root)?;
    eprintln!("==> corpus ({} modules)", modules.len());
    let rows = tools.run(&modules, jobs, |m| {
        Ok(Job {
            name: format!("{} {}", m.case, m.path),
            source: m.source,
            text: Cow::Owned(std::fs::read(&m.file)?),
        })
    })?;
    let mut table = format!(
        "{}\n# modules {} sources {}\n{}",
        generated("syntax"),
        modules.len(),
        corpus_syntax::digest(&modules)?,
        tools.columns("case\tpath")
    );
    // Only modules some tool rejects; the rest are accepted everywhere.
    for (m, row) in modules.iter().zip(&rows) {
        if row.outcomes.iter().any(|o| *o != Outcome::Accepted) {
            writeln!(table, "{}\t{}\t{}", m.case, m.path, row.cells())?;
        }
    }
    let dir = root.join(corpus_syntax::DIR);
    std::fs::create_dir_all(&dir)?;
    std::fs::write(dir.join("oracle.tsv"), table)?;
    Ok(())
}

/// The hash `reports/<seed>.tsv` keeps of a report.
fn report_hash(report: &[u8]) -> String {
    sha256(report)[..REPORT_HASH].to_owned()
}

/// Runs the mutation `case`, named `<seed> / <mutation>` as the ratchets
/// name it, again and prints each elm's report and its hash.
pub(crate) fn show(root: &Path, refs: &References, case: &str) -> Result<()> {
    let tools = Tools::new(root, refs)?;
    let (seed_name, name) = case
        .split_once(" / ")
        .with_context(|| format!("`{case}` is not `<seed> / <mutation>`"))?;
    for seed in seeds(root)?.into_iter().filter(|s| s.name == seed_name) {
        let Some(m) = mutation::suite(&seed.name, &seed.text)
            .into_iter()
            .find(|m| m.name == name)
        else {
            continue;
        };
        let dir = cache_dir()?.join("sandboxes").join("syntax-show");
        for (compat, elm) in &tools.elms {
            let sandbox = Sandbox::create(&dir.join(compat.as_str()), elm, *compat, root)?;
            match sandbox.check(seed.kind, m.source.as_bytes())? {
                Verdict::Rejected { report, .. } => println!(
                    "== elm {compat} {}\n{}",
                    report_hash(&report),
                    String::from_utf8_lossy(&report)
                ),
                verdict => println!("== elm {compat}: {verdict:?}"),
            }
        }
        return Ok(());
    }
    bail!("no mutation is named `{case}`")
}

/// Every pinned elm, oldest first, then elm-format.
struct Tools {
    root: PathBuf,
    elms: Vec<(Compat, PathBuf)>,
    elm_format: PathBuf,
}

/// One module to check.
struct Job<'a> {
    /// What it is, for messages.
    name: String,
    /// The kind of project it belongs to.
    source: Source,
    text: Cow<'a, [u8]>,
}

impl Tools {
    fn new(root: &Path, refs: &References) -> Result<Tools> {
        let elms = (compat::compats(refs)?.into_iter())
            .map(|c| Ok((c, pinned_elm(refs, c)?)))
            .collect::<Result<_>>()?;
        Ok(Tools {
            root: root.to_path_buf(),
            elms,
            elm_format: pinned_binary(refs, "elm-format", None)?,
        })
    }

    /// The tables' header: `first`, then each elm and elm-format.
    fn columns(&self, first: &str) -> String {
        let header = elm_header(first, self.elms.iter().map(|(c, _)| *c));
        format!("{header}\telm-format\n")
    }

    /// The reports' header: `first`, then each elm.
    fn elm_columns(&self, first: &str) -> String {
        let header = elm_header(first, self.elms.iter().map(|(c, _)| *c));
        format!("{header}\n")
    }

    /// What the tools made of each item, in order, on `workers` threads
    /// with sandboxes of their own, each reading the item's module, which
    /// `job` gives, as it takes it.
    fn run<T: Sync>(
        &self,
        items: &[T],
        workers: usize,
        job: impl Fn(&T) -> Result<Job<'_>> + Sync,
    ) -> Result<Vec<Row>> {
        let queue = Queue {
            next: AtomicUsize::new(0),
            done: AtomicUsize::new(0),
            failed: AtomicBool::new(false),
            rows: Mutex::new((0..items.len()).map(|_| None).collect()),
        };
        let dir = cache_dir()?.join("sandboxes").join("syntax");
        std::thread::scope(|scope| {
            let handles: Vec<_> = (0..workers.min(items.len()))
                .map(|w| {
                    let (queue, dir, job) = (&queue, dir.join(w.to_string()), &job);
                    scope.spawn(move || {
                        let result = self.work(&dir, items, job, queue);
                        if result.is_err() {
                            queue.failed.store(true, Ordering::Relaxed);
                        }
                        result
                    })
                })
                .collect();
            handles
                .into_iter()
                .try_for_each(|h| h.join().expect("a worker panicked"))
        })?;
        Ok((queue
            .rows
            .into_inner()
            .expect("every worker ended")
            .into_iter())
        .map(|row| row.expect("every job ran"))
        .collect())
    }

    /// One worker: its sandboxes, then items until none are left or another
    /// worker failed.
    fn work<T>(
        &self,
        dir: &Path,
        items: &[T],
        job: impl Fn(&T) -> Result<Job<'_>>,
        queue: &Queue,
    ) -> Result<()> {
        let sandboxes = (self.elms.iter())
            .map(|(c, elm)| Sandbox::create(&dir.join(c.as_str()), elm, *c, &self.root))
            .collect::<Result<Vec<_>>>()?;
        while !queue.failed.load(Ordering::Relaxed) {
            let i = queue.next.fetch_add(1, Ordering::Relaxed);
            let Some(item) = items.get(i) else { break };
            let job = job(item)?;
            let row = (self.row(&sandboxes, job.source, &job.text)).context(job.name)?;
            queue.rows.lock().expect("no worker panicked")[i] = Some(row);
            let n = queue.done.fetch_add(1, Ordering::Relaxed) + 1;
            if n.is_multiple_of(1000) {
                eprintln!("  {n}/{}", items.len());
            }
        }
        Ok(())
    }

    /// What every tool made of `text`, a module of a `source` project.
    fn row(&self, sandboxes: &[Sandbox], source: Source, text: &[u8]) -> Result<Row> {
        let mut row = Row {
            outcomes: Vec::new(),
            reports: Vec::new(),
        };
        for sandbox in sandboxes {
            let (outcome, report) = match sandbox.check(source, text)? {
                Verdict::Accepted => (Outcome::Accepted, None),
                Verdict::Rejected { summary, report } => {
                    (Outcome::Rejected(summary), Some(report_hash(&report)))
                }
                Verdict::Crashed => (Outcome::Crashed, None),
            };
            row.outcomes.push(outcome);
            row.reports.push(report);
        }
        row.outcomes
            .push(match elm_format(&self.elm_format, text)? {
                Formatted::Output(_) => Outcome::Accepted,
                Formatted::Rejected(at) => Outcome::Rejected(at),
                Formatted::Crashed(stderr) => bail!("elm-format failed:\n{stderr}"),
            });
        Ok(row)
    }
}

/// What the tools made of one job: each elm's outcome then elm-format's,
/// in the tables' order, and the hash of each elm's report where it
/// rejected the job.
struct Row {
    outcomes: Vec<Outcome>,
    reports: Vec<Option<String>>,
}

impl Row {
    /// The row's cells in the tables.
    fn cells(&self) -> String {
        let cells: Vec<String> = self.outcomes.iter().map(Outcome::to_string).collect();
        cells.join("\t")
    }
}

/// The jobs' progress and what they gave, shared by the workers.
struct Queue {
    next: AtomicUsize,
    done: AtomicUsize,
    failed: AtomicBool,
    rows: Mutex<Vec<Option<Row>>>,
}
