//! Every `elm.json` fixture under `outline_fixtures/` against what each
//! elm does with it, as `cargo xtask oracle outline` stored it in
//! `oracle.tsv`: accepts the outline, reports a problem, or loops or
//! crashes where nomi reports one. The reports themselves are checked
//! whole in `reports.rs`. Every accepted outline must also read back from
//! what it writes.

use std::collections::BTreeMap;
use std::path::Path;

use nomi_base::Compat;
use nomi_project::json::ParseErrorKind;
use nomi_project::json::decode::{self, Problem};
use nomi_project::outline::{OutlineProblem, SrcDir};
use nomi_project::{Outline, OutlineError};
use nomi_testkit::release_column;

const RERUN: &str = "rerun `cargo xtask oracle outline`";

#[test]
fn outline_fixtures() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/outline_fixtures");
    let table = std::fs::read_to_string(dir.join("oracle.tsv"))
        .unwrap_or_else(|e| panic!("reading oracle.tsv: {e}; {RERUN}"));
    let mut lines = table.lines().filter(|l| !l.starts_with('#'));
    let header: Vec<&str> = lines.next().expect("a header").split('\t').collect();
    let compats: Vec<Compat> = header[1..]
        .iter()
        .map(|column| release_column(column).unwrap_or_else(|| panic!("unknown column {column}")))
        .collect();
    let recorded: BTreeMap<&str, Vec<&str>> = lines
        .map(|line| {
            let mut cells = line.split('\t');
            (cells.next().unwrap(), cells.collect())
        })
        .collect();
    let mut fixtures: Vec<String> = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(|e| {
            let name = e.unwrap().file_name().into_string().unwrap();
            name.strip_suffix(".json").map(str::to_owned)
        })
        .collect();
    fixtures.sort();
    let mut failures = Vec::new();
    for name in &fixtures {
        let Some(expected) = recorded.get(name.as_str()) else {
            failures.push(format!("{name}: not in oracle.tsv; {RERUN}"));
            continue;
        };
        let src = std::fs::read(dir.join(format!("{name}.json"))).unwrap();
        for (&compat, &expected) in compats.iter().zip(expected) {
            let read = Outline::read(&src, compat);
            let found = match &read {
                Ok(_) => Outcome::Accepted,
                Err(e) => outcome(e, compat),
            };
            if found != Outcome::of_cell(expected) {
                failures.push(format!("{name} on {compat}: {found:?}, elm: {expected}"));
            }
            if let Ok(outline) = read
                && let Err(e) = rewrite(&outline, compat)
            {
                failures.push(format!("{name} on {compat}: {e}"));
            }
        }
    }
    for name in recorded.keys() {
        if !fixtures.iter().any(|f| f == name) {
            failures.push(format!("{name}: in oracle.tsv, but no fixture; {RERUN}"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// What elm does with an outline.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Outcome {
    Accepted,
    Report,
    Loop,
    Crash,
}

impl Outcome {
    /// From `oracle.tsv`'s cell: a report's title and the shape of its
    /// code excerpt are a report.
    fn of_cell(cell: &str) -> Outcome {
        match cell {
            "accepted" => Outcome::Accepted,
            "loop" => Outcome::Loop,
            "crash" => Outcome::Crash,
            _ => Outcome::Report,
        }
    }
}

/// What elm does with an outline nomi rejects with `error`: it reports
/// the problem, unless it loops or crashes on the way.
fn outcome(error: &OutlineError, compat: Compat) -> Outcome {
    match error {
        // `parseErrorToReport` shows from `max 1 (row - 2)`, in 0.19.1's
        // 16-bit rows: from row 1 or 0 that wraps past the error, so there
        // are no lines and `last []` crashes.
        OutlineError::Parse(e)
            if (compat == Compat::V0_19_1 && e.at.row < 2) || e.kind == ParseErrorKind::Crash =>
        {
            Outcome::Crash
        }
        // `expectationToReport` defines the region it shows of a value on
        // one row as itself, and so never returns.
        OutlineError::Decode(problem)
            if matches!(reported(problem), Problem::Expecting(region, _)
                if region.start.row == region.end.row) =>
        {
            Outcome::Loop
        }
        OutlineError::Parse(_)
        | OutlineError::Decode(_)
        | OutlineError::NoPkgCore
        | OutlineError::NoAppCore
        | OutlineError::NoAppJson => Outcome::Report,
    }
}

/// `problemToReport`'s choice of problem.
fn reported(problem: &Problem<OutlineProblem>) -> &Problem<OutlineProblem> {
    match problem {
        Problem::Field(_, inner) | Problem::Index(_, inner) => reported(inner),
        Problem::OneOf(first, rest) => reported(decode::deepest(first, rest)),
        Problem::Failure(..) | Problem::Expecting(..) => problem,
    }
}

/// Reads back what `outline` writes. Upstream doesn't round-trip source
/// directories: it writes their characters escaped but reads them raw,
/// so backslashes double, and 0.19.1 writes characters that aren't any as
/// bytes it then can't read. Everything else must come back as it was.
fn rewrite(outline: &Outline, compat: Compat) -> Result<(), String> {
    let written = outline.write();
    let again = match Outline::read(&written, compat) {
        Ok(again) => again,
        Err(_) if source_dirs(outline).iter().any(|d| d.path().is_none()) => return Ok(()),
        Err(e) => return Err(format!("unreadable as written: {e:?}")),
    };
    let (mut before, mut after) = (outline.clone(), again);
    let (dirs, dirs_after) = match (&mut before, &mut after) {
        (Outline::App(a), Outline::App(b)) => (
            std::mem::take(&mut a.source_dirs),
            std::mem::take(&mut b.source_dirs),
        ),
        _ => (Vec::new(), Vec::new()),
    };
    if before != after {
        return Err("rewriting changes it".to_owned());
    }
    let kept = |(d, e): (&SrcDir, &SrcDir)| d.raw().contains(&b'\\') || d == e;
    if dirs.len() != dirs_after.len() || !dirs.iter().zip(&dirs_after).all(kept) {
        return Err("rewriting changes a source directory without a backslash".to_owned());
    }
    Ok(())
}

fn source_dirs(outline: &Outline) -> &[SrcDir] {
    match outline {
        Outline::App(app) => &app.source_dirs,
        Outline::Pkg(_) => &[],
    }
}
