//! What one run of a tool costs: wall time from process start to exit, and
//! CPU time and peak memory as `/usr/bin/time` reports them (GNU time's
//! `-v`, BSD time's `-l`), since the standard library has no `wait4`.

use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail, ensure};

use crate::oracle::sandbox::offline;

/// How long one run may take before it counts as hung.
const TIMEOUT: Duration = Duration::from_secs(600);

/// What one run cost.
#[derive(Clone, Copy, Debug)]
pub struct Cost {
    pub wall: Duration,
    pub cpu: Duration,
    /// Peak resident memory, in KiB.
    pub memory: u64,
}

impl Cost {
    pub const MAX: Cost = Cost {
        wall: Duration::MAX,
        cpu: Duration::MAX,
        memory: u64::MAX,
    };

    /// The least of each measure.
    pub fn min(self, other: Cost) -> Cost {
        Cost {
            wall: self.wall.min(other.wall),
            cpu: self.cpu.min(other.cpu),
            memory: self.memory.min(other.memory),
        }
    }
}

/// A command to measure, offline.
pub struct Run<'a> {
    pub binary: &'a Path,
    pub args: Vec<OsString>,
    pub dir: &'a Path,
    pub env: &'a [(&'static str, PathBuf)],
    pub stdin: Option<&'a [u8]>,
    /// Where `/usr/bin/time` writes its report.
    pub report: &'a Path,
}

/// Runs `run` once: what it cost and its exit code, `None` when a signal
/// ended it.
pub fn measure(run: &Run<'_>) -> Result<(Cost, Option<i32>)> {
    use std::io::Write;
    let mut command = Command::new("/usr/bin/time");
    command.arg("-o").arg(run.report);
    command.arg(if cfg!(target_os = "macos") {
        "-l"
    } else {
        "-v"
    });
    command
        .arg(run.binary)
        .args(&run.args)
        .current_dir(run.dir)
        .env("ALL_PROXY", "http://127.0.0.1:9")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    offline(&mut command);
    for (key, value) in run.env {
        command.env(key, value);
    }
    let start = Instant::now();
    let mut child = command.spawn().context("running /usr/bin/time")?;
    let mut input = child.stdin.take().expect("piped");
    if let Some(text) = run.stdin {
        input.write_all(text)?;
    }
    drop(input);
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if start.elapsed() > TIMEOUT {
            child.kill()?;
            child.wait()?;
            bail!(
                "{} {:?} ran past {TIMEOUT:?}",
                run.binary.display(),
                run.args
            );
        }
        std::thread::sleep(Duration::from_millis(1));
    };
    let wall = start.elapsed();
    let report = fs::read_to_string(run.report)?;
    let (cpu, memory) = parse_time(&report)?;
    let signalled = report.contains("terminated by signal");
    let code = if signalled { None } else { status.code() };
    Ok((Cost { wall, cpu, memory }, code))
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

/// The best of `runs` runs of `run` for each measure, which must exit
/// alike every time: the exit code with the costs.
pub fn best_of(
    runs: usize,
    mut each: impl FnMut() -> Result<(Cost, Option<i32>)>,
) -> Result<(Cost, Option<i32>)> {
    let mut best = Cost::MAX;
    let mut code = None;
    for i in 0..runs {
        let (cost, exit) = each()?;
        ensure!(
            i == 0 || exit == code,
            "exits with {exit:?}, before with {code:?}"
        );
        best = best.min(cost);
        code = exit;
    }
    Ok((best, code))
}

/// The machine, as a scoreboard records it: processor, threads, memory and
/// target.
pub fn machine() -> String {
    let threads = std::thread::available_parallelism().map_or(0, |n| n.get());
    let target = format!("{}-{}", std::env::consts::ARCH, std::env::consts::OS);
    let read = |path: &str| fs::read_to_string(path).unwrap_or_default();
    let field = |text: &str, key: &str| -> Option<String> {
        text.lines()
            .find(|l| l.starts_with(key))
            .and_then(|l| l.split_once(':'))
            .map(|(_, v)| v.trim().to_owned())
    };
    let cpu = field(&read("/proc/cpuinfo"), "model name").or_else(|| {
        let out = Command::new("sysctl")
            .args(["-n", "machdep.cpu.brand_string"])
            .output()
            .ok()?;
        Some(String::from_utf8_lossy(&out.stdout).trim().to_owned())
    });
    let memory = field(&read("/proc/meminfo"), "MemTotal")
        .and_then(|m| m.trim_end_matches(" kB").trim().parse::<u64>().ok())
        .map(|kib| kib * 1024)
        .or_else(|| {
            let out = Command::new("sysctl")
                .args(["-n", "hw.memsize"])
                .output()
                .ok()?;
            String::from_utf8_lossy(&out.stdout).trim().parse().ok()
        });
    let memory = memory.map_or_else(String::new, |bytes| {
        format!(", {} GB", (bytes as f64 / 1e9).round())
    });
    format!(
        "{} ({target}, {threads} threads{memory})",
        cpu.unwrap_or_else(|| "an unknown processor".to_owned())
    )
}
