//! The elm-format workloads: nomi, run as `elm-format`, against the pinned
//! elm-format 0.8.8, end to end. Each corpus app is validated whole, as
//! `elm-format --validate <app>`, which reads every module and writes
//! nothing, and its largest module goes through `--stdin`, as an editor
//! formats a buffer. Both tools must exit alike and, on standard input,
//! write the same bytes.

use std::ffi::OsString;
use std::fs;
use std::process::{Command, Stdio};

use anyhow::{Context, Result, ensure};

use super::measure::{Run, best_of, measure};
use super::{Row, Setup, Table};
use crate::corpus::{apps, corpus_dir, files_below};
use crate::oracle::{binary_path, installed};
use crate::references::References;

const RUNS: usize = 5;

/// A workload: its name, the arguments, and what goes to standard input.
type Input<'a> = (String, Vec<OsString>, Option<&'a [u8]>);

pub(crate) fn table(root: &std::path::Path, refs: &References, setup: &Setup) -> Result<Table> {
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
        let modules: Vec<_> = (files_below(&dir)?.into_iter())
            .filter(|f| {
                f.ends_with(".elm")
                    && !(f.split('/'))
                        .any(|part| ["elm-stuff", "node_modules", ".git"].contains(&part))
            })
            .map(|f| dir.join(f))
            .collect();
        let sizes = (modules.iter())
            .map(|m| Ok((fs::metadata(m)?.len(), m)))
            .collect::<Result<Vec<_>>>()?;
        let total: u64 = sizes.iter().map(|(size, _)| size).sum();
        let (_, largest) = sizes.iter().max().context("an app without modules")?;
        let text = fs::read(largest)?;
        let inputs: [Input<'_>; 2] = [
            (
                format!(
                    "{} `--validate`, {} modules, {} KiB",
                    app.name,
                    modules.len(),
                    total / 1024
                ),
                vec!["--validate".into(), dir.clone().into()],
                None,
            ),
            (
                format!("{} `--stdin`, {} KiB", app.name, text.len() / 1024),
                vec!["--stdin".into()],
                Some(&text),
            ),
        ];
        for (name, args, stdin) in inputs {
            if let Some(input) = stdin {
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
                    stdin,
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

/// What `binary` writes for `input` on `--stdin`.
fn formatted(binary: &std::path::Path, input: &[u8]) -> Result<Vec<u8>> {
    use std::io::Write;
    let mut child = Command::new(binary)
        .arg("--stdin")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .with_context(|| binary.display().to_string())?;
    child.stdin.take().expect("piped").write_all(input)?;
    Ok(child.wait_with_output()?.stdout)
}
