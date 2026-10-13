//! `cargo xtask bench`: the scoreboard. The release build of
//! nomi against the pinned upstream tools on every workload they share,
//! each measured for wall time, CPU time and peak memory, with the same
//! output from both. `bench elm`, `bench elm-format` and `bench lsp` print
//! their part;
//! the whole run also writes it into `docs/benchmarks.md`.

pub(crate) mod elm;
pub(crate) mod elm_format;
pub(crate) mod lsp;
pub(crate) mod measure;

use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result, ensure};

use crate::oracle::cache_dir;
use crate::references::References;
use measure::Cost;

/// Where `docs/benchmarks.md` holds the scoreboard.
const START: &str = "<!-- scoreboard:start -->";
const END: &str = "<!-- scoreboard:end -->";

/// What every workload shares: the scratch directory and nomi's release
/// build in it under each name it answers to.
pub(crate) struct Setup {
    pub scratch: PathBuf,
    pub bin: PathBuf,
}

impl Setup {
    /// nomi, run as `name`.
    pub(crate) fn nomi(&self, name: &str) -> PathBuf {
        self.bin.join(name)
    }
}

/// One workload family's results.
pub(crate) struct Table {
    pub title: String,
    /// What the workloads are and how they were run.
    pub about: String,
    pub upstream: String,
    pub rows: Vec<Row>,
    /// Workloads both tools refuse alike, which have nothing to compare.
    pub refused: Vec<String>,
}

pub(crate) struct Row {
    pub name: String,
    pub upstream: Cost,
    pub nomi: Cost,
}

/// Runs `part`, `elm` or `elm-format`, or with none every workload,
/// writing the scoreboard.
pub(crate) fn run(root: &Path, refs: &References, part: Option<&str>) -> Result<()> {
    crate::cargo(
        root,
        &["build", "--locked", "--release", "--package", "nomi-cli"],
    )?;
    let scratch = cache_dir()?.join("sandboxes").join("bench");
    let bin = scratch.join("bin");
    if bin.exists() {
        fs::remove_dir_all(&bin)?;
    }
    fs::create_dir_all(&bin)?;
    for name in ["elm", "elm-format", "nomi"] {
        fs::copy(root.join("target/release/nomi"), bin.join(name))?;
    }
    let setup = Setup { scratch, bin };
    let mut tables = Vec::new();
    if part.is_none_or(|p| p == "elm") {
        tables.extend(elm::tables(root, refs, &setup)?);
    }
    if part.is_none_or(|p| p == "elm-format") {
        tables.push(elm_format::table(root, refs, &setup)?);
    }
    if part.is_none_or(|p| p == "lsp") {
        tables.push(lsp::table(root, refs, &setup)?);
    }
    let board = render(&tables);
    print!("{board}");
    if part.is_none() {
        write(root, &board)?;
    }
    Ok(())
}

/// The scoreboard as Markdown.
fn render(tables: &[Table]) -> String {
    let mut out = String::new();
    let (mut won, mut cells) = (0, 0);
    for table in tables {
        for row in &table.rows {
            won += usize::from(row.nomi.wall <= row.upstream.wall)
                + usize::from(row.nomi.cpu <= row.upstream.cpu)
                + usize::from(row.nomi.memory <= row.upstream.memory);
            cells += 3;
        }
    }
    let _ = writeln!(
        out,
        "Measured on {}, rustc {}. nomi is at or under upstream in {won} of {cells} cells; \
         a cell where it isn't is marked ✗.\n",
        measure::machine(),
        rustc_version()
    );
    for table in tables {
        let _ = writeln!(out, "### {}\n\n{}\n", table.title, table.about);
        let up = &table.upstream;
        let _ = writeln!(
            out,
            "| workload | {up} | nomi | speedup | {up} CPU | nomi CPU | {up} memory | nomi memory |"
        );
        let _ = writeln!(out, "|---|--:|--:|--:|--:|--:|--:|--:|");
        for row in &table.rows {
            let (u, n) = (row.upstream, row.nomi);
            let mark = |worse: bool| if worse { " ✗" } else { "" };
            let _ = writeln!(
                out,
                "| {} | {} | {}{} | {:.2}× | {} | {}{} | {} | {}{} |",
                row.name,
                time(u.wall),
                time(n.wall),
                mark(n.wall > u.wall),
                u.wall.as_secs_f64() / n.wall.as_secs_f64(),
                time(u.cpu),
                time(n.cpu),
                mark(n.cpu > u.cpu),
                mib(u.memory),
                mib(n.memory),
                mark(n.memory > u.memory),
            );
        }
        if !table.refused.is_empty() {
            let _ = writeln!(out, "\nBoth refuse: {}.", table.refused.join(", "));
        }
        out.push('\n');
    }
    out
}

fn time(d: Duration) -> String {
    let s = d.as_secs_f64();
    if s < 1.0 {
        format!("{:.1} ms", s * 1e3)
    } else {
        format!("{s:.2} s")
    }
}

fn mib(kib: u64) -> String {
    format!("{:.0} MiB", kib as f64 / 1024.0)
}

fn rustc_version() -> String {
    let out = std::process::Command::new("rustc")
        .arg("--version")
        .output();
    out.ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .and_then(|v| v.split_whitespace().nth(1).map(str::to_owned))
        .unwrap_or_else(|| "of unknown version".to_owned())
}

/// Puts `board` between the scoreboard's markers in `docs/benchmarks.md`.
fn write(root: &Path, board: &str) -> Result<()> {
    let path = root.join("docs/benchmarks.md");
    let text = fs::read_to_string(&path)?;
    let start = text
        .find(START)
        .context("docs/benchmarks.md has no scoreboard start")?;
    let end = text
        .find(END)
        .context("docs/benchmarks.md has no scoreboard end")?;
    ensure!(start < end, "the scoreboard's markers are out of order");
    let updated = format!("{}{START}\n\n{board}{}", &text[..start], &text[end..]);
    fs::write(&path, updated)?;
    println!("wrote {}", path.display());
    Ok(())
}
