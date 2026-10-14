//! elm-review's problems, as elm-language-server finds them: its CLI run
//! in the project, as the client's settings name it and its tools, and
//! its JSON report read.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use nomi_base::{ColumnUnit, LineCol, LineIndex, TextRange};
use nomi_engine::Snapshot;
use serde_json::Value;

use crate::{Code, Edit, Problem, Severity};

/// How elm-review runs: `elmReviewDiagnostics`, and the programs the
/// client names in `elmReviewPath`, `elmPath` and `elmFormatPath`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReviewSettings {
    /// How its problems show.
    pub severity: Severity,
    /// elm-review, where not the one on the `PATH`; a relative path is
    /// the project's.
    pub program: Option<String>,
    pub elm: Option<String>,
    pub elm_format: Option<String>,
}

/// elm-review's problems in the project at `root`, where the project
/// configures it: its CLI's JSON report, as elm-language-server runs it.
pub(crate) fn problems(
    snapshot: &Snapshot,
    root: &Path,
    settings: &ReviewSettings,
    unit: ColumnUnit,
) -> Vec<Problem> {
    if !root.join("review/src/ReviewConfig.elm").is_file() {
        return Vec::new();
    }
    let Ok(output) = command(root, settings).output() else {
        return Vec::new();
    };
    if output.status.success() {
        return Vec::new();
    }
    match serde_json::from_slice(&output.stdout) {
        Ok(report) => read(snapshot, root, &report, settings.severity, unit),
        Err(_) => Vec::new(),
    }
}

/// The command that runs elm-review in `root` for its JSON report.
fn command(root: &Path, settings: &ReviewSettings) -> Command {
    let program = match settings.program.as_deref() {
        None => PathBuf::from("elm-review"),
        Some(path) if path.contains(['/', '\\']) => root.join(path),
        Some(path) => PathBuf::from(path),
    };
    let mut command = if cfg!(windows) && program.extension().is_some_and(|e| e == "js") {
        // Windows runs no script by its first line.
        let mut node = Command::new("node");
        node.arg(&program);
        node
    } else {
        Command::new(&program)
    };
    command.args(["--report", "json", "--namespace", "vscode"]);
    if let Some(elm) = &settings.elm {
        command.args(["--compiler", elm]);
    }
    if let Some(elm_format) = &settings.elm_format {
        command.args(["--elm-format-path", elm_format]);
    }
    command.current_dir(root).stdin(Stdio::null());
    command
}

/// The problems elm-review's `report` lists in the project at `root`,
/// less the suppressed ones, as `severity` says they show. Its one-based
/// lines and columns are read in `unit`, as elm-language-server takes
/// them for the protocol's.
fn read(
    snapshot: &Snapshot,
    root: &Path,
    report: &Value,
    severity: Severity,
    unit: ColumnUnit,
) -> Vec<Problem> {
    if report["type"] != "review-errors" {
        return Vec::new();
    }
    let mut found = Vec::new();
    for file in report["errors"].as_array().into_iter().flatten() {
        let path = root.join(file["path"].as_str().unwrap_or_default());
        let Ok(text) = snapshot.read(&path) else {
            continue;
        };
        let lines = LineIndex::new(&text);
        let range = |region: &Value| {
            let at = |p: &Value| {
                let one_based = |key: &str| (p[key].as_u64().unwrap_or(1) as u32).saturating_sub(1);
                let at = LineCol {
                    line: one_based("line"),
                    col: one_based("column"),
                };
                lines.clamped_offset(&text, at, unit)
            };
            let (start, end) = (at(&region["start"]), at(&region["end"]));
            TextRange::new(start, end.max(start))
        };
        for error in file["errors"].as_array().into_iter().flatten() {
            if error["suppressed"].as_bool() == Some(true) {
                continue;
            }
            let fixes = (error["fix"].as_array().into_iter().flatten())
                .map(|fix| Edit {
                    range: range(&fix["range"]),
                    text: fix["string"].as_str().unwrap_or_default().to_owned(),
                })
                .collect();
            let message = error["message"].as_str().unwrap_or_default().to_owned();
            found.push(Problem {
                severity,
                unnecessary: (error["rule"].as_str()).is_some_and(|r| r.starts_with("NoUnused")),
                fixes,
                ..Problem::new(
                    path.clone(),
                    range(&error["region"]),
                    Code::ElmReview,
                    message,
                )
            });
        }
    }
    found
}
