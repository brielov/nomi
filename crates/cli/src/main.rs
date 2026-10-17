//! The `nomi` binary, which is also elm-format, elm and elm-test when
//! invoked under those names. Command logic lives in other crates; this crate only maps
//! arguments onto them.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use nomi_base::Platform;
use nomi_engine::check::Style;
use nomi_engine::console::{Console, Personality, StdConsole};
use nomi_engine::format::{self, Mode};
use nomi_engine::make::{FfiModule, Make, NamedIn};
use nomi_engine::packages::Resolve;
use nomi_engine::paths::Cache;
use nomi_engine::scope::Verb;
use nomi_engine::{
    Context, Vfs, Web, bump, diff, init, install, make, publish, remove, run, scope, tree, update,
};

/// One toolchain for Elm: compiler, formatter, language server, test runner
/// and package manager.
#[derive(Debug, Parser)]
#[command(name = "nomi", version, arg_required_else_help = true)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

/// nomi's commands, each with the name elm gives it, where elm has it,
/// as a visible alias.
#[derive(Debug, Subcommand)]
enum Command {
    /// Format Elm modules as elm-format 0.8.8 does.
    #[command(visible_alias = "fmt")]
    Format {
        /// Files, or directories to search for `.elm` files.
        #[arg(default_value = ".", conflicts_with = "stdin")]
        paths: Vec<PathBuf>,
        /// Change no files; list those that aren't formatted, and fail if
        /// there are any.
        #[arg(long)]
        check: bool,
        /// Format standard input to standard output.
        #[arg(long)]
        stdin: bool,
    },
    /// Start an application in a directory, the current one by default.
    #[command(visible_alias = "init")]
    New { dir: Option<PathBuf> },
    /// Add a package to the project's dependencies.
    #[command(visible_alias = "install")]
    Add {
        /// The package, e.g. elm/http.
        package: String,
        /// The member of the workspace to add it to, by its directory or
        /// package name.
        #[arg(short = 'p', long = "package", value_name = "MEMBER")]
        member: Option<String>,
    },
    /// Remove a package from the project's dependencies, with what only
    /// it needed.
    #[command(visible_alias = "remove")]
    Rm {
        /// The package, e.g. elm/http.
        package: String,
        /// The member of the workspace to work on, by its directory or
        /// package name.
        #[arg(short = 'p', long = "package", value_name = "MEMBER")]
        member: Option<String>,
    },
    /// Move the project's dependencies to their newest releases of the
    /// same major version.
    Update {
        /// Only this package, e.g. elm/http, and those that need it.
        package: Option<String>,
        /// Members of the workspace to update together, by directory or
        /// package name; at its root, every member.
        #[arg(short = 'p', long = "package", value_name = "MEMBER")]
        members: Vec<String>,
    },
    /// List the dependencies that have newer releases.
    Outdated {
        /// The member of the workspace to work on, by its directory or
        /// package name.
        #[arg(short = 'p', long = "package", value_name = "MEMBER")]
        member: Option<String>,
    },
    /// Show the releases the project builds with, under the dependencies
    /// that need them.
    Tree {
        /// The member of the workspace to work on, by its directory or
        /// package name.
        #[arg(short = 'p', long = "package", value_name = "MEMBER")]
        member: Option<String>,
    },
    /// Start the language server, on standard input and output.
    Lsp {
        /// Taken and ignored, as many editors pass it: standard input and
        /// output are the only transport.
        #[arg(long, hide = true)]
        stdio: bool,
    },
    /// Build modules into JavaScript or HTML as `elm make` does.
    #[command(visible_alias = "make")]
    Build(Build),
    /// Build an entry of nomi.toml and run it under Node, as the process,
    /// or serve it if it runs in the browser; without one named, every
    /// entry, together.
    Run {
        /// The entry to run, by its `main`.
        #[arg(conflicts_with = "elm")]
        entry: Option<PathBuf>,
        /// Run in this member of the workspace, by its directory or
        /// package name: its entries, or the one named, from its
        /// directory.
        #[arg(short = 'p', long = "package", value_name = "MEMBER")]
        member: Option<String>,
        /// Run this module's `main` instead, without nomi.toml.
        #[arg(short, long, value_name = "FILE")]
        elm: Option<PathBuf>,
        /// Build and run it again each time the project changes.
        #[arg(long)]
        watch: bool,
        /// The program's arguments, after `--`.
        #[arg(last = true)]
        args: Vec<OsString>,
    },
    /// Show how a package's API changed: the code against its latest
    /// release or a given one, two of its releases, or two of any
    /// package's.
    Diff {
        /// Nothing, a version, two versions, or a package and two
        /// versions.
        #[arg(num_args = 0..=3)]
        args: Vec<String>,
        /// The member of the workspace to work on, by its directory or
        /// package name.
        #[arg(short = 'p', long = "package", value_name = "MEMBER")]
        member: Option<String>,
    },
    /// Set the package's version to the one its API changes call for.
    Bump {
        /// The member of the workspace to work on, by its directory or
        /// package name.
        #[arg(short = 'p', long = "package", value_name = "MEMBER")]
        member: Option<String>,
    },
    /// Publish the package to package.elm-lang.org, once it passes every
    /// check `elm publish` makes.
    Publish {
        /// The member of the workspace to work on, by its directory or
        /// package name.
        #[arg(short = 'p', long = "package", value_name = "MEMBER")]
        member: Option<String>,
    },
    /// Run the tests, as elm-test does, with its options.
    #[command(disable_help_flag = true)]
    Test {
        /// Members of the workspace to test, by directory or package
        /// name; at its root, every member with tests. Taken among
        /// elm-test's options too.
        #[arg(short = 'p', long = "package", value_name = "MEMBER")]
        members: Vec<String>,
        /// elm-test's options and globs; `nomi test --help` lists them.
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<OsString>,
    },
    /// Check modules as `elm make --output=/dev/null` does, generating no
    /// code.
    Check {
        /// The modules to start from; without them, each entry of
        /// nomi.toml's, or a package's exposed modules.
        paths: Vec<PathBuf>,
        /// Members of the workspace to check, by directory or package
        /// name; at its root, every member.
        #[arg(
            short = 'p',
            long = "package",
            value_name = "MEMBER",
            conflicts_with = "paths"
        )]
        members: Vec<String>,
        /// Report problems as JSON on standard error, as `elm make
        /// --report=json` does.
        #[arg(long, value_name = "FORMAT")]
        report: Option<ReportFormat>,
    },
}

/// `nomi build`'s arguments.
#[derive(Debug, clap::Args)]
struct Build {
    /// The modules with the `main`s to build; without them, each entry
    /// of nomi.toml, or a package's exposed modules, checked.
    paths: Vec<PathBuf>,
    /// Members of the workspace to build, by directory or package name;
    /// at its root, every member.
    #[arg(
        short = 'p',
        long = "package",
        value_name = "MEMBER",
        conflicts_with = "paths"
    )]
    members: Vec<String>,
    /// Where to write the code: a `.js` or `.html` file, an `.mjs` ES
    /// module, or `/dev/null` for none. `index.html` for one `main` and `elm.js`
    /// for more by default.
    #[arg(long, value_name = "FILE")]
    output: Option<String>,
    /// Build with the time-travelling debugger.
    #[arg(long, conflicts_with = "optimize")]
    debug: bool,
    /// Shorten record fields and unbox types, refusing `Debug`.
    #[arg(long, visible_alias = "release")]
    optimize: bool,
    /// Write a package's documentation to this file.
    #[arg(long, value_name = "FILE")]
    docs: Option<PathBuf>,
    /// Report problems as JSON on standard error, as `elm make
    /// --report=json` does.
    #[arg(long, value_name = "FORMAT")]
    report: Option<ReportFormat>,
    /// Where the code runs: by default Node for a nomi/core
    /// `Program`, else the browser. Under Node, an `.mjs` output of a
    /// `Program` starts it when run; any other exports `Elm`, as in
    /// the browser.
    #[arg(long, value_enum)]
    platform: Option<PlatformName>,
    /// The ES module whose exported async functions `Nomi.Ffi.task`
    /// calls, imported by an `.mjs` output built for Node.
    #[arg(long, value_name = "FILE")]
    ffi: Option<PathBuf>,
}

/// The machine-readable report formats.
#[derive(Clone, Copy, Debug, clap::ValueEnum)]
enum ReportFormat {
    Json,
}

/// The platforms code is built for.
#[derive(Clone, Copy, Debug, clap::ValueEnum)]
enum PlatformName {
    Browser,
    Node,
}

mod elm;
mod elm_format;
mod terminal;

/// mimalloc, which builds cold about a fifth faster than the system's
/// allocator and jemalloc a tenth. Without transparent huge
/// pages, which would make cold builds faster still but cost warm ones
/// CPU time and memory.
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

fn main() -> ExitCode {
    let mut args = std::env::args_os();
    let name = args.next().unwrap_or_default();
    match Path::new(&name).file_stem().and_then(|stem| stem.to_str()) {
        Some("elm-format") => return elm_format::main(args),
        Some("elm") => return elm::main(args),
        Some("elm-test") => return elm_test(args, Personality::Elm, &[]),
        _ => {}
    }
    let cli = Cli::parse();
    let succeeded = match cli.command {
        None => true,
        Some(Command::Format {
            paths,
            check,
            stdin,
        }) => {
            let mode = if check { Mode::Check } else { Mode::Write };
            if stdin {
                format::run_stdin(&mut StdConsole, mode)
            } else {
                format::run(&Vfs::new(), &mut StdConsole, &paths, mode)
            }
        }
        Some(Command::New { dir }) => new(Personality::Nomi, dir),
        Some(Command::Add { package, member }) => {
            in_member(member.as_deref(), Verb::Add, |cx, console| {
                install::run(cx, console, Personality::Nomi, Some(&package))
            })
        }
        Some(Command::Rm { package, member }) => {
            in_member(member.as_deref(), Verb::Rm, |cx, console| {
                remove::run(cx, console, &package)
            })
        }
        Some(Command::Update { package, members }) => {
            in_project(|cx, console| update::run(cx, console, package.as_deref(), &members))
        }
        Some(Command::Outdated { member }) => {
            in_member(member.as_deref(), Verb::Outdated, update::outdated)
        }
        Some(Command::Tree { member }) => in_member(member.as_deref(), Verb::Tree, tree::run),
        Some(Command::Lsp { .. }) => match nomi_lsp::stdio() {
            Ok(()) => true,
            Err(e) => {
                eprintln!("error: the language server stopped: {e}");
                false
            }
        },
        Some(Command::Build(args)) => build(args),
        Some(Command::Run {
            entry,
            member,
            elm,
            watch,
            args,
        }) => {
            let program = match elm {
                Some(module) => run::Program::Module(module),
                None => run::Program::Entry(entry),
            };
            return nomi_run(&program, member.as_deref(), watch, &args);
        }
        Some(Command::Diff { args, member }) => {
            let args = args.iter().map(String::as_str).collect::<Vec<_>>();
            if let Some(args) = diff::parse_args(&args) {
                in_member(member.as_deref(), Verb::Diff, |cx, console| {
                    diff::run(cx, console, &args)
                })
            } else {
                eprintln!(
                    "error: diff takes nothing, a version, two versions, or a package and two \
                     versions"
                );
                false
            }
        }
        Some(Command::Publish { member }) => {
            in_member(member.as_deref(), Verb::Publish, publish::run)
        }
        Some(Command::Bump { member }) => in_member(member.as_deref(), Verb::Bump, bump::run),
        Some(Command::Test { mut members, args }) => {
            let args = test_args(args, &mut members);
            return elm_test(args.into_iter(), Personality::Nomi, &members);
        }
        Some(Command::Check {
            paths,
            members,
            report,
        }) => in_project(|cx, console| {
            scope::make(cx, console, &members, &Make::check(paths, style(report)))
        }),
    };
    exit(succeeded)
}

fn style(report: Option<ReportFormat>) -> Style {
    match report {
        Some(ReportFormat::Json) => Style::Json,
        None => Style::Terminal,
    }
}

/// `nomi build` with `args`, for the project around the current
/// directory.
fn build(args: Build) -> bool {
    let Build {
        paths,
        members,
        output,
        debug,
        optimize,
        docs,
        report,
        platform,
        ffi,
    } = args;
    let platform = platform.map(|platform| match platform {
        PlatformName::Browser => Platform::Browser,
        PlatformName::Node => Platform::Node,
    });
    let Some(make) = Make::build(paths, output.as_deref(), platform) else {
        eprintln!("error: --output takes a file ending in .js, .mjs or .html, or /dev/null");
        return false;
    };
    let make = Make {
        style: style(report),
        kind: make::kind(debug, optimize),
        ffi: ffi.map(|path| FfiModule {
            path,
            named_in: NamedIn::Flag,
        }),
        docs,
        ..make
    };
    in_project(|cx, console| scope::make(cx, console, &members, &make))
}

/// `nomi test`'s arguments after its first of elm-test's, with each
/// `-p` and `--package` among them, and its value, moved to `members`:
/// clap stops reading options at the first it doesn't know.
fn test_args(args: Vec<OsString>, members: &mut Vec<String>) -> Vec<OsString> {
    let mut kept = Vec::with_capacity(args.len());
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        let text = arg.to_string_lossy();
        if text == "--" {
            kept.push(arg);
            kept.extend(args);
            break;
        }
        if text == "-p" || text == "--package" {
            match args.next() {
                Some(member) => members.push(member.to_string_lossy().into_owned()),
                None => kept.push(arg),
            }
        } else if let Some(member) = (text.strip_prefix("--package="))
            .or_else(|| text.strip_prefix("-p").filter(|rest| !rest.is_empty()))
        {
            members.push(member.trim_start_matches('=').to_owned());
        } else {
            kept.push(arg);
        }
    }
    kept
}

/// `nomi run`, in the current directory or the member it names.
fn nomi_run(
    program: &run::Program,
    member: Option<&str>,
    watch: bool,
    args: &[OsString],
) -> ExitCode {
    let (Some(cache), Some(dir)) = (cache(), current_dir()) else {
        return ExitCode::FAILURE;
    };
    ExitCode::from(run_in(&dir, &cache, |cx, console| {
        if watch {
            run::watch(cx, console, program, member, args)
        } else {
            run::run(cx, console, program, member, args)
        }
    }))
}

/// elm-test's command line, as `elm-test`, or as `nomi test` of the
/// members `members` names, resolving dependencies as nomi does.
fn elm_test(
    args: impl Iterator<Item = OsString>,
    personality: Personality,
    members: &[String],
) -> ExitCode {
    let args: Vec<String> = args.map(|a| a.to_string_lossy().into_owned()).collect();
    let (Some(cache), Some(cwd)) = (cache(), current_dir()) else {
        return ExitCode::FAILURE;
    };
    let var = |name: &str| std::env::var(name).ok();
    let code = run_in(&cwd, &cache, |cx, console| match personality {
        Personality::Elm => nomi_test::main(cx, console, &args, &var, Resolve::Upstream),
        Personality::Nomi => nomi_test::members(cx, console, &args, &var, members),
    });
    ExitCode::from(u8::try_from(code).unwrap_or(1))
}

fn exit(succeeded: bool) -> ExitCode {
    if succeeded {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

/// `nomi new` and `elm init`, in `dir` or the current directory.
fn new(personality: Personality, dir: Option<PathBuf>) -> bool {
    let Some(cache) = cache() else { return false };
    let dir = dir.unwrap_or_else(|| PathBuf::from("."));
    run_in(&dir, &cache, |cx, console| {
        init::run(cx, console, personality)
    })
}

/// `elm install`, for the project around the current directory.
fn elm_install(package: Option<&str>) -> bool {
    in_project(|cx, console| install::run(cx, console, Personality::Elm, package))
}

/// `elm diff`, for the project around the current directory, saying
/// `usage` where `args` are none of its four forms.
fn diff(args: &[&str], usage: &str) -> bool {
    let Some(args) = diff::parse_args(args) else {
        eprintln!("error: {usage}");
        return false;
    };
    in_project(|cx, console| diff::run(cx, console, &args))
}

/// `command` for the member of the workspace `member` names, or the
/// project around the current directory, as `verb`, which needs one.
fn in_member(
    member: Option<&str>,
    verb: Verb,
    command: impl FnOnce(&Context<'_, Web>, &mut dyn Console) -> bool,
) -> bool {
    in_project(|cx, console| scope::in_one(cx, console, member, verb, false, command))
}

/// `command` for the project around the current directory.
fn in_project(command: impl FnOnce(&Context<'_, Web>, &mut dyn Console) -> bool) -> bool {
    let Some(cache) = cache() else { return false };
    let Some(dir) = current_dir() else {
        return false;
    };
    run_in(&dir, &cache, command)
}

/// `command` run in `dir`, with the cache at `cache` and the package
/// website, on the process's standard streams.
fn run_in<T>(
    dir: &Path,
    cache: &Cache,
    command: impl FnOnce(&Context<'_, Web>, &mut dyn Console) -> T,
) -> T {
    let cx = Context {
        vfs: &Vfs::new(),
        dir,
        cache,
        http: &Web::new(),
    };
    command(&cx, &mut StdConsole)
}

fn current_dir() -> Option<PathBuf> {
    std::env::current_dir()
        .inspect_err(|e| eprintln!("error: the current directory: {e}"))
        .ok()
}

fn cache() -> Option<Cache> {
    Cache::locate()
        .inspect_err(|e| eprintln!("error: {e}"))
        .ok()
}
