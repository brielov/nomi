//! The `nomi` binary, which is also elm-format when invoked under that
//! name. Command logic lives in other crates; this crate only maps
//! arguments onto them.

use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use nomi_engine::check::{self, Style};
use nomi_engine::console::{Personality, StdConsole};
use nomi_engine::format::{self, Mode, Outcome};
use nomi_engine::paths::Cache;
use nomi_engine::{Vfs, Web, bump, diff, init, install, make, publish, remove, tree, update};

/// One toolchain for Elm: compiler, formatter, language server, test runner
/// and package manager.
#[derive(Debug, Parser)]
#[command(name = "nomi", version, arg_required_else_help = true)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

/// Subcommands are added by the plan steps that implement them. Each one
/// declares the Elm names it answers to as aliases, e.g.
/// `#[command(visible_alias = "make")] Build`.
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
    },
    /// Remove a package from the project's dependencies, with what only
    /// it needed.
    #[command(visible_alias = "remove")]
    Rm {
        /// The package, e.g. elm/http.
        package: String,
    },
    /// Move the project's dependencies to their newest releases of the
    /// same major version.
    Update {
        /// Only this package, e.g. elm/http, and those that need it.
        package: Option<String>,
    },
    /// List the dependencies that have newer releases.
    Outdated,
    /// Show the releases the project builds with, under the dependencies
    /// that need them.
    Tree,
    /// Start the language server, on standard input and output.
    Lsp,
    /// Build modules into JavaScript or HTML as `elm make` does.
    #[command(visible_alias = "make")]
    Build {
        /// The modules with the `main`s to build; a package's exposed
        /// modules, checked, if none.
        paths: Vec<PathBuf>,
        /// Where to write the code: a `.js` or `.html` file, or
        /// `/dev/null` for none. `index.html` for one `main` and `elm.js`
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
    },
    /// Show how a package's API changed: the code against its latest
    /// release or a given one, two of its releases, or two of any
    /// package's.
    Diff {
        /// Nothing, a version, two versions, or a package and two
        /// versions.
        #[arg(num_args = 0..=3)]
        args: Vec<String>,
    },
    /// Set the package's version to the one its API changes call for.
    Bump,
    /// Publish the package to package.elm-lang.org, once it passes every
    /// check `elm publish` makes.
    Publish,
    /// Check modules as `elm make --output=/dev/null` does, generating no
    /// code.
    Check {
        /// The modules to start from; a package's exposed modules if
        /// none.
        paths: Vec<PathBuf>,
        /// Report problems as JSON on standard error, as `elm make
        /// --report=json` does.
        #[arg(long, value_name = "FORMAT")]
        report: Option<ReportFormat>,
    },
}

/// The machine-readable report formats.
#[derive(Clone, Copy, Debug, clap::ValueEnum)]
enum ReportFormat {
    Json,
}

mod elm;
mod elm_format;
mod terminal;

fn main() -> ExitCode {
    let mut args = std::env::args_os();
    let name = args.next().unwrap_or_default();
    match Path::new(&name).file_stem().and_then(|stem| stem.to_str()) {
        Some("elm-format") => return elm_format::main(args),
        Some("elm") => return elm::main(args),
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
                format_stdin(mode)
            } else {
                format_paths(&paths, mode)
            }
        }
        Some(Command::New { dir }) => new(Personality::Nomi, dir),
        Some(Command::Add { package }) => add(Personality::Nomi, Some(&package)),
        Some(Command::Rm { package }) => rm(&package),
        Some(Command::Update { package }) => in_project(|console, dir, cache, http| {
            update::run(&Vfs::new(), console, dir, cache, http, package.as_deref())
        }),
        Some(Command::Outdated) => in_project(|console, dir, cache, http| {
            update::outdated(&Vfs::new(), console, dir, cache, http)
        }),
        Some(Command::Tree) => in_project(|console, dir, cache, http| {
            tree::run(&Vfs::new(), console, dir, cache, http)
        }),
        Some(Command::Lsp) => match nomi_lsp::stdio() {
            Ok(()) => true,
            Err(e) => {
                eprintln!("error: the language server stopped: {e}");
                false
            }
        },
        Some(Command::Build {
            paths,
            output,
            debug,
            optimize,
            docs,
            report,
        }) => {
            let output = match output.as_deref().map(make::Output::parse) {
                None => None,
                Some(Some(output)) => Some(output),
                Some(None) => {
                    eprintln!("error: --output takes a file ending in .js or .html, or /dev/null");
                    return exit(false);
                }
            };
            let make = make::Make {
                paths,
                personality: Personality::Nomi,
                style: match report {
                    Some(ReportFormat::Json) => Style::Json,
                    None => Style::Terminal,
                },
                debug,
                optimize,
                output,
                docs,
            };
            in_project(|console, dir, cache, http| {
                make::run(&Vfs::new(), console, dir, cache, http, &make)
            })
        }
        Some(Command::Diff { args }) => {
            let args: Vec<&str> = args.iter().map(String::as_str).collect();
            let Some(args) = diff::parse_args(&args) else {
                eprintln!(
                    "error: diff takes nothing, a version, two versions, or a package and two versions"
                );
                return exit(false);
            };
            in_project(|console, dir, cache, http| {
                diff::run(&Vfs::new(), console, dir, cache, http, &args)
            })
        }
        Some(Command::Publish) => in_project(|console, dir, cache, http| {
            publish::run(&Vfs::new(), console, dir, cache, http)
        }),
        Some(Command::Bump) => in_project(|console, dir, cache, http| {
            bump::run(&Vfs::new(), console, dir, cache, http)
        }),
        Some(Command::Check { paths, report }) => in_project(|console, dir, cache, http| {
            let style = match report {
                Some(ReportFormat::Json) => Style::Json,
                None => Style::Terminal,
            };
            check::run(&Vfs::new(), console, dir, cache, http, &paths, style)
        }),
    };
    exit(succeeded)
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
    init::run(
        &Vfs::new(),
        &mut StdConsole,
        personality,
        &dir,
        &cache,
        &Web::new(),
    )
}

/// `nomi add` and `elm install`, for the project around the current
/// directory.
fn add(personality: Personality, package: Option<&str>) -> bool {
    let Some(cache) = cache() else { return false };
    let Some(dir) = current_dir() else {
        return false;
    };
    install::run(
        &Vfs::new(),
        &mut StdConsole,
        personality,
        &dir,
        &cache,
        &Web::new(),
        package,
    )
}

/// `nomi rm`, for the project around the current directory.
fn rm(package: &str) -> bool {
    in_project(|console, dir, cache, http| {
        remove::run(&Vfs::new(), console, dir, cache, http, package)
    })
}

/// `command` for the project around the current directory.
pub(crate) fn in_project(
    command: impl FnOnce(&mut StdConsole, &Path, &Cache, &Web) -> bool,
) -> bool {
    let Some(cache) = cache() else { return false };
    let Some(dir) = current_dir() else {
        return false;
    };
    command(&mut StdConsole, &dir, &cache, &Web::new())
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

fn format_paths(paths: &[PathBuf], mode: Mode) -> bool {
    let report = format::format_paths(&Vfs::new(), paths, mode);
    for (root, error) in &report.roots {
        eprintln!("error: {}: {error}", root.display());
    }
    for (file, outcome) in &report.files {
        match outcome {
            Outcome::Unchanged => {}
            Outcome::Formatted if mode == Mode::Check => println!("{}", file.display()),
            Outcome::Formatted => {}
            Outcome::Invalid(at) => {
                eprintln!(
                    "error: {}:{}:{}: syntax error",
                    file.display(),
                    at.row,
                    at.column
                )
            }
            Outcome::Failed(e) => eprintln!("error: {}: {e}", file.display()),
        }
    }
    report.succeeded(mode)
}

fn format_stdin(mode: Mode) -> bool {
    let mut text = String::new();
    if let Err(e) = io::stdin().read_to_string(&mut text) {
        eprintln!("error: <stdin>: {e}");
        return false;
    }
    match format::format_text(&text) {
        Ok(formatted) if mode == Mode::Check => {
            let unchanged = formatted == text;
            if !unchanged {
                println!("<stdin>");
            }
            unchanged
        }
        Ok(formatted) => io::stdout().write_all(formatted.as_bytes()).is_ok(),
        Err(at) => {
            eprintln!("error: <stdin>:{}:{}: syntax error", at.row, at.column);
            false
        }
    }
}
