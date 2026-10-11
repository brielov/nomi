//! `cargo xtask bench elm-format`: nomi, run as `elm-format`, against the
//! pinned elm-format, end to end. Each corpus app is validated whole, as
//! `elm-format --validate <app>`, which reads every module and writes
//! nothing, and its largest module goes through `--stdin`, as an editor
//! formats a buffer. Both tools must exit alike; the fastest of
//! [`RUNS`] runs counts.

pub mod elm;

use std::fs;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, ensure};

use crate::corpus::{apps, corpus_dir, files_below};
use crate::oracle::{binary_path, cache_dir};
use crate::references::References;

const RUNS: usize = 5;

pub fn elm_format(root: &Path, refs: &References) -> Result<()> {
    let pin = refs
        .binary
        .iter()
        .find(|b| b.name == "elm-format")
        .context("elm-format is not pinned")?;
    let upstream = binary_path(pin)?;
    ensure!(
        upstream.exists(),
        "{} is missing; run `cargo xtask oracle fetch`",
        upstream.display()
    );
    crate::cargo(
        root,
        &["build", "--locked", "--release", "--package", "nomi-cli"],
    )?;
    let scratch = cache_dir()?.join("sandboxes").join("bench");
    if scratch.exists() {
        fs::remove_dir_all(&scratch)?;
    }
    let home = scratch.join("home");
    fs::create_dir_all(&home)?;
    let nomi = scratch.join("elm-format");
    fs::copy(root.join("target/release/nomi"), &nomi)?;

    let corpus = corpus_dir()?;
    println!(
        "| input                  |   modules |      size | elm-format |      nomi | speedup |"
    );
    println!(
        "|------------------------|-----------|-----------|------------|-----------|---------|"
    );
    for app in apps::load(root)?.app {
        let dir = apps::app_dir(&corpus, &app);
        ensure!(
            dir.exists(),
            "{} is missing; run `cargo xtask corpus fetch`",
            dir.display()
        );
        let modules: Vec<_> = files_below(&dir)?
            .into_iter()
            .filter(|f| {
                f.ends_with(".elm")
                    && !f
                        .split('/')
                        .any(|part| ["elm-stuff", "node_modules", ".git"].contains(&part))
            })
            .map(|f| dir.join(f))
            .collect();
        let sizes = modules
            .iter()
            .map(|m| Ok((fs::metadata(m)?.len(), m)))
            .collect::<Result<Vec<_>>>()?;
        let total = sizes.iter().map(|(size, _)| size).sum();
        let args = ["--validate".as_ref(), dir.as_os_str()];
        let (them, us) = compare(&upstream, &nomi, &home, &args, None)?;
        row(&app.name, modules.len(), total, them, us);

        let (size, largest) = sizes.iter().max().context("an app without modules")?;
        let text = fs::read(largest)?;
        let (them, us) = compare(&upstream, &nomi, &home, &["--stdin".as_ref()], Some(&text))?;
        row(&format!("{} --stdin", app.name), 1, *size, them, us);
    }
    Ok(())
}

/// The fastest run of each tool on `args` and `stdin`.
fn compare(
    upstream: &Path,
    nomi: &Path,
    home: &Path,
    args: &[&std::ffi::OsStr],
    stdin: Option<&[u8]>,
) -> Result<(Duration, Duration)> {
    let mut best = [Duration::MAX; 2];
    let mut codes = [None; 2];
    for _ in 0..RUNS {
        for (i, binary) in [upstream, nomi].into_iter().enumerate() {
            let (time, code) = time(binary, home, args, stdin)?;
            best[i] = best[i].min(time);
            codes[i] = Some(code);
        }
    }
    ensure!(
        codes[0] == codes[1],
        "elm-format {args:?} exits with {:?} but nomi with {:?}",
        codes[0],
        codes[1]
    );
    Ok((best[0], best[1]))
}

fn time(
    binary: &Path,
    home: &Path,
    args: &[&std::ffi::OsStr],
    stdin: Option<&[u8]>,
) -> Result<(Duration, Option<i32>)> {
    use std::io::Write;
    let start = Instant::now();
    let mut child = Command::new(binary)
        .args(args)
        .env("ELM_HOME", home)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .with_context(|| binary.display().to_string())?;
    let mut input = child.stdin.take().expect("piped");
    if let Some(text) = stdin {
        input.write_all(text)?;
    }
    drop(input);
    let status = child.wait()?;
    Ok((start.elapsed(), status.code()))
}

fn row(name: &str, modules: usize, size: u64, them: Duration, us: Duration) {
    println!(
        "| {name:<22} | {modules:>9} | {:>5} KiB | {:>7.1} ms | {:>6.1} ms | {:>6.1}× |",
        size / 1024,
        them.as_secs_f64() * 1e3,
        us.as_secs_f64() * 1e3,
        them.as_secs_f64() / us.as_secs_f64(),
    );
}
