//! `NOMI_TIMINGS=1`: where a build's time goes. Stages run one
//! after another on the build's thread, so their wall times add up to the
//! build's; work runs on the pool, so its times, added across threads, come
//! to about the build's CPU time. Off, a phase costs one load of a flag.

use std::fmt::Write as _;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

/// What a build spends its time on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    // Stages.
    /// Reading `elm.json` and the dependencies' interfaces.
    Load,
    /// Building the dependencies.
    Packages,
    /// Finding and checking the project's modules, as the dependencies
    /// build.
    Modules,
    /// Merging the graphs and generating the code.
    Generate,
    /// Writing the output.
    Write,
    // Work.
    /// Reading and parsing a module, and validating its syntax.
    Parse,
    /// Lowering the syntax tree.
    Lower,
    Canonicalize,
    Types,
    /// The checks after types: patterns, `main` and documentation.
    Checks,
    Optimize,
    /// Making and encoding a module's interface.
    Interface,
    CacheRead,
    CacheWrite,
}

const STAGES: [Phase; 5] = [
    Phase::Load,
    Phase::Packages,
    Phase::Modules,
    Phase::Generate,
    Phase::Write,
];

const WORK: [Phase; 9] = [
    Phase::Parse,
    Phase::Lower,
    Phase::Canonicalize,
    Phase::Types,
    Phase::Checks,
    Phase::Optimize,
    Phase::Interface,
    Phase::CacheRead,
    Phase::CacheWrite,
];

const COUNT: usize = STAGES.len() + WORK.len();

static TOTALS: [AtomicU64; COUNT] = [const { AtomicU64::new(0) }; COUNT];
static TIMES: [AtomicU64; COUNT] = [const { AtomicU64::new(0) }; COUNT];

/// Each module checked, with how long it took.
static MODULES: Mutex<Vec<(String, Duration)>> = Mutex::new(Vec::new());

/// How many of the slowest modules the report lists.
const SLOWEST: usize = 10;

/// Whether `NOMI_TIMINGS` asks for timings.
pub fn on() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("NOMI_TIMINGS").is_some_and(|v| !v.is_empty() && v != "0"))
}

/// Runs `f`, counting its time to `phase`.
pub fn time<T>(phase: Phase, f: impl FnOnce() -> T) -> T {
    if !on() {
        return f();
    }
    let start = Instant::now();
    let result = f();
    add(phase, start.elapsed());
    result
}

/// Runs `f`, the whole of checking `module`, for the list of the slowest.
pub fn module<T>(module: &str, f: impl FnOnce() -> T) -> T {
    if !on() {
        return f();
    }
    let start = Instant::now();
    let result = f();
    let took = start.elapsed();
    MODULES.lock().unwrap().push((module.to_owned(), took));
    result
}

fn add(phase: Phase, took: Duration) {
    let i = phase as usize;
    let nanos = u64::try_from(took.as_nanos()).unwrap_or(u64::MAX);
    TOTALS[i].fetch_add(nanos, Ordering::Relaxed);
    TIMES[i].fetch_add(1, Ordering::Relaxed);
}

/// The table of what was counted, for a build that took `wall`.
pub fn report(wall: Duration) -> String {
    let mut out = String::new();
    let ms = |nanos: u64| nanos as f64 / 1e6;
    let mut section = |title: &str, phases: &[Phase]| {
        let _ = writeln!(out, "{title}");
        let mut sum = 0;
        for &phase in phases {
            let total = TOTALS[phase as usize].load(Ordering::Relaxed);
            let times = TIMES[phase as usize].load(Ordering::Relaxed);
            sum += total;
            if times > 0 {
                let _ = writeln!(
                    out,
                    "  {:<14} {:>10.1} ms {times:>8}×",
                    format!("{phase:?}"),
                    ms(total)
                );
            }
        }
        let _ = writeln!(out, "  {:<14} {:>10.1} ms", "total", ms(sum));
    };
    section("stages (wall)", &STAGES);
    section("work (all threads)", &WORK);
    let mut modules = MODULES.lock().unwrap().clone();
    if !modules.is_empty() {
        modules.sort_by_key(|m| std::cmp::Reverse(m.1));
        let _ = writeln!(out, "slowest modules");
        for (name, took) in modules.iter().take(SLOWEST) {
            let _ = writeln!(out, "  {:>10.1} ms  {name}", took.as_secs_f64() * 1e3);
        }
    }
    let _ = writeln!(out, "build {:.1} ms", wall.as_secs_f64() * 1e3);
    out
}
