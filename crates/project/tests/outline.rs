//! Every `elm.json` fixture under `outline_fixtures/` against what each
//! elm reports for it, as `cargo xtask oracle outline` stored it in
//! `oracle.tsv`: whether elm accepts the outline, or the report's title
//! and the shape of its code excerpt, or that elm loops or crashes. The
//! report itself is checked whole in `reports.rs`; here it is reduced to that shape with
//! upstream's choices of region (`Reporting/Error/Json.hs`,
//! `Reporting/Exit.hs`) and of rendering (`Reporting/Render/Code.hs`), so
//! the test pins the problem and its positions. Every accepted outline
//! must also read back from what it writes.

use std::collections::BTreeMap;
use std::path::Path;

use nomi_base::Compat;
use nomi_project::json::decode::{Expectation, Problem};
use nomi_project::json::{ParseError, ParseErrorKind, Position, Region, StringProblem};
use nomi_project::outline::{OutlineProblem, SrcDir};
use nomi_project::{ConstraintError, Outline, OutlineError};

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
        .map(|column| {
            Compat::ALL
                .into_iter()
                .find(|c| format!("elm {c}") == *column)
                .unwrap_or_else(|| panic!("unknown column {column}"))
        })
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
                Ok(_) => "accepted".to_owned(),
                Err(e) => signature(e, &src, compat),
            };
            if found != expected {
                failures.push(format!("{name} on {compat}: {found}, elm: {expected}"));
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

/// What elm's report on `error` looks like, in `oracle.tsv`'s terms.
fn signature(error: &OutlineError, src: &[u8], compat: Compat) -> String {
    let rows = rows(src);
    match error {
        OutlineError::NoPkgCore | OutlineError::NoAppCore | OutlineError::NoAppJson => {
            "MISSING DEPENDENCY rows none".to_owned()
        }
        OutlineError::Parse(error) => parse_error(error, rows, compat),
        OutlineError::Decode(problem) => problem_report(problem, rows),
    }
}

/// `parseErrorToReport`: two rows back to the error, the error marked.
fn parse_error(error: &ParseError, rows: u32, compat: Compat) -> String {
    let title = match error.kind {
        ParseErrorKind::Start => "EXPECTING A VALUE",
        ParseErrorKind::ObjectField => "EXTRA COMMA",
        ParseErrorKind::ObjectColon => "EXPECTING COLON",
        ParseErrorKind::ObjectEnd => "UNFINISHED OBJECT",
        ParseErrorKind::ArrayEnd => "UNFINISHED ARRAY",
        ParseErrorKind::String(StringProblem::BadStringEnd) => "ENDLESS STRING",
        ParseErrorKind::String(StringProblem::BadStringControlChar) => {
            "UNEXPECTED CONTROL CHARACTER"
        }
        ParseErrorKind::String(StringProblem::BadStringEscapeChar) => "UNKNOWN ESCAPE",
        ParseErrorKind::String(StringProblem::BadStringEscapeHex) => "BAD HEX ESCAPE",
        ParseErrorKind::String(StringProblem::BadStringNotUtf8) => "UNEXPECTED ENCODING",
        ParseErrorKind::NoLeadingZeros => "BAD NUMBER",
        ParseErrorKind::NoFloats => "UNEXPECTED NUMBER",
        ParseErrorKind::BadEnd => "JSON PROBLEM",
        ParseErrorKind::Crash => return "crash".to_owned(),
    };
    let at = error.at;
    // `max 1 (row - 2)`, in 0.19.1's 16-bit rows: from row 1 it wraps to
    // 65535, past the error, so there are no lines and `last []` crashes.
    let first = match compat {
        Compat::V0_19_1 => u32::from((at.row as u16).wrapping_sub(2)).max(1),
        Compat::V0_19_2 | Compat::V0_19_3 => at.row.saturating_sub(2).max(1),
    };
    let surroundings = Region {
        start: Position { row: first, col: 1 },
        end: at,
    };
    let Some(code) = render(rows, surroundings, Region { start: at, end: at }) else {
        return "crash".to_owned();
    };
    format!("{title} {code}")
}

/// `problemToReport`.
fn problem_report(problem: &Problem<OutlineProblem>, rows: u32) -> String {
    match problem {
        Problem::Field(_, inner) | Problem::Index(_, inner) => problem_report(inner, rows),
        // The deepest problem, the first of equals.
        Problem::OneOf(first, rest) => {
            let deepest = std::iter::once(&**first)
                .chain(rest)
                .rev()
                .max_by_key(|p| depth(p))
                .unwrap();
            problem_report(deepest, rows)
        }
        Problem::Expecting(region, expectation) => {
            // `expectationToReport` defines `region` as itself when the
            // value is on one row, and so never returns.
            if region.start.row == region.end.row {
                return "loop".to_owned();
            }
            let title = match expectation {
                Expectation::Object => "EXPECTING OBJECT",
                Expectation::ObjectWith(_) => "MISSING FIELD",
                Expectation::Array => "EXPECTING ARRAY",
                Expectation::String => "EXPECTING STRING",
                Expectation::Bool => "EXPECTING BOOL",
                Expectation::Int => "EXPECTING INT",
                Expectation::ArrayPair(_) => "EXPECTING PAIR",
            };
            let start = Region {
                start: region.start,
                end: region.start,
            };
            snippet(title, rows, start, None)
        }
        Problem::Failure(region, x) => {
            let (title, highlight) = match x {
                OutlineProblem::BadType => ("UNEXPECTED TYPE", None),
                OutlineProblem::BadPkgName(at) => ("INVALID PACKAGE NAME", Some(*at)),
                OutlineProblem::BadVersion(at) => ("PROBLEM WITH VERSION", Some(*at)),
                OutlineProblem::BadConstraint(ConstraintError::BadFormat(at)) => {
                    ("PROBLEM WITH CONSTRAINT", Some(*at))
                }
                OutlineProblem::BadConstraint(ConstraintError::InvalidRange(..)) => {
                    ("PROBLEM WITH CONSTRAINT", None)
                }
                OutlineProblem::BadModuleName(at) => ("PROBLEM WITH MODULE NAME", Some(*at)),
                OutlineProblem::BadModuleHeaderTooLong => ("HEADER TOO LONG", None),
                OutlineProblem::BadDependencyName(at) => {
                    ("PROBLEM WITH DEPENDENCY NAME", Some(*at))
                }
                OutlineProblem::BadLicense(..) => ("UNKNOWN LICENSE", None),
                OutlineProblem::BadSummaryTooLong => ("SUMMARY TOO LONG", None),
                OutlineProblem::NoSrcDirs => ("NO SOURCE DIRECTORIES", None),
            };
            snippet(title, rows, *region, highlight)
        }
    }
}

/// `getMaxDepth`.
fn depth(problem: &Problem<OutlineProblem>) -> usize {
    match problem {
        Problem::Field(_, inner) | Problem::Index(_, inner) => 1 + depth(inner),
        Problem::OneOf(first, rest) => std::iter::once(&**first)
            .chain(rest)
            .map(depth)
            .max()
            .unwrap(),
        Problem::Failure(..) | Problem::Expecting(..) => 0,
    }
}

fn snippet(title: &str, rows: u32, region: Region, highlight: Option<Position>) -> String {
    let highlight = highlight.map_or(region, |at| Region { start: at, end: at });
    match render(rows, region, highlight) {
        Some(code) => format!("{title} {code}"),
        None => "crash".to_owned(),
    }
}

/// `Code.render`'s shape: the rows of `region` the source has, then the
/// underline of a one-row `highlight` that reaches the last row, or else
/// the rows of `highlight` marked. `None` where upstream takes the width
/// of the row numbers from no rows at all.
fn render(rows: u32, region: Region, highlight: Region) -> Option<String> {
    let first = region.start.row;
    let last = region.end.row.min(rows);
    if first > last {
        return None;
    }
    let mut shape = format!("rows {first}-{last}");
    if highlight.start.row == highlight.end.row && highlight.end.row >= region.end.row {
        let width = highlight.end.col.saturating_sub(highlight.start.col).max(1);
        shape += &format!(" ^{}+{width}", highlight.start.col);
    } else {
        let marked_first = highlight.start.row.max(first);
        let marked_last = highlight.end.row.min(last);
        if marked_first <= marked_last {
            shape += &format!(" >{marked_first}-{marked_last}");
        }
    }
    Some(shape)
}

/// How many rows `Code.toSource` makes of `src`: Haskell's `lines`, then
/// one empty row more.
fn rows(src: &[u8]) -> u32 {
    let newlines = src.iter().filter(|&&b| b == b'\n').count();
    let unterminated = usize::from(!src.is_empty() && !src.ends_with(b"\n"));
    u32::try_from(newlines + unterminated + 1).unwrap()
}
