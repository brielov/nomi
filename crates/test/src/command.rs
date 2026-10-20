//! elm-test's command line (`elm-test.js`): its arguments as
//! commander 9.5 parses them for elm-test, its help, version and errors,
//! and its commands: a run (`RunTests.run`) and `make`.

use std::path::{Path, PathBuf};

use nomi_base::Compat;
use nomi_engine::check::Style;
use nomi_engine::console::Console;
use nomi_engine::packages::Resolve;
use nomi_engine::scope::{self, Scope};
use nomi_engine::test_deps::{Order, Solve};
use nomi_engine::{Context, Http};
use nomi_project::{Name, Outline};

use crate::TEST_PACKAGE;
use crate::build::{self, Env, Failure, Step};
use crate::files::{Project, resolve};
use crate::generate::{Options, Report};
use crate::install::{self, InstallError, Installed};
use crate::supervise::{self, ELM_TEST_VERSION, Ended, Supervise};
use crate::watch;

/// The options elm-test takes, before or after its command.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Flags {
    pub watch: bool,
    /// Off with `--no-clear-console`.
    pub clear_console: bool,
    pub seed: Option<u64>,
    pub fuzz: u32,
    pub workers: Option<usize>,
    pub report: ReportName,
    /// `--dependencies` and `--offline`.
    pub solve: Solve,
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
            solve: Solve::default(),
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
    /// A run of the tests the globs find.
    Run(Flags, Vec<String>),
    /// `make` of the files the globs find.
    Make(Flags, Vec<String>),
    /// `init` or `install`.
    Setup(Setup),
}

/// What `init` and `install` add to the test dependencies.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Setup {
    /// elm-explorations/test, and `tests/Example.elm`.
    Init,
    /// The package named.
    Install(String),
}

/// elm-test's commands, but `help`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Command {
    Init,
    Install,
    Make,
}

impl Command {
    /// The command called `name`.
    fn named(name: &str) -> Option<Command> {
        [Command::Init, Command::Install, Command::Make]
            .into_iter()
            .find(|command| command.name() == name)
    }

    fn name(self) -> &'static str {
        match self {
            Command::Init => "init",
            Command::Install => "install",
            Command::Make => "make",
        }
    }

    fn help(self) -> String {
        let (usage, description) = match self {
            Command::Init => (
                "init [options]",
                "Install elm-explorations/test and create tests/Example.elm",
            ),
            Command::Install => (
                "install [options] <package>",
                "Like `elm install package`, except it installs to \"test-dependencies\" in your elm.json",
            ),
            Command::Make => (
                "make [options] [globs...]",
                "Check files matching the globs for compilation errors",
            ),
        };
        format!("Usage: elm-test {usage}\n\n{description}\n\nOptions:\n  -h, --help  Show help\n")
    }
}

/// What an option of elm-test's program sets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Opt {
    Watch,
    NoClearConsole,
    Seed,
    Fuzz,
    Workers,
    Report,
    NoColor,
    Color,
    Compiler,
    Dependencies,
    Offline,
    Output,
    Version,
}

/// An option of elm-test's program.
struct Spec {
    opt: Opt,
    long: &'static str,
    /// Its value's name, for those that take one.
    value: Option<&'static str>,
}

const fn flag(opt: Opt, long: &'static str) -> Spec {
    Spec {
        opt,
        long,
        value: None,
    }
}

const fn valued(opt: Opt, long: &'static str, value: &'static str) -> Spec {
    Spec {
        opt,
        long,
        value: Some(value),
    }
}

/// elm-test's options, in the order it declares them.
const OPTIONS: [Spec; 13] = [
    flag(Opt::Watch, "--watch"),
    flag(Opt::NoClearConsole, "--no-clear-console"),
    valued(Opt::Seed, "--seed", "<int>"),
    valued(Opt::Fuzz, "--fuzz", "<int>"),
    valued(Opt::Workers, "--workers", "<int>"),
    valued(Opt::Report, "--report", "<format>"),
    flag(Opt::NoColor, "--no-color"),
    flag(Opt::Color, "--color"),
    valued(Opt::Compiler, "--compiler", "<path>"),
    valued(Opt::Dependencies, "--dependencies", "<strategy>"),
    flag(Opt::Offline, "--offline"),
    valued(Opt::Output, "--output", "<output>"),
    flag(Opt::Version, "--version"),
];

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
        Ok(Scan::Args { operands, unknown }) => (operands, unknown),
        Ok(Scan::Version) => return Parsed::Out(format!("{ELM_TEST_VERSION}\n")),
        Err(error) => return Parsed::Err(error),
    };
    let help = |unknown: &[String]| unknown.iter().any(|a| a == "-h" || a == "--help");
    let first = operands.first().map(String::as_str);
    if let Some(command) = first.and_then(Command::named) {
        let (operands, unknown) = rest(&operands[1..], &unknown);
        if help(&unknown) {
            return Parsed::Out(command.help());
        }
        if let Some(option) = unknown.first() {
            return Parsed::Err(unknown_option(option));
        }
        return match command {
            Command::Init if operands.is_empty() => Parsed::Setup(Setup::Init),
            Command::Init => Parsed::Err(too_many(command, 0, operands.len())),
            Command::Install => match operands.as_slice() {
                [] => Parsed::Err("error: missing required argument 'package'".to_owned()),
                [package] => Parsed::Setup(Setup::Install(package.clone())),
                _ => Parsed::Err(too_many(command, 1, operands.len())),
            },
            Command::Make => Parsed::Make(flags, operands),
        };
    }
    if first == Some("help") {
        return match operands.get(1) {
            None => Parsed::Out(program_help(cores)),
            Some(name) => match Command::named(name) {
                Some(command) => Parsed::Out(command.help()),
                None => Parsed::Err(program_help(cores).trim_end().to_owned()),
            },
        };
    }
    if help(&unknown) {
        return Parsed::Out(program_help(cores));
    }
    let (globs, unknown) = rest(&operands, &unknown);
    if let Some(option) = unknown.first() {
        return Parsed::Err(unknown_option(option));
    }
    Parsed::Run(flags, globs)
}

/// What the program's options leave of its arguments.
enum Scan {
    /// `--version`, which ends the parse.
    Version,
    /// The operands and, from the first unknown option on, the rest.
    Args {
        operands: Vec<String>,
        unknown: Vec<String>,
    },
}

/// `parseOptions` for the program: its options taken into `flags`,
/// wherever they are.
fn options(args: &[String], flags: &mut Flags) -> Result<Scan, String> {
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
            if spec.opt == Opt::Version {
                return Ok(Scan::Version);
            }
            let value = (spec.value)
                .map(|name| {
                    let missing =
                        || format!("error: option '{} {name}' argument missing", spec.long);
                    args.next().map(String::as_str).ok_or_else(missing)
                })
                .transpose()?;
            set(flags, spec, value)?;
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
    Ok(Scan::Args { operands, unknown })
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

/// Sets what `spec` sets with `value`.
fn set(flags: &mut Flags, spec: &Spec, value: Option<&str>) -> Result<(), String> {
    let invalid = |why: String| {
        format!(
            "error: option '{} {}' argument '{}' is invalid. {why}",
            spec.long,
            spec.value.unwrap_or_default(),
            value.unwrap_or_default()
        )
    };
    match spec.opt {
        Opt::Watch => flags.watch = true,
        Opt::NoClearConsole => flags.clear_console = false,
        Opt::Seed => flags.seed = Some(positive(value, 0).map_err(invalid)?),
        Opt::Fuzz => {
            let fuzz = positive(value, 1).map_err(invalid)?;
            flags.fuzz = u32::try_from(fuzz).unwrap_or(u32::MAX);
        }
        Opt::Workers => {
            let workers = positive(value, 1).map_err(invalid)?;
            flags.workers = Some(usize::try_from(workers).unwrap_or(usize::MAX));
        }
        Opt::Report => {
            let choices = [
                ("json", ReportName::Json),
                ("junit", ReportName::JUnit),
                ("console", ReportName::Console),
            ];
            flags.report = choice(value, &choices).map_err(invalid)?;
        }
        Opt::Dependencies => {
            let choices = [
                ("newest", Order::NewestFirst),
                ("oldest", Order::OldestFirst),
            ];
            flags.solve.order = choice(value, &choices).map_err(invalid)?;
        }
        Opt::Offline => flags.solve.offline = true,
        Opt::Output if value != Some("/dev/null") => return Err(invalid(OUTPUT_ERROR.to_owned())),
        // `--color` and `--no-color` are read from the whole line, as
        // `supports-color` reads them; nomi is its own compiler; and
        // `--version` ends the parse before it is set.
        Opt::Output | Opt::NoColor | Opt::Color | Opt::Compiler | Opt::Version => {}
    }
    Ok(())
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

/// The choice `value` names, by the names `choices` gives them.
fn choice<T: Copy>(value: Option<&str>, choices: &[(&str, T)]) -> Result<T, String> {
    let value = value.unwrap_or_default();
    if let Some(&(_, chosen)) = choices.iter().find(|(name, _)| *name == value) {
        return Ok(chosen);
    }
    let names: Vec<&str> = choices.iter().map(|(name, _)| *name).collect();
    Err(format!("Allowed choices are {}.", names.join(", ")))
}

fn too_many(command: Command, expected: usize, got: usize) -> String {
    let s = if expected == 1 { "" } else { "s" };
    format!(
        "error: too many arguments for '{}'. Expected {expected} argument{s} but got {got}.",
        command.name()
    )
}

/// `unknownOption`, with `suggestSimilar`'s guess for a long option.
fn unknown_option(option: &str) -> String {
    let mut message = format!("error: unknown option '{option}'");
    if let Some(word) = option.strip_prefix("--") {
        // `--output` is hidden.
        let candidates = (OPTIONS.iter())
            .filter(|s| s.opt != Opt::Output)
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
        let length = word.len().max(chars.len());
        // commander gives up on words further apart in length.
        let distance = if word.len().abs_diff(chars.len()) > MAX_DISTANCE {
            length
        } else {
            nomi_base::suggest::distance(&word, &chars)
        };
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
    similar.sort_unstable();
    let similar: Vec<String> = similar.iter().map(|c| format!("--{c}")).collect();
    match similar.as_slice() {
        [] => String::new(),
        [one] => format!("\n(Did you mean {one}?)"),
        many => format!("\n(Did you mean one of {}?)", many.join(", ")),
    }
}

/// elm-test run in `cx` with `args`, `var` reading the environment's
/// variables; its exit code. Tests and their runner resolve their
/// dependencies as `resolve` says: as nomi does, for `nomi test`.
pub fn main<H: Http + Sync>(
    cx: &Context<'_, H>,
    console: &mut dyn Console,
    args: &[String],
    var: &dyn Fn(&str) -> Option<String>,
    resolve: Resolve,
) -> i32 {
    let cores = supervise::default_workers();
    let color = supports_color(args, var, console.out_is_terminal());
    let mut parsed = parse(args, cores);
    if let Parsed::Run(flags, _) | Parsed::Make(flags, _) = &mut parsed {
        flags.solve.resolve = resolve;
    }
    match parsed {
        Parsed::Out(text) => {
            console.out(&text);
            0
        }
        Parsed::Err(text) => {
            console.err(&format!("{text}\n"));
            1
        }
        Parsed::Run(flags, globs) => Run {
            flags,
            globs,
            color,
            cores,
        }
        .tests(cx, console),
        Parsed::Make(flags, globs) => make(cx, console, &flags, &globs, color),
        Parsed::Setup(setup) => install(cx, console, &setup),
    }
}

/// `nomi test` with `args` for the projects `specs`, the values of `-p`,
/// name: [`main`] in the project around the directory, or in each member
/// named, or, outside every member of the workspace around it, in each
/// one with a `tests` directory, each after a line naming it where there
/// are several. Its exit code: the first that isn't 0. Help, the version
/// and a bad option are said once; a report for machines, `--watch` and
/// elm-test's other commands need one member.
pub fn members<H: Http + Sync>(
    cx: &Context<'_, H>,
    console: &mut dyn Console,
    args: &[String],
    var: &dyn Fn(&str) -> Option<String>,
    specs: &[String],
) -> i32 {
    let parsed = parse(args, supervise::default_workers());
    if let Parsed::Out(_) | Parsed::Err(_) = parsed {
        return main(cx, console, args, var, Resolve::Nomi);
    }
    let (workspace, dirs) = match scope::find(cx, specs) {
        Ok(Scope::Project) => return main(cx, console, args, var, Resolve::Nomi),
        Ok(Scope::Named(workspace, dirs)) => (workspace, dirs),
        Ok(Scope::Every(workspace)) => {
            let root = workspace.root();
            let dirs = (workspace.members().iter())
                .map(|member| member.dir.clone())
                .filter(|dir| root.join(dir).join("tests").is_dir())
                .collect();
            (workspace, dirs)
        }
        Err(report) => {
            nomi_engine::console::report(console, &report, Compat::LATEST);
            return 1;
        }
    };
    if dirs.is_empty() {
        console.err("No member of the workspace has a tests directory.\n");
        return 1;
    }
    let several = dirs.len() > 1;
    if several {
        let one = match parsed {
            Parsed::Run(flags, _) => flags.watch || flags.report.machine_readable(),
            Parsed::Make(..) | Parsed::Setup(_) => true,
            Parsed::Out(_) | Parsed::Err(_) => false,
        };
        if one {
            console.err(
                "--watch, a --report for machines, and elm-test's commands work on one \
                 member: name it with -p.\n",
            );
            return 1;
        }
    }
    let mut code = 0;
    for dir in dirs {
        if several && dir.as_os_str().is_empty() {
            console.out("Testing .\n");
        } else if several {
            console.out(&format!("Testing {}\n", dir.display()));
        }
        let dir = workspace.root().join(dir);
        let ran = main(
            &Context { dir: &dir, ..*cx },
            console,
            args,
            var,
            Resolve::Nomi,
        );
        if code == 0 {
            code = ran;
        }
    }
    code
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

/// The project `command` runs in, or its error shown.
fn read_project<H>(
    cx: &Context<'_, H>,
    console: &mut dyn Console,
    command: &str,
) -> Option<Project> {
    match project_root(cx.dir, command).and_then(|root| Project::read(&root)) {
        Ok(project) => Some(project),
        Err(message) => {
            console.err(&format!("{message}\n"));
            None
        }
    }
}

/// `ElmJson.requireElmTestPackage`.
fn require_test_package(project: &Project) -> Result<(), String> {
    let test: Name = TEST_PACKAGE.parse().expect("a package name");
    // The version, and whether it is 2.x.
    let version = match &project.outline {
        Outline::App(app) => (app.test_direct.get(&test))
            .or_else(|| app.direct.get(&test))
            .map(|v| (v.to_string(), v.major == 2)),
        Outline::Pkg(pkg) => (pkg.test_deps.get(&test))
            .or_else(|| pkg.deps.get(&test))
            .map(|c| (c.to_string(), c.lower.major == 2)),
    };
    let path = project.root.join("elm.json");
    match version {
        None => Err(format!(
            "{}\nYou must have \"{TEST_PACKAGE}\" in your \"test-dependencies\" or \"dependencies\" to run elm-test.",
            path.display()
        )),
        Some((version, false)) => Err(format!(
            "{}\nThis version of elm-test only supports {TEST_PACKAGE} 2.x, but you have \"{version}\".",
            path.display()
        )),
        Some((_, true)) => Ok(()),
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
    fn new(flags: &Flags) -> Progress {
        Progress {
            items: Vec::new(),
            shown: !flags.report.machine_readable(),
            clears: flags.clear_console,
        }
    }

    pub(crate) fn log(&mut self, console: &mut dyn Console, item: &str) {
        self.items.push(item.to_owned());
        if self.shown {
            console.out(&format!("{}\r", self.items.join(" > ")));
        }
    }

    fn new_line(&mut self, console: &mut dyn Console) {
        self.items.clear();
        if self.shown {
            console.out("\n");
        }
    }

    /// Ends the run with `message` after a line break, as elm-test's
    /// `catch` does; 1, its exit code.
    fn fail(&mut self, console: &mut dyn Console, message: &str) -> i32 {
        self.new_line(console);
        console.err(&format!("{message}\n"));
        1
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

/// A run of the tests, as its arguments and the machine ask for it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Run {
    pub flags: Flags,
    pub globs: Vec<String>,
    /// Whether elm-test colors its output (`supports-color`).
    pub color: bool,
    /// The machine's logical cores: the workers, unless `--workers` says.
    pub cores: usize,
}

impl Run {
    /// What the runner of `files` is generated with, `seed` its seed.
    pub fn options(&self, files: &[PathBuf], seed: u64) -> Options {
        Options {
            runs: self.flags.fuzz,
            report: match self.flags.report {
                ReportName::Console => Report::Console { color: self.color },
                ReportName::Json => Report::Json,
                ReportName::JUnit => Report::JUnit,
            },
            seed,
            processes: self.flags.workers.unwrap_or(self.cores),
            globs: self.globs.clone(),
            paths: files.iter().map(|f| f.display().to_string()).collect(),
        }
    }

    /// `RunTests.runTests`: one run, or with `--watch` a run each time the
    /// tests change.
    fn tests<H: Http + Sync>(&self, cx: &Context<'_, H>, console: &mut dyn Console) -> i32 {
        let root = match project_root(cx.dir, "elm-test") {
            Ok(root) => root,
            Err(message) => {
                console.err(&format!("{message}\n"));
                return 1;
            }
        };
        let mut progress = Progress::new(&self.flags);
        if self.flags.watch {
            let mut once = |console: &mut dyn Console, progress: &mut Progress| {
                self.once(cx, console, progress, &root).1
            };
            watch::watch(console, &mut progress, &root, self.color, &mut once)
        } else {
            self.once(cx, console, &mut progress, &root).0
        }
    }

    /// `RunTests.run`: its exit code, and the test source directories it
    /// found.
    fn once<H: Http + Sync>(
        &self,
        cx: &Context<'_, H>,
        console: &mut dyn Console,
        progress: &mut Progress,
        root: &Path,
    ) -> (i32, Vec<PathBuf>) {
        let project = Project::read(root).and_then(|project| {
            require_test_package(&project)?;
            project.validate()?;
            Ok(project)
        });
        match project {
            Ok(project) => (
                self.project(cx, console, progress, &project),
                project.source_dirs,
            ),
            Err(message) => (progress.fail(console, &message), Vec::new()),
        }
    }

    /// The rest of `RunTests.run`, once the project is read.
    fn project<H: Http + Sync>(
        &self,
        cx: &Context<'_, H>,
        console: &mut dyn Console,
        progress: &mut Progress,
        project: &Project,
    ) -> i32 {
        let files = match resolve(project, cx.dir, &self.globs) {
            Ok(files) => files,
            Err(message) => return progress.fail(console, &message),
        };
        let seed = self.flags.seed.unwrap_or_else(supervise::random_seed);
        let options = self.options(&files, seed);
        let env = Env {
            cx,
            solve: self.flags.solve,
            color: self.color,
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
            Err(Failure::Shown) => {
                return progress.fail(console, "`elm make` failed with exit code 1.");
            }
            Err(failure) => {
                progress.new_line(console);
                return failed(console, &failure);
            }
        };
        progress.log(console, "Starting tests");
        progress.new_line(console);
        let supervise = Supervise {
            report: options.report,
            processes: options.processes,
            color: self.color,
        };
        match supervise.run(console, &runner.js) {
            Ok(Ended::Code(code)) => code,
            Ok(Ended::Thrown) => {
                progress.new_line(console);
                1
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                progress.new_line(console);
                nomi_engine::console::report(
                    console,
                    &nomi_engine::run::no_bun(&e),
                    Compat::LATEST,
                );
                1
            }
            Err(e) => progress.fail(console, &e.to_string()),
        }
    }
}

/// Shows what `failure` says that isn't shown already; 1, the exit code.
fn failed(console: &mut dyn Console, failure: &Failure) -> i32 {
    match failure {
        Failure::Files(message) => console.err(&format!("{message}\n")),
        Failure::Project(report) => nomi_engine::console::report(console, report, Compat::LATEST),
        Failure::Shown => {}
    }
    1
}

/// `elm-test init`, or `install` of a package.
fn install<H: Http + Sync>(cx: &Context<'_, H>, console: &mut dyn Console, setup: &Setup) -> i32 {
    let command = match setup {
        Setup::Init => "elm-test init",
        Setup::Install(_) => "elm-test install",
    };
    let Some(project) = read_project(cx, console, command) else {
        return 1;
    };
    let env = Env {
        cx,
        solve: Solve::default(),
        color: false,
    };
    let done = match setup {
        Setup::Init => install::init(&env, console, &project).map(|()| Installed::Added),
        Setup::Install(package) => install::install(&env, console, &project, package),
    };
    match done {
        Ok(Installed::Already) => {
            console.out("It is already installed!\n");
            0
        }
        Ok(Installed::Added) => 0,
        Err(InstallError::Message(message)) => {
            console.err(&format!("{message}\n"));
            1
        }
        Err(InstallError::Shown) => 1,
    }
}

/// `elm-test make`.
fn make<H: Http + Sync>(
    cx: &Context<'_, H>,
    console: &mut dyn Console,
    flags: &Flags,
    globs: &[String],
    color: bool,
) -> i32 {
    let Some(project) = read_project(cx, console, "elm-test make") else {
        return 1;
    };
    // `resolveGlobs`, which finding nothing doesn't stop here.
    let files = resolve(&project, cx.dir, globs).unwrap_or_default();
    let env = Env {
        cx,
        solve: flags.solve,
        color,
    };
    let style = match flags.report {
        ReportName::Json => Style::Json,
        ReportName::Console | ReportName::JUnit => Style::Terminal,
    };
    let made = if flags.report.machine_readable() {
        build::make(&env, &mut Silent(console), &project, &files, style)
    } else {
        build::make(&env, console, &project, &files, style)
    };
    match made {
        Ok(()) => 0,
        Err(failure) => failed(console, &failure),
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
        assert_eq!(parse_line("make --help"), Parsed::Out(Command::Make.help()));
        assert_eq!(parse_line("help init"), Parsed::Out(Command::Init.help()));
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
        assert!(flags.solve.offline);
        assert_eq!(globs, ["src/A.elm"]);
    }
}
