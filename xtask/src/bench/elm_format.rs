//! The elm-format workloads: nomi, run as `elm-format`, against the pinned
//! elm-format 0.8.8, end to end. Each corpus app is validated whole, as
//! `elm-format --validate <app>`, which reads every module and writes
//! nothing, and its largest module goes through `--stdin`, as an editor
//! formats a buffer. Both tools must exit alike and, on standard input,
//! write the same bytes.

use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, Result, ensure};
use nomi_testkit::files_below;

use super::measure::{Run, best_of, measure};
use super::{Row, Setup, Table};
use crate::corpus::{apps, corpus_dir, is_module};
use crate::oracle::process::run_within;
use crate::oracle::{binary_path, installed};
use crate::references::References;

const RUNS: usize = 5;

/// A workload: its name, elm-format's arguments, and its standard input
/// if it reads one.
struct Workload {
    name: String,
    args: Vec<OsString>,
    stdin: Option<Vec<u8>>,
}

pub(crate) fn table(root: &Path, refs: &References, setup: &Setup) -> Result<Table> {
    let pin = (refs.binary.iter())
        .find(|b| b.name == "elm-format")
        .context("elm-format is not pinned")?;
    let upstream = installed(binary_path(pin)?)?;
    let nomi = setup.nomi("elm-format");
    let home = setup.scratch.join("elm-format-home");
    fs::create_dir_all(&home)?;
    let env = [("ELM_HOME", home)];
    let report = setup.scratch.join("time.txt");
    let corpus = corpus_dir()?;
    let mut rows = Vec::new();
    for app in apps::load(root)?.app {
        let dir = apps::app_dir(&corpus, &app);
        ensure!(
            dir.exists(),
            "{} is missing; run `cargo xtask corpus fetch`",
            dir.display()
        );
        for workload in workloads(&app.name, &dir)? {
            let Workload { name, args, stdin } = workload;
            if let Some(input) = &stdin {
                ensure!(
                    formatted(&upstream, input)? == formatted(&nomi, input)?,
                    "elm-format and nomi format {name} differently"
                );
            }
            let mut costs = Vec::new();
            let mut codes = Vec::new();
            for binary in [&upstream, &nomi] {
                let run = Run {
                    binary,
                    args: args.clone(),
                    dir: &dir,
                    env: &env,
                    stdin: stdin.as_deref(),
                    report: &report,
                };
                let (cost, code) = best_of(RUNS, || measure(&run))
                    .with_context(|| format!("{} on {name}", binary.display()))?;
                costs.push(cost);
                codes.push(code);
            }
            ensure!(
                codes[0] == codes[1],
                "elm-format exits with {:?} on {name} but nomi with {:?}",
                codes[0],
                codes[1]
            );
            rows.push(Row {
                name,
                upstream: costs[0],
                nomi: costs[1],
            });
        }
    }
    Ok(Table {
        title: "elm-format".to_owned(),
        about: format!(
            "nomi, run as `elm-format`, against the pinned elm-format {} (its official build, \
             with its default runtime options), from process start to exit. Each corpus app is \
             validated whole with `--validate`, which reads and formats every module and writes \
             nothing; its largest module is formatted through `--stdin`, as editors do, with the \
             same output from both. The best of {RUNS} runs counts for each measure.",
            pin.version
        ),
        upstream: "elm-format".to_owned(),
        rows,
        refused: Vec::new(),
    })
}

/// The workloads of the app `name` at `dir`: validating it whole, and
/// formatting its largest module from standard input.
fn workloads(name: &str, dir: &Path) -> Result<[Workload; 2]> {
    let modules: Vec<PathBuf> = (files_below(dir)?.into_iter())
        .filter(|f| is_module(f))
        .map(|f| dir.join(f))
        .collect();
    let sizes = (modules.iter())
        .map(|m| Ok((fs::metadata(m)?.len(), m)))
        .collect::<Result<Vec<_>>>()?;
    let total: u64 = sizes.iter().map(|(size, _)| size).sum();
    let (_, largest) = sizes.iter().max().context("an app without modules")?;
    let text = fs::read(largest)?;
    Ok([
        Workload {
            name: format!(
                "{name} `--validate`, {} modules, {} KiB",
                modules.len(),
                total / 1024
            ),
            args: vec!["--validate".into(), dir.into()],
            stdin: None,
        },
        Workload {
            name: format!("{name} `--stdin`, {} KiB", text.len() / 1024),
            args: vec!["--stdin".into()],
            stdin: Some(text),
        },
    ])
}

/// What `binary` writes for `input` on `--stdin`.
fn formatted(binary: &Path, input: &[u8]) -> Result<Vec<u8>> {
    let mut command = Command::new(binary);
    command
        .arg("--stdin")
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let output = run_within(&mut command, input, None)
        .with_context(|| binary.display().to_string())?
        .expect("a run without a limit ends");
    Ok(output.stdout)
}
