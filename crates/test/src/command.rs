//! elm-test's command line (`elm-test.js`, plan 10.6a): its arguments as
//! commander 9.5 parses them for elm-test, its help, version and errors,
//! and its commands: a run (`RunTests.run`) and `make`.

use std::path::{Path, PathBuf};

use nomi_engine::check::Style;
use nomi_engine::console::Console;
use nomi_engine::paths::Cache;
use nomi_engine::test_deps::Solve;
use nomi_engine::{Http, Vfs};
use nomi_project::Outline;

use crate::build::{self, Env, Failure, Step};
use crate::files::{Project, resolve};
use crate::generate::{Options, Report};
use crate::install::Installed;
use crate::supervise::{self, ELM_TEST_VERSION, Ended, Supervise};

/// The options elm-test takes, before or after its command.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Flags {
    pub watch: bool,
    pub clear_console: bool,
    pub seed: Option<u64>,
    pub fuzz: u32,
    pub workers: Option<usize>,
    pub report: ReportName,
    pub compiler: Option<String>,
    pub oldest: bool,
    pub offline: bool,
}

impl Default for Flags {
    fn default() -> Flags {
        Flags {
            watch: false,
            clear_console: true,
            seed: None,
            fuzz: 100,
            workers: None,
            report: ReportName::Console,
            compiler: None,
            oldest: false,
            offline: false,
        }
    }
}

/// `--report`'s choices.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReportName {
    Console,
    Json,
    JUnit,
}

impl ReportName {
    /// `Report.isMachineReadable`.
    fn machine_readable(self) -> bool {
        self != ReportName::Console
    }
}

/// What the arguments ask for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Parsed {
    /// Text for standard output, then exit 0.
    Out(String),
    /// Text for standard error, then exit 1.
    Err(String),
    Run(Flags, Vec<String>),
    Make(Flags, Vec<String>),
    Init(Flags),
    Install(Flags, String),
}

/// An option of elm-test's program.
struct Spec {
    long: &'static str,
    /// Its value's name, for those that take one.
    value: Option<&'static str>,
    hidden: bool,
}

const fn flag(long: &'static str) -> Spec {
    Spec {
        long,
        value: None,
        hidden: false,
    }
}

const fn valued(long: &'static str, value: &'static str) -> Spec {
    Spec {
        long,
        value: Some(value),
        hidden: false,
    }
}

/// elm-test's options, in the order it declares them.
const OPTIONS: [Spec; 13] = [
    flag("--watch"),
    flag("--no-clear-console"),
    valued("--seed", "<int>"),
    valued("--fuzz", "<int>"),
    valued("--workers", "<int>"),
    valued("--report", "<format>"),
    flag("--no-color"),
    flag("--color"),
    valued("--compiler", "<path>"),
    valued("--dependencies", "<strategy>"),
    flag("--offline"),
    Spec {
        long: "--output",
        value: Some("<output>"),
        hidden: true,
    },
    flag("--version"),
];

const COMMANDS: [&str; 3] = ["init", "install", "make"];

/// The program's help, with the machine's logical cores.
fn program_help(cores: usize) -> String {
    format!(
        r#"Usage: elm-test [options] [globs...]

elm-test
  Run tests in the tests/ folder

elm-test "src/**/*Tests.elm"
  Run tests in files matching the glob

Options:
  --watch                    Run tests on file changes (default: false)
  --no-clear-console         Don't clear the console when running with --watch
  --seed <int>               Run with a specific fuzzer seed (default: random)
  --fuzz <int>               Define how many times each fuzz-test should run
                             (default: 100)
  --workers <int>            Number of worker threads (default: number of
                             logical CPU cores, {cores} on this machine)
  --report <format>          Specify which format to use for reporting test
                             results (choices: "json", "junit", "console",
                             default: "console")
  --no-color                 Disable colored console output (setting
                             FORCE_COLOR=0 also works)
  --color                    Force colored console output (setting FORCE_COLOR
                             to anything but 0 also works)
  --compiler <path>          Use a custom path to an Elm executable (default:
                             elm)
  --dependencies <strategy>  Choose newest or oldest compatible dependencies
                             for a package project (ignored for applications)
                             (choices: "newest", "oldest", default: "newest")
  --offline                  Do not make any HTTP requests when solving
                             dependencies
  --version                  Print version and exit
  -h, --help                 Show help

Commands:
  init                       Install elm-explorations/test and create
                             tests/Example.elm
  install <package>          Like `elm install package`, except it installs to
                             "test-dependencies" in your elm.json
  make [globs...]            Check files matching the globs for compilation
                             errors
  help [command]             Show help
"#
    )
}

/// A command's help.
fn command_help(command: &str) -> String {
    let (usage, description) = match command {
        "init" => (
            "init [options]",
            "Install elm-explorations/test and create tests/Example.elm",
        ),
        "install" => (
            "install [options] <package>",
            "Like `elm install package`, except it installs to \"test-dependencies\" in your elm.json",
        ),
        _ => (
            "make [options] [globs...]",
            "Check files matching the globs for compilation errors",
        ),
    };
    format!("Usage: elm-test {usage}\n\n{description}\n\nOptions:\n  -h, --help  Show help\n")
}

/// `outputOptionError`.
const OUTPUT_ERROR: &str = "Only /dev/null is allowed.

NOTE: Including this option will not have any effect, as `elm-test` does not
produce any output files. It is meant to improve compatibility with editor
plugins explicitly setting the --output flag to /dev/null when using
`elm-test make` as a drop-in replacement for `elm make`.'";

/// Parses elm-test's arguments as commander does, `cores` the machine's
/// logical cores for the help.
pub fn parse(args: &[String], cores: usize) -> Parsed {
    let mut flags = Flags::default();
    // The program's options, wherever they are.
    let (operands, unknown) = match options(args, &mut flags) {
        Ok(Some(split)) => split,
        Ok(None) => return Parsed::Out(format!("{ELM_TEST_VERSION}\n")),
        Err(error) => return Parsed::Err(error),
    };
    let help = |unknown: &[String]| unknown.iter().any(|a| a == "-h" || a == "--help");
    match operands.first().map(String::as_str) {
        Some(command) if COMMANDS.contains(&command) => {
            let (operands, unknown) = rest(&operands[1..], &unknown);
            if help(&unknown) {
                return Parsed::Out(command_help(command));
            }
            if let Some(option) = unknown.first() {
                return Parsed::Err(unknown_option(option));
            }
            match command {
                "init" if operands.is_empty() => Parsed::Init(flags),
                "init" => Parsed::Err(too_many("init", 0, operands.len())),
                "install" => match operands.as_slice() {
                    [] => Parsed::Err("error: missing required argument 'package'".to_owned()),
                    [package] => Parsed::Install(flags, package.clone()),
                    _ => Parsed::Err(too_many("install", 1, operands.len())),
                },
                _ => Parsed::Make(flags, operands),
            }
        }
        Some("help") => match operands.get(1) {
            None => Parsed::Out(program_help(cores)),
            Some(command) if COMMANDS.contains(&command.as_str()) => {
                Parsed::Out(command_help(command))
            }
            Some(_) => Parsed::Err(program_help(cores).trim_end().to_owned()),
        },
        _ => {
            if help(&unknown) {
                return Parsed::Out(program_help(cores));
            }
            let (globs, unknown) = rest(&operands, &unknown);
            if let Some(option) = unknown.first() {
                return Parsed::Err(unknown_option(option));
            }
            Parsed::Run(flags, globs)
        }
    }
}

/// `parseOptions` for the program: its options taken, wherever they are,
/// and the operands and, from the first unknown option on, the rest;
/// `None` for `--version`.
#[allow(clippy::type_complexity)]
fn options(
    args: &[String],
    flags: &mut Flags,
) -> Result<Option<(Vec<String>, Vec<String>)>, String> {
    let (mut operands, mut unknown) = (Vec::new(), Vec::new());
    let mut in_unknown = false;
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        if arg == "--" {
            if in_unknown {
                unknown.push(arg.clone());
                unknown.extend(args.cloned());
            } else {
                operands.extend(args.cloned());
            }
            break;
        }
        let maybe_option = arg.len() > 1 && arg.starts_with('-');
        if maybe_option && let Some(spec) = OPTIONS.iter().find(|s| s.long == arg) {
            let value = match spec.value {
                Some(name) => match args.next() {
                    Some(value) => Some(value.as_str()),
                    None => {
                        return Err(format!(
                            "error: option '{} {name}' argument missing",
                            spec.long
                        ));
                    }
                },
                None => None,
            };
            if !set(flags, spec, value)? {
                return Ok(None);
            }
            continue;
        }
        if let Some(rest) = arg.strip_prefix("--")
            && let Some((name, value)) = rest.split_once('=')
            && !name.is_empty()
            && let Some(spec) = (OPTIONS.iter()).find(|s| s.value.is_some() && s.long[2..] == *name)
        {
            set(flags, spec, Some(value))?;
            continue;
        }
        if maybe_option {
            in_unknown = true;
        }
        if in_unknown {
            unknown.push(arg.clone());
        } else {
            operands.push(arg.clone());
        }
    }
    Ok(Some((operands, unknown)))
}

/// `parseOptions` for a command, which has no options of its own:
/// `operands` and the unknown ones' operands, up to the first unknown
/// option.
fn rest(operands: &[String], unknown: &[String]) -> (Vec<String>, Vec<String>) {
    let mut operands = operands.to_vec();
    let at = (unknown.iter())
        .position(|a| a == "--" || (a.len() > 1 && a.starts_with('-')))
        .unwrap_or(unknown.len());
    if unknown.get(at).is_some_and(|a| a == "--") {
        operands.extend(unknown[..at].iter().cloned());
        operands.extend(unknown[at + 1..].iter().cloned());
        return (operands, Vec::new());
    }
    operands.extend(unknown[..at].iter().cloned());
    (operands, unknown[at..].to_vec())
}

/// Sets what `spec` sets with `value`; false for `--version`.
fn set(flags: &mut Flags, spec: &Spec, value: Option<&str>) -> Result<bool, String> {
    let invalid = |why: &str| {
        format!(
            "error: option '{} {}' argument '{}' is invalid. {why}",
            spec.long,
            spec.value.unwrap_or_default(),
            value.unwrap_or_default()
        )
    };
    let choice = |choices: &[&str]| -> Result<String, String> {
        let value = value.unwrap_or_default();
        if choices.contains(&value) {
            Ok(value.to_owned())
        } else {
            Err(invalid(&format!(
                "Allowed choices are {}.",
                choices.join(", ")
            )))
        }
    };
    match spec.long {
        "--watch" => flags.watch = true,
        "--no-clear-console" => flags.clear_console = false,
        "--seed" => flags.seed = Some(positive(value, 0).map_err(|why| invalid(&why))?),
        "--fuzz" => {
            let fuzz = positive(value, 1).map_err(|why| invalid(&why))?;
            flags.fuzz = u32::try_from(fuzz).unwrap_or(u32::MAX);
        }
        "--workers" => {
            let workers = positive(value, 1).map_err(|why| invalid(&why))?;
            flags.workers = Some(usize::try_from(workers).unwrap_or(usize::MAX));
        }
        "--report" => {
            flags.report = match choice(&["json", "junit", "console"])?.as_str() {
                "json" => ReportName::Json,
                "junit" => ReportName::JUnit,
                _ => ReportName::Console,
            }
        }
        "--compiler" => flags.compiler = value.map(str::to_owned),
        "--dependencies" => flags.oldest = choice(&["newest", "oldest"])? == "oldest",
        "--offline" => flags.offline = true,
        "--output" if value != Some("/dev/null") => return Err(invalid(OUTPUT_ERROR)),
        "--version" => return Ok(false),
        // `--color` and `--no-color` are read from the whole line, as
        // `supports-color` reads them.
        _ => {}
    }
    Ok(true)
}

/// `parsePositiveInteger`.
fn positive(value: Option<&str>, minimum: u64) -> Result<u64, String> {
    let value = value.unwrap_or_default();
    if value.is_empty() || !value.bytes().all(|b| b.is_ascii_digit()) {
        return Err("Expected one or more digits.".to_owned());
    }
    if !value.parse::<f64>().is_ok_and(f64::is_finite) {
        return Err("Expected a finite number.".to_owned());
    }
    let number = value.parse::<u64>().unwrap_or(u64::MAX);
    if number < minimum {
        return Err(format!("Expected at least {minimum}."));
    }
    Ok(number)
}

fn too_many(command: &str, expected: usize, got: usize) -> String {
    let s = if expected == 1 { "" } else { "s" };
    format!(
        "error: too many arguments for '{command}'. Expected {expected} argument{s} but got {got}."
    )
}

/// `unknownOption`, with `suggestSimilar`'s guess for a long option.
fn unknown_option(option: &str) -> String {
    let mut message = format!("error: unknown option '{option}'");
    if let Some(word) = option.strip_prefix("--") {
        let candidates = (OPTIONS.iter())
            .filter(|s| !s.hidden)
            .map(|s| &s.long[2..])
            .chain(["help"]);
        message.push_str(&suggest_similar(word, candidates));
    }
    message
}

/// commander's `suggestSimilar` among options' names.
fn suggest_similar<'a>(word: &str, candidates: impl Iterator<Item = &'a str>) -> String {
    const MAX_DISTANCE: usize = 3;
    let word: Vec<char> = word.chars().collect();
    let mut similar: Vec<&str> = Vec::new();
    let mut best = MAX_DISTANCE;
    let mut seen = Vec::new();
    for candidate in candidates {
        if seen.contains(&candidate) {
            continue;
        }
        seen.push(candidate);
        let chars: Vec<char> = candidate.chars().collect();
        if chars.len() <= 1 {
            continue;
        }
        let distance = edit_distance(&word, &chars, MAX_DISTANCE);
        let length = word.len().max(chars.len());
        let similarity = (length as f64 - distance as f64) / length as f64;
        if similarity > 0.4 {
            if distance < best {
                best = distance;
                similar = vec![candidate];
            } else if distance == best {
                similar.push(candidate);
            }
        }
    }
    similar.sort();
    let similar: Vec<String> = similar.iter().map(|c| format!("--{c}")).collect();
    match similar.as_slice() {
        [] => String::new(),
        [one] => format!("\n(Did you mean {one}?)"),
        many => format!("\n(Did you mean one of {}?)", many.join(", ")),
    }
}

/// Damerau–Levenshtein's optimal string alignment distance, as commander
/// computes it, giving up past `max` length apart.
fn edit_distance(a: &[char], b: &[char], max: usize) -> usize {
    if a.len().abs_diff(b.len()) > max {
        return a.len().max(b.len());
    }
    let mut d = vec![vec![0; b.len() + 1]; a.len() + 1];
    for (i, row) in d.iter_mut().enumerate() {
        row[0] = i;
    }
    for (j, cell) in d[0].iter_mut().enumerate() {
        *cell = j;
    }
    for j in 1..=b.len() {
        for i in 1..=a.len() {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            d[i][j] = (d[i - 1][j] + 1)
                .min(d[i][j - 1] + 1)
                .min(d[i - 1][j - 1] + cost);
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                d[i][j] = d[i][j].min(d[i - 2][j - 2] + 1);
            }
        }
    }
    d[a.len()][b.len()]
}

/// What the command reads of where it runs.
pub struct Context<'a, H> {
    pub vfs: &'a Vfs,
    pub cwd: &'a Path,
    pub cache: &'a Cache,
    pub http: &'a H,
    /// The environment's variables, as `supports-color` reads them.
    pub var: &'a dyn Fn(&str) -> Option<String>,
}

impl<H> std::fmt::Debug for Context<'_, H> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Context")
            .field("cwd", &self.cwd)
            .field("cache", &self.cache.dir())
            .finish_non_exhaustive()
    }
}

/// elm-test run with `args`; its exit code.
pub fn main<H: Http + Sync>(
    context: &Context<'_, H>,
    console: &mut dyn Console,
    args: &[String],
) -> i32 {
    let cores = supervise::default_workers();
    let color = supports_color(args, context.var, console.out_is_terminal());
    match parse(args, cores) {
        Parsed::Out(text) => {
            console.out(&text);
            0
        }
        Parsed::Err(text) => {
            console.err(&format!("{text}\n"));
            1
        }
        Parsed::Run(flags, globs) => run(context, console, &flags, &globs, color, cores),
        Parsed::Make(flags, globs) => make(context, console, &flags, &globs, color),
        Parsed::Init(_) => install(context, console, "init", None),
        Parsed::Install(_, package) => install(context, console, "install", Some(&package)),
    }
}

/// `supports-color`: whether elm-test colors its output.
fn supports_color(args: &[String], var: &dyn Fn(&str) -> Option<String>, terminal: bool) -> bool {
    if let Some(force) = var("FORCE_COLOR").filter(|f| !f.is_empty()) {
        return force != "false" && force != "0";
    }
    if args.iter().any(|a| a == "--no-color") {
        return false;
    }
    if args.iter().any(|a| a == "--color") {
        return true;
    }
    if var("TF_BUILD").is_some() && var("AGENT_NAME").is_some() {
        return true;
    }
    if !terminal {
        return false;
    }
    if cfg!(windows) || var("CI").is_some() {
        return true;
    }
    if var("TERM").as_deref() == Some("dumb") {
        return false;
    }
    ["TERM", "COLORTERM", "TERM_PROGRAM", "TEAMCITY_VERSION"]
        .iter()
        .any(|name| var(name).is_some())
}

/// The project around `cwd`, as `getProjectRootDir` finds it; its error
/// for `command`.
fn project_root(cwd: &Path, command: &str) -> Result<PathBuf, String> {
    let mut dir = Some(cwd);
    while let Some(at) = dir {
        if at.join("elm.json").exists() {
            return Ok(at.to_owned());
        }
        dir = at.parent();
    }
    Err(format!(
        "`{command}` requires an elm.json up the directory tree, but none could be found! To make one: elm init"
    ))
}

/// `ElmJson.requireElmTestPackage`.
fn require_test_package(project: &Project) -> Result<(), String> {
    const TEST: &str = "elm-explorations/test";
    let path = project.root.join("elm.json");
    let version = match &project.outline {
        Outline::App(app) => (app.test_direct.iter().chain(&app.direct))
            .find(|(name, _)| name.to_string() == TEST)
            .map(|(_, version)| version.to_string()),
        Outline::Pkg(pkg) => (pkg.test_deps.iter().chain(&pkg.deps))
            .find(|(name, _)| name.to_string() == TEST)
            .map(|(_, constraint)| constraint.to_string()),
    };
    match version {
        None => Err(format!(
            "{}\nYou must have \"{TEST}\" in your \"test-dependencies\" or \"dependencies\" to run elm-test.",
            path.display()
        )),
        Some(version) if !version.trim_start().starts_with("2.") => Err(format!(
            "{}\nThis version of elm-test only supports {TEST} 2.x, but you have \"{version}\".",
            path.display()
        )),
        Some(_) => Ok(()),
    }
}

/// `makeProgressLogger`: the steps on one line, rewritten as each comes,
/// for people only.
pub(crate) struct Progress {
    items: Vec<String>,
    shown: bool,
    /// Whether `clearConsole` clears it (`--no-clear-console`).
    clears: bool,
}

impl Progress {
    pub(crate) fn log(&mut self, console: &mut dyn Console, item: &str) {
        self.items.push(item.to_owned());
        if self.shown {
            console.out(&format!("{}\r", self.items.join(" > ")));
        }
    }

    pub(crate) fn new_line(&mut self, console: &mut dyn Console) {
        self.items.clear();
        if self.shown {
            console.out("\n");
        }
    }

    /// Whether it shows: for people, not machines.
    pub(crate) fn shown(&self) -> bool {
        self.shown
    }

    /// Whether clearing the console clears it.
    pub(crate) fn clears(&self) -> bool {
        self.clears
    }

    /// `readline.clearLine`.
    pub(crate) fn clear_line(&mut self, console: &mut dyn Console) {
        self.items.clear();
        if self.shown {
            console.out("\x1b[2K");
        }
    }

    pub(crate) fn clear_console(&mut self, console: &mut dyn Console) {
        self.items.clear();
        if self.shown && self.clears {
            console.out(if cfg!(windows) {
                "\x1b[2J\x1b[0f"
            } else {
                "\x1b[2J\x1b[3J\x1b[H"
            });
        }
    }
}

/// `RunTests.runTests`: one run, or with `--watch` a run each time the
/// tests change.
fn run<H: Http + Sync>(
    context: &Context<'_, H>,
    console: &mut dyn Console,
    flags: &Flags,
    globs: &[String],
    color: bool,
    cores: usize,
) -> i32 {
    let root = match project_root(context.cwd, "elm-test") {
        Ok(root) => root,
        Err(message) => {
            console.err(&format!("{message}\n"));
            return 1;
        }
    };
    let mut progress = Progress {
        items: Vec::new(),
        shown: !flags.report.machine_readable(),
        clears: flags.clear_console,
    };
    let mut once = |console: &mut dyn Console, progress: &mut Progress| {
        run_once(
            context, console, progress, &root, flags, globs, color, cores,
        )
    };
    if flags.watch {
        crate::watch::watch(console, &mut progress, &root, color, &mut once)
    } else {
        once(console, &mut progress).0
    }
}

/// `RunTests.run`: its exit code, and the test source directories it
/// found.
#[allow(clippy::too_many_arguments)]
fn run_once<H: Http + Sync>(
    context: &Context<'_, H>,
    console: &mut dyn Console,
    progress: &mut Progress,
    root: &Path,
    flags: &Flags,
    globs: &[String],
    color: bool,
    cores: usize,
) -> (i32, Vec<PathBuf>) {
    // Each error ends the run after a line break, as elm-test's `catch`
    // does.
    let fail = |console: &mut dyn Console, progress: &mut Progress, message: &str| {
        progress.new_line(console);
        console.err(&format!("{message}\n"));
        1
    };
    let project = Project::read(root).and_then(|project| {
        require_test_package(&project)?;
        project.validate()?;
        Ok(project)
    });
    let project = match project {
        Ok(project) => project,
        Err(message) => return (fail(console, progress, &message), Vec::new()),
    };
    let dirs = project.source_dirs.clone();
    let code = run_project(
        context, console, progress, &project, flags, globs, color, cores,
    );
    (code, dirs)
}

/// The rest of `RunTests.run`, once the project is read.
#[allow(clippy::too_many_arguments)]
fn run_project<H: Http + Sync>(
    context: &Context<'_, H>,
    console: &mut dyn Console,
    progress: &mut Progress,
    project: &Project,
    flags: &Flags,
    globs: &[String],
    color: bool,
    cores: usize,
) -> i32 {
    let fail = |console: &mut dyn Console, progress: &mut Progress, message: &str| {
        progress.new_line(console);
        console.err(&format!("{message}\n"));
        1
    };
    let files = match resolve(project, context.cwd, globs) {
        Ok(files) => files,
        Err(message) => return fail(console, progress, &message),
    };
    let processes = flags.workers.unwrap_or(cores);
    let options = Options {
        runs: flags.fuzz,
        report: match flags.report {
            ReportName::Console => Report::Console { color },
            ReportName::Json => Report::Json,
            ReportName::JUnit => Report::JUnit,
        },
        seed: flags.seed.unwrap_or_else(supervise::random_seed),
        processes,
        globs: globs.to_vec(),
        paths: files.iter().map(|f| f.display().to_string()).collect(),
    };
    let env = Env {
        vfs: context.vfs,
        cache: context.cache,
        http: context.http,
        solve: Solve {
            oldest: flags.oldest,
            offline: flags.offline,
        },
        color,
    };
    let built = build::build(
        &env,
        console,
        project,
        &files,
        &options,
        &mut |console, step| {
            let item = match step {
                Step::Solving => "Solving dependencies",
                Step::Compiling => "Compiling",
            };
            progress.log(console, item);
        },
    );
    let runner = match built {
        Ok(runner) => runner,
        Err(Failure::Files(message)) => return fail(console, progress, &message),
        Err(Failure::Project(report)) => {
            progress.new_line(console);
            nomi_engine::console::report(console, &report, nomi_base::Compat::LATEST);
            return 1;
        }
        Err(Failure::Shown) => {
            return fail(console, progress, "`elm make` failed with exit code 1.");
        }
    };
    progress.log(console, "Starting tests");
    progress.new_line(console);
    let supervise = Supervise {
        report: options.report,
        processes,
        color,
    };
    match supervise::run(console, &runner.js, supervise) {
        Ok(Ended::Code(code)) => code,
        Ok(Ended::Thrown) => {
            progress.new_line(console);
            1
        }
        Err(e) => fail(console, progress, &format!("{e}")),
    }
}

/// `elm-test init`, or `install` of `package`.
fn install<H: Http + Sync>(
    context: &Context<'_, H>,
    console: &mut dyn Console,
    command: &str,
    package: Option<&str>,
) -> i32 {
    let project = (project_root(context.cwd, &format!("elm-test {command}")))
        .and_then(|root| Project::read(&root));
    let project = match project {
        Ok(project) => project,
        Err(message) => {
            console.err(&format!("{message}\n"));
            return 1;
        }
    };
    let env = Env {
        vfs: context.vfs,
        cache: context.cache,
        http: context.http,
        solve: Solve::default(),
        color: false,
    };
    let done = match package {
        None => crate::install::init(&env, console, &project).map(|()| Installed::Added),
        Some(package) => crate::install::install(&env, console, &project, package),
    };
    match done {
        Ok(Installed::Already) if package.is_some() => {
            console.out("It is already installed!\n");
            0
        }
        Ok(_) => 0,
        Err(Some(message)) => {
            console.err(&format!("{message}\n"));
            1
        }
        Err(None) => 1,
    }
}

/// `elm-test make`.
fn make<H: Http + Sync>(
    context: &Context<'_, H>,
    console: &mut dyn Console,
    flags: &Flags,
    globs: &[String],
    color: bool,
) -> i32 {
    let project = project_root(context.cwd, "elm-test make").and_then(|root| Project::read(&root));
    let project = match project {
        Ok(project) => project,
        Err(message) => {
            console.err(&format!("{message}\n"));
            return 1;
        }
    };
    // `resolveGlobs`, which finding nothing doesn't stop here.
    let files = resolve(&project, context.cwd, globs).unwrap_or_default();
    let env = Env {
        vfs: context.vfs,
        cache: context.cache,
        http: context.http,
        solve: Solve {
            oldest: flags.oldest,
            offline: flags.offline,
        },
        color,
    };
    let style = match flags.report {
        ReportName::Json => Style::Json,
        _ => Style::Terminal,
    };
    let made = if flags.report.machine_readable() {
        build::make(&env, &mut Silent(console), &project, &files, style)
    } else {
        build::make(&env, console, &project, &files, style)
    };
    match made {
        Ok(true) => 0,
        Ok(false) => 1,
        Err(Failure::Files(message)) => {
            console.err(&format!("{message}\n"));
            1
        }
        Err(Failure::Project(report)) => {
            nomi_engine::console::report(console, &report, nomi_base::Compat::LATEST);
            1
        }
        Err(Failure::Shown) => 1,
    }
}

/// `console` without standard output, as elm-test runs `elm make` for a
/// machine-readable report.
struct Silent<'a>(&'a mut dyn Console);

impl Console for Silent<'_> {
    fn out(&mut self, _: &str) {}

    fn err(&mut self, text: &str) {
        self.0.err(text);
    }

    fn read_line(&mut self) -> std::io::Result<Option<String>> {
        self.0.read_line()
    }

    fn read_all(&mut self) -> std::io::Result<Vec<u8>> {
        self.0.read_all()
    }

    fn err_is_terminal(&self) -> bool {
        self.0.err_is_terminal()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_line(line: &str) -> Parsed {
        let args: Vec<String> = line.split_whitespace().map(str::to_owned).collect();
        parse(&args, 8)
    }

    fn err(line: &str) -> String {
        match parse_line(line) {
            Parsed::Err(text) => text,
            other => panic!("{line}: {other:?}"),
        }
    }

    /// What elm-test 0.19.3-0 answered to each.
    #[test]
    fn arguments_are_parsed_as_commander_parses_them() {
        assert_eq!(
            err("--seed"),
            "error: option '--seed <int>' argument missing"
        );
        assert_eq!(
            err("--fuzz 0"),
            "error: option '--fuzz <int>' argument '0' is invalid. Expected at least 1."
        );
        assert_eq!(
            err("--seed 5 --seed abc"),
            "error: option '--seed <int>' argument 'abc' is invalid. Expected one or more digits."
        );
        assert_eq!(
            err("--dependencies x"),
            "error: option '--dependencies <strategy>' argument 'x' is invalid. Allowed choices are newest, oldest."
        );
        assert_eq!(err("-x"), "error: unknown option '-x'");
        assert_eq!(err("-hx"), "error: unknown option '-hx'");
        assert_eq!(
            err("--no-color=1"),
            "error: unknown option '--no-color=1'\n(Did you mean --no-color?)"
        );
        assert_eq!(err("--seed=5 --frob"), "error: unknown option '--frob'");
        assert_eq!(err("make --frob"), "error: unknown option '--frob'");
        assert_eq!(err("install"), "error: missing required argument 'package'");
        assert_eq!(
            err("install a b"),
            "error: too many arguments for 'install'. Expected 1 argument but got 2."
        );
        assert_eq!(
            err("init x"),
            "error: too many arguments for 'init'. Expected 0 arguments but got 1."
        );
        assert!(err("--output foo").ends_with("as a drop-in replacement for `elm make`.'"));
        assert!(err("help nope").starts_with("Usage: elm-test [options] [globs...]"));
        assert_eq!(
            parse_line("--version extra"),
            Parsed::Out("0.19.3-0\n".to_owned())
        );
        assert_eq!(parse_line("make --help"), Parsed::Out(command_help("make")));
        assert_eq!(parse_line("help init"), Parsed::Out(command_help("init")));
        assert_eq!(parse_line("-h make"), Parsed::Out(program_help(8)));
        let Parsed::Run(flags, globs) = parse_line("--seed=5 tests/A.elm --report json -- --x")
        else {
            panic!()
        };
        assert_eq!((flags.seed, flags.report), (Some(5), ReportName::Json));
        assert_eq!(globs, ["tests/A.elm", "--x"]);
        let Parsed::Make(flags, globs) = parse_line("make --offline src/A.elm") else {
            panic!()
        };
        assert!(flags.offline);
        assert_eq!(globs, ["src/A.elm"]);
    }
}
