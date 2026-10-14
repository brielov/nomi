//! `cargo xtask oracle outline`: what each pinned elm reports for each
//! `elm.json` fixture, stored for the outline test.
//!
//! elm runs `make` with no modules in a project holding only the fixture
//! and an empty `src/`, so the first thing it does is read `elm.json`
//! (`Details.load`). A report about the file is summarized as its title
//! and the code excerpt's shape: the rows shown, and the underline or the
//! rows marked with `>`. Any other outcome means elm read the outline and
//! moved on. Each report about the file is also kept whole, as elm wrote
//! it, in `reports.tsv`, for the report tests.

use std::fmt::{self, Write as _};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use anyhow::{Context, Result, bail, ensure};
use nomi_base::Compat;
use serde::Deserialize;

use super::process::run_to_files;
use super::sandbox::{json_report, offline, sanitized};
use super::{by_output, cache_dir, cores, elm_header, fresh_dir, generated, pinned_elm};
use crate::corpus::compat;
use crate::corpus::parallel_on;
use crate::corpus::syntax::CRASH;
use crate::references::References;

const FIXTURES: &str = "crates/project/tests/outline_fixtures";

/// Long enough to read any fixture many times over. upstream loops
/// forever on some decode errors (`Reporting/Error/Json.hs`,
/// `expectationToReport`), so this is how such a loop is told.
const TIMEOUT: Duration = Duration::from_secs(10);

/// The titles of reports about `elm.json` itself, before elm looks at
/// source directories or packages.
const TITLES: &[&str] = &[
    // Reporting/Error/Json.hs, parse errors.
    "EXPECTING A VALUE",
    "EXTRA COMMA",
    "EXPECTING COLON",
    "UNFINISHED OBJECT",
    "UNFINISHED ARRAY",
    "ENDLESS STRING",
    "UNEXPECTED CONTROL CHARACTER",
    "UNKNOWN ESCAPE",
    "BAD HEX ESCAPE",
    "UNEXPECTED ENCODING",
    "BAD NUMBER",
    "UNEXPECTED NUMBER",
    "JSON PROBLEM",
    // Reporting/Error/Json.hs, decode errors.
    "EXPECTING OBJECT",
    "EXPECTING ARRAY",
    "EXPECTING STRING",
    "EXPECTING BOOL",
    "EXPECTING INT",
    "MISSING FIELD",
    "EXPECTING PAIR",
    // Reporting/Exit.hs, toOutlineProblemReport and toOutlineReport.
    "UNEXPECTED TYPE",
    "INVALID PACKAGE NAME",
    "PROBLEM WITH VERSION",
    "PROBLEM WITH CONSTRAINT",
    "PROBLEM WITH MODULE NAME",
    "HEADER TOO LONG",
    "PROBLEM WITH DEPENDENCY NAME",
    "UNKNOWN LICENSE",
    "SUMMARY TOO LONG",
    "NO SOURCE DIRECTORIES",
    "MISSING DEPENDENCY",
];

/// `cargo xtask oracle outline`.
pub(crate) fn run(root: &Path, refs: &References) -> Result<()> {
    let dir = root.join(FIXTURES);
    let mut fixtures: Vec<String> = fs::read_dir(&dir)
        .with_context(|| format!("reading {}", dir.display()))?
        .map(|entry| Ok(entry?.file_name().to_string_lossy().into_owned()))
        .collect::<Result<Vec<_>>>()?
        .into_iter()
        .filter_map(|name| name.strip_suffix(".json").map(str::to_owned))
        .collect();
    fixtures.sort();
    let elms: Vec<(Compat, PathBuf)> = (compat::compats(refs)?.into_iter())
        .map(|c| Ok((c, pinned_elm(refs, c)?)))
        .collect::<Result<_>>()?;
    let sandboxes = cache_dir()?.join("sandboxes").join("outline");
    let jobs: Vec<(&String, &(Compat, PathBuf))> = (fixtures.iter())
        .flat_map(|name| elms.iter().map(move |elm| (name, elm)))
        .collect();
    let results = parallel_on(cores(), &jobs, |&(name, (compat, elm))| {
        let text = fs::read(dir.join(format!("{name}.json")))?;
        let project = sandboxes.join(compat.as_str()).join(name);
        record(elm, &project, &text).with_context(|| format!("elm {compat} on {name}"))
    });
    let results = results.into_iter().collect::<Result<Vec<_>>>()?;
    let header = elm_header("fixture", elms.iter().map(|(c, _)| *c));
    let mut out = format!("{}\n{header}\n", generated("outline"));
    // A report as many elms write it is one row, which names them all.
    let mut reports = format!("{}\nfixture\telms\treport\n", generated("outline"));
    for (name, results) in fixtures.iter().zip(results.chunks(elms.len())) {
        out.push_str(name);
        for outcome in results {
            write!(out, "\t{outcome}")?;
        }
        let written = (elms.iter().zip(results)).filter_map(|((compat, _), outcome)| {
            let Outcome::Report { report, .. } = outcome else {
                return None;
            };
            Some((*compat, report.as_str()))
        });
        for (elms, report) in by_output(written) {
            writeln!(reports, "{name}\t{}\t{report}", elms.join(","))?;
        }
        out.push('\n');
    }
    fs::write(dir.join("oracle.tsv"), out)?;
    fs::write(dir.join("reports.tsv"), reports)?;
    println!(
        "recorded {} fixtures for {} elms",
        fixtures.len(),
        elms.len()
    );
    Ok(())
}

/// What elm made of a fixture, as `oracle.tsv` writes it.
enum Outcome {
    /// It read the outline and moved on.
    Accepted,
    /// It looped, as upstream does on some decode errors.
    Loop,
    /// It died of an uncaught exception.
    Crash,
    /// It reported on the file: the title and the excerpt's shape, and
    /// the report as elm wrote it, on one line.
    Report { summary: String, report: String },
}

impl fmt::Display for Outcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Outcome::Accepted => "accepted",
            Outcome::Loop => "loop",
            Outcome::Crash => CRASH,
            Outcome::Report { summary, .. } => summary,
        })
    }
}

/// Runs `elm` on a fresh project whose `elm.json` is `text`.
fn record(elm: &Path, project: &Path, text: &[u8]) -> Result<Outcome> {
    fresh_dir(project)?;
    fs::create_dir_all(project.join("src"))?;
    fs::create_dir_all(project.join("home"))?;
    fs::write(project.join("elm.json"), text)?;
    let mut command = Command::new(elm);
    command
        // A loop allocates without bound; the cap ends it early.
        .args(["make", "--report=json", "+RTS", "-N1", "-M512m", "-RTS"])
        .current_dir(project)
        .env("ELM_HOME", project.join("home"));
    let Some(output) = run_to_files(offline(&mut command), project, TIMEOUT)? else {
        return Ok(Outcome::Loop);
    };
    let errors = String::from_utf8_lossy(&output.stderr);
    if errors.contains("Heap exhausted") || errors.contains("<<loop>>") {
        return Ok(Outcome::Loop);
    }
    Ok(match json_report(&output) {
        Some(json) => match summarize(json)? {
            Some(summary) => Outcome::Report {
                summary,
                // One line: elm escapes the newlines it writes.
                report: String::from_utf8_lossy(json.trim_ascii_end()).into_owned(),
            },
            None => Outcome::Accepted,
        },
        // GHC's report of an uncaught exception.
        None if errors.starts_with("elm: ") => Outcome::Crash,
        None => Outcome::Accepted,
    })
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
enum Report {
    Error {
        path: Option<String>,
        title: String,
        message: Vec<Chunk>,
    },
    CompileErrors {},
}

#[derive(Deserialize)]
#[serde(untagged)]
enum Chunk {
    Plain(String),
    Styled { string: String },
}

/// The title and the excerpt's shape of a report about `elm.json`; none
/// for one about anything else.
fn summarize(json: &[u8]) -> Result<Option<String>> {
    // 0.19.1 writes the bytes of a character that isn't one.
    let json = sanitized(json);
    let json = String::from_utf8_lossy(&json);
    let report: Report = serde_json::from_str(&json).context("parsing elm's report")?;
    let Report::Error {
        path,
        title,
        message,
    } = report
    else {
        return Ok(None);
    };
    if path.as_deref() != Some("elm.json") || !TITLES.contains(&title.as_str()) {
        return Ok(None);
    }
    let text: String = message
        .iter()
        .map(|chunk| match chunk {
            Chunk::Plain(s) | Chunk::Styled { string: s } => s.as_str(),
        })
        .collect();
    Ok(Some(format!("{title} {}", excerpt(&text)?)))
}

/// `Render/Code.hs`'s `render`, read back: `rows A-B`, then `^C+W` for an
/// underline at column C, W wide, or `>C-D` for the rows marked.
fn excerpt(text: &str) -> Result<String> {
    let mut rows = Vec::new();
    let mut marked = Vec::new();
    let mut width = 0;
    let mut underline = None;
    for line in text.lines() {
        let trimmed = line.trim_start();
        let digits = trimmed.bytes().take_while(u8::is_ascii_digit).count();
        if digits > 0 && trimmed.as_bytes().get(digits) == Some(&b'|') {
            let row: u32 = trimmed[..digits].parse()?;
            rows.push(row);
            if trimmed.as_bytes().get(digits + 1) == Some(&b'>') {
                marked.push(row);
            }
            width = line.len() - trimmed.len() + digits;
        } else if !trimmed.is_empty() && trimmed.bytes().all(|b| b == b'^') && underline.is_none() {
            let spaces = line.len() - trimmed.len();
            underline = Some((spaces, trimmed.len()));
        }
    }
    let (Some(first), Some(last)) = (rows.first(), rows.last()) else {
        return Ok("rows none".to_owned());
    };
    let mut shape = format!("rows {first}-{last}");
    if let Some((spaces, carets)) = underline {
        ensure!(marked.is_empty(), "both an underline and marked rows");
        // `c1 + width + 2` spaces for a zero-based column.
        let col = spaces
            .checked_sub(width + 1)
            .context("an underline left of the code")?;
        write!(shape, " ^{col}+{carets}")?;
    } else if let (Some(a), Some(b)) = (marked.first(), marked.last()) {
        if marked.len() as u32 != b - a + 1 {
            bail!("marked rows are not contiguous: {marked:?}");
        }
        write!(shape, " >{a}-{b}")?;
    }
    Ok(shape)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn underlines_are_one_based_columns() {
        let text = "I ran into trouble:\n\n 9|    \"x\": 1\n10|    \"y\" 2\n          ^\nTry this.";
        assert_eq!(excerpt(text).unwrap(), "rows 9-10 ^7+1");
    }

    #[test]
    fn marked_rows() {
        let text = "Here:\n\n1| {\n2|>    \"a\": [\n3|>    ]\n4| }\nMore.";
        assert_eq!(excerpt(text).unwrap(), "rows 1-4 >2-3");
    }

    #[test]
    fn reports_about_other_things_are_acceptance() {
        let report = br#"{"type":"error","path":"elm.json","title":"MISSING SOURCE DIRECTORY","message":[]}"#;
        assert_eq!(summarize(report).unwrap(), None);
        let report = br#"{"type":"error","path":"elm.json","title":"NO SOURCE DIRECTORIES","message":["Here:\n\n3|   \"source-directories\": []\n",{"bold":false,"underline":false,"color":"RED","string":"                         ^^"},"\nMore."]}"#;
        assert_eq!(
            summarize(report).unwrap().as_deref(),
            Some("NO SOURCE DIRECTORIES rows 3-3 ^23+2")
        );
    }
}
