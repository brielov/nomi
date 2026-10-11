//! `cargo xtask bench elm`: nomi, run as `elm`, against the pinned elm
//! 0.19.1, building each corpus app's entries with `elm make`, end to end.
//! A cold build starts without `elm-stuff` or any built dependency: elm's
//! `artifacts.dat` files are removed from its `ELM_HOME`, and nomi's
//! artifact cache is emptied. A warm build follows it with nothing changed.
//! Both tools run offline over the same package sources, each in its own
//! copy of the app, and must exit alike and write the same JavaScript. The
//! fastest of [`RUNS`] runs counts for wall time, and the least CPU time
//! and peak memory of any run for those, as `/usr/bin/time` reports them.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail, ensure};

use crate::corpus::compat::{self, Version};
use crate::corpus::{apps, corpus_dir, files_below};
use crate::oracle::cache_dir;
use crate::oracle::corpus_runs::{copy_modules, elm_binary, elm_home, releases};
use crate::oracle::sandbox::offline;
use crate::references::References;

const RUNS: usize = 3;

/// What one build cost.
#[derive(Clone, Copy)]
struct Cost {
    wall: Duration,
    cpu: Duration,
    /// Peak resident memory, in KiB.
    memory: u64,
}

impl Cost {
    const MAX: Cost = Cost {
        wall: Duration::MAX,
        cpu: Duration::MAX,
        memory: u64::MAX,
    };

    fn min(self, other: Cost) -> Cost {
        Cost {
            wall: self.wall.min(other.wall),
            cpu: self.cpu.min(other.cpu),
            memory: self.memory.min(other.memory),
        }
    }
}

/// One tool's side of the comparison.
struct Tool {
    name: &'static str,
    binary: PathBuf,
    env: Vec<(&'static str, PathBuf)>,
    /// Removes what the tool built, for a cold build.
    clear: Box<dyn Fn() -> Result<()>>,
}

pub fn run(root: &Path, refs: &References) -> Result<()> {
    let compat = Version::parse("0.19.1")?;
    let scratch = cache_dir()?.join("sandboxes").join("bench-elm");
    let store = cache_dir()?.join("nomi");
    let releases = releases(&store.join("packages"))?;
    ensure!(
        !releases.is_empty(),
        "the development cache has no releases; run `cargo xtask corpus names`"
    );
    crate::cargo(
        root,
        &["build", "--locked", "--release", "--package", "nomi-cli"],
    )?;
    let bin = scratch.join("bin");
    fs::create_dir_all(&bin)?;
    let nomi = bin.join("elm");
    fs::copy(root.join("target/release/nomi"), &nomi)?;

    // elm's home holds every release the development cache has, kept
    // between runs as the corpus oracle keeps its own.
    let home = scratch.join("home");
    elm_home(&home, compat, &releases)?;
    let home_packages = home.join(compat.to_string()).join("packages");
    // nomi's cache reads the development cache's packages and registry.
    let cache = scratch.join("cache");
    if cache.exists() {
        fs::remove_dir_all(&cache)?;
    }
    fs::create_dir_all(&cache)?;
    link_or_copy(&store.join("packages"), &cache.join("packages"))?;
    link_or_copy(&store.join("registry"), &cache.join("registry"))?;
    let artifacts = cache.join("artifacts");
    let tools = [
        Tool {
            name: "elm",
            binary: elm_binary(refs, compat)?,
            env: vec![("ELM_HOME", home.clone())],
            clear: Box::new(move || {
                for file in files_below(&home_packages)? {
                    if file.ends_with("artifacts.dat") {
                        fs::remove_file(home_packages.join(file))?;
                    }
                }
                Ok(())
            }),
        },
        Tool {
            name: "nomi",
            binary: nomi,
            env: vec![("NOMI_CACHE", cache.clone())],
            clear: Box::new(move || match fs::remove_dir_all(&artifacts) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.into()),
                _ => Ok(()),
            }),
        },
    ];

    let classification = compat::load(root)?;
    let corpus = corpus_dir()?;
    println!(
        "| build             |    elm |   nomi | speedup | elm CPU | nomi CPU | elm memory | nomi memory |"
    );
    println!(
        "|-------------------|--------|--------|---------|---------|----------|------------|-------------|"
    );
    for app in apps::load(root)?.app {
        let top = apps::app_dir(&corpus, &app);
        ensure!(
            top.exists(),
            "{} is missing; run `cargo xtask corpus fetch`",
            top.display()
        );
        let manifest = compat::sandbox_manifest(&classification, &app, compat)?;
        let mut costs = [[Cost::MAX; 2]; 2];
        let mut outputs = Vec::new();
        for (t, tool) in tools.iter().enumerate() {
            let work = scratch.join("work").join(tool.name).join(&app.name);
            if work.exists() {
                fs::remove_dir_all(&work)?;
            }
            copy_modules(&top, &work)?;
            let dir = work.join(&app.root);
            fs::create_dir_all(&dir)?;
            fs::write(dir.join("elm.json"), &manifest)?;
            let output = scratch.join(format!("{}-{}.js", tool.name, app.name));
            let mut args: Vec<String> = vec!["make".to_owned()];
            args.extend(app.entries.iter().cloned());
            args.push(format!("--output={}", output.display()));
            for _ in 0..RUNS {
                (tool.clear)()?;
                let _ = fs::remove_dir_all(dir.join("elm-stuff"));
                for (build, cost) in costs.iter_mut().enumerate() {
                    let measured = measure(tool, &dir, &args, &scratch.join("time.txt"))
                        .with_context(|| {
                            let kind = ["cold", "warm"][build];
                            format!("{} {kind} on {}", tool.name, app.name)
                        })?;
                    cost[t] = cost[t].min(measured);
                }
            }
            outputs.push(fs::read(&output)?);
        }
        ensure!(
            outputs[0] == outputs[1],
            "elm and nomi write different JavaScript for {}",
            app.name
        );
        for (build, [them, us]) in ["cold", "warm"].into_iter().zip(costs) {
            row(&format!("{} {build}", app.name), them, us);
        }
    }
    Ok(())
}

/// `from` at `to`, as a link where links are made without privileges.
fn link_or_copy(from: &Path, to: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(from, to)?;
    }
    #[cfg(not(unix))]
    {
        for file in files_below(from)? {
            let target = to.join(&file);
            fs::create_dir_all(target.parent().unwrap())?;
            fs::copy(from.join(&file), &target)?;
        }
    }
    Ok(())
}

/// One build by `tool` in `dir`, which must succeed, under `/usr/bin/time`
/// writing to `report`.
fn measure(tool: &Tool, dir: &Path, args: &[String], report: &Path) -> Result<Cost> {
    let mut command = Command::new("/usr/bin/time");
    command.arg("-o").arg(report);
    command.arg(if cfg!(target_os = "macos") {
        "-l"
    } else {
        "-v"
    });
    command
        .arg(&tool.binary)
        .args(args)
        .current_dir(dir)
        .env("ALL_PROXY", "http://127.0.0.1:9")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    offline(&mut command);
    for (key, value) in &tool.env {
        command.env(key, value);
    }
    let start = Instant::now();
    let output = command.output().context("running /usr/bin/time")?;
    let wall = start.elapsed();
    ensure!(
        output.status.success(),
        "exits with {:?}: {}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr)
    );
    let (cpu, memory) = parse_time(&fs::read_to_string(report)?)?;
    Ok(Cost { wall, cpu, memory })
}

/// User plus system time and peak memory in KiB, from GNU time's `-v` or
/// BSD time's `-l` report.
fn parse_time(report: &str) -> Result<(Duration, u64)> {
    let mut cpu = 0.0;
    let mut memory = None;
    for line in report.lines().map(str::trim) {
        let number = |text: &str| -> Result<f64> {
            text.trim()
                .parse()
                .with_context(|| format!("reading {text:?} in /usr/bin/time's report"))
        };
        if let Some(seconds) = (line.strip_prefix("User time (seconds):"))
            .or_else(|| line.strip_prefix("System time (seconds):"))
        {
            cpu += number(seconds)?;
        } else if let Some(kib) = line.strip_prefix("Maximum resident set size (kbytes):") {
            memory = Some(number(kib)? as u64);
        } else if line.ends_with(" sys") && line.contains(" real ") {
            let words: Vec<&str> = line.split_whitespace().collect();
            if let [_, "real", user, "user", sys, "sys"] = words[..] {
                cpu += number(user)? + number(sys)?;
            }
        } else if let Some(bytes) = line.strip_suffix("maximum resident set size") {
            memory = Some(number(bytes)? as u64 / 1024);
        }
    }
    match memory {
        Some(memory) => Ok((Duration::from_secs_f64(cpu), memory)),
        None => bail!("/usr/bin/time reported no peak memory:\n{report}"),
    }
}

fn row(name: &str, them: Cost, us: Cost) {
    let seconds = |d: Duration| format!("{:.2} s", d.as_secs_f64());
    let mib = |kib: u64| format!("{} MiB", kib / 1024);
    println!(
        "| {name:<17} | {:>6} | {:>6} | {:>6.2}× | {:>7} | {:>8} | {:>10} | {:>11} |",
        seconds(them.wall),
        seconds(us.wall),
        them.wall.as_secs_f64() / us.wall.as_secs_f64(),
        seconds(them.cpu),
        seconds(us.cpu),
        mib(them.memory),
        mib(us.memory),
    );
}
