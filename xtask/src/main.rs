//! Repository automation, run as `cargo xtask <command>`.

mod bench;
mod checks;
mod codegen;
mod corpus;
mod download;
mod fuzz;
mod oracle;
mod references;
mod refs;

use std::path::{Path, PathBuf};
use std::process::{Command as Process, ExitCode};

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use oracle::corpus_runs::Suite;

#[derive(Debug, Parser)]
#[command(name = "cargo xtask")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Tier 1: format check, clippy, tests and architecture checks.
    Ci,
    /// Only the architecture checks.
    Check,
    /// Regenerate the typed syntax views from elm.ungram.
    Codegen,
    /// Sync reference/ to the source commits pinned in references.toml.
    Refs,
    /// Pinned upstream tools that parity is measured against.
    Oracle {
        #[command(subcommand)]
        command: OracleCommand,
    },
    /// Run a parity suite and record its newly passing cases in passing.txt.
    Ratchet {
        /// The suite: a test target `crates/<crate>/tests/<suite>.rs`.
        suite: String,
    },
    /// Real-world Elm code that the parity suites run over.
    Corpus {
        #[command(subcommand)]
        command: CorpusCommand,
    },
    /// Run a fuzz target of fuzz/ under libFuzzer, on stable.
    Fuzz {
        /// The target: a binary of fuzz/Cargo.toml.
        target: String,
        /// How many inputs to run, across every job.
        #[arg(long, default_value_t = 1_000_000)]
        runs: u64,
        /// Concurrent fuzzer processes; the available cores by default.
        #[arg(long)]
        jobs: Option<usize>,
    },
    /// Time a crate's benchmarks, which fail when twice the input takes
    /// three times as long or more; or nomi against the pinned upstream
    /// tools, writing the scoreboard into docs/benchmarks.md.
    Bench {
        /// The crate: `syntax` for nomi-syntax, `format` for nomi-format;
        /// or `elm` or `elm-format`, to print that part of the scoreboard.
        /// The whole scoreboard if none.
        target: Option<String>,
    },
}

#[derive(Debug, Subcommand)]
enum CorpusCommand {
    /// Re-pin corpus/packages.toml from the live registry.
    Pin,
    /// Download and verify every pinned package and app.
    Fetch,
    /// Record which Compats accept each package, and pin each app's
    /// manifest rewritten for each Compat, in corpus/compat.toml.
    Classify,
    /// Count what is pinned and fetched, and what each Compat accepts.
    Stats,
    /// Tier 2: whether nomi accepts and rejects every corpus module as the
    /// oracles did, a case per project.
    Syntax {
        /// Record newly passing projects in corpus/syntax/passing.txt.
        #[arg(long)]
        record: bool,
    },
    /// Whether nomi reads every corpus `elm.json` as each Compat that
    /// builds it and writes it back as elm would.
    Outline,
    /// Whether every corpus app's dependencies check out and download as
    /// each Compat, from the live registry; not part of CI.
    Deps,
    /// Copy the elm/* packages tier 1 compiles against into the compiler's
    /// tests.
    Fixtures,
    /// Tier 2, by hand: whether every corpus app and package each elm
    /// accepts canonicalizes cleanly, downloading dependencies from the
    /// live registry into the development cache.
    Names {
        /// Only projects whose case contains this text.
        only: Option<String>,
    },
    /// Tier 2, by hand: whether nomi documents every corpus package as
    /// each elm did (`cargo xtask oracle corpus-docs`), downloading
    /// dependencies as `names` does.
    Docs {
        /// Only packages whose case contains this text.
        only: Option<String>,
    },
    /// Tier 2, by hand: whether `nomi check` reports on every corpus app
    /// and package as each elm did (`cargo xtask oracle corpus-check`),
    /// downloading dependencies as `names` does.
    Check {
        /// Only projects whose case contains this text.
        only: Option<String>,
    },
    /// Tier 2, by hand: whether nomi's graph of every corpus project's
    /// dependencies is the `o.dat` each elm wrote (`cargo xtask oracle
    /// corpus-objects`), downloading dependencies as `names` does.
    Objects {
        /// Only projects whose case contains this text.
        only: Option<String>,
    },
    /// Tier 2, by hand: whether nomi generates the JavaScript each elm
    /// generated for every corpus app in each mode (`cargo xtask oracle
    /// corpus-javascript`), downloading dependencies as `names` does.
    Javascript {
        /// Only apps whose case contains this text.
        only: Option<String>,
    },
    /// Where two `.elmo` files' graphs first differ, nomi's and elm's.
    GraphDiff {
        /// The release that wrote them, as `0.19.1`.
        release: String,
        nomi: PathBuf,
        elm: PathBuf,
    },
    /// Tier 2, by hand: whether nomi optimizes every module of every
    /// corpus app and package as each elm did (`cargo xtask oracle
    /// corpus-graphs`), downloading dependencies as `names` does.
    Graphs {
        /// Only projects whose case contains this text.
        only: Option<String>,
    },
    /// Tier 2, by hand: whether nomi answers the language-server session
    /// of every sampled corpus project as elm-language-server did (`cargo
    /// xtask oracle corpus-lsp`).
    Lsp {
        /// Record newly passing answers in corpus/lsp/passing.txt.
        #[arg(long)]
        record: bool,
        /// Only projects whose case contains this text.
        only: Option<String>,
    },
    /// By hand: whether `nomi test` runs every corpus app's test suite as
    /// elm-test did (`cargo xtask oracle corpus-tests`), downloading test
    /// dependencies into the development cache.
    Tests {
        /// Only apps whose name contains this text.
        only: Option<String>,
    },
    /// Tier 2: whether every module elm accepts lowers to the HIR with its
    /// source map whole.
    Hir,
    /// Tier 2: whether nomi formats every corpus module as elm-format did,
    /// a case per project.
    Format {
        /// Record newly passing projects in corpus/format/passing.txt.
        #[arg(long)]
        record: bool,
    },
}

#[derive(Debug, Subcommand)]
enum OracleCommand {
    /// Download, verify and install every pinned oracle.
    Fetch,
    /// Print every pin and whether it is installed.
    Versions,
    /// Store whether each elm and elm-format accept every syntax mutation
    /// and corpus module.
    Syntax {
        /// Only these inputs; both by default.
        inputs: Option<oracle::syntax::Inputs>,
        /// Concurrent oracle runs; twice the available cores by default.
        #[arg(long)]
        jobs: Option<usize>,
        /// Print each elm's report on this mutation, named `<seed> / <mutation>`,
        /// instead.
        #[arg(long, conflicts_with = "inputs")]
        show: Option<String>,
    },
    /// Store elm-format's output for every formatter fixture and corpus
    /// module.
    Format {
        /// Only these inputs; both by default.
        inputs: Option<oracle::format::Inputs>,
        /// Concurrent oracle runs; twice the available cores by default.
        #[arg(long)]
        jobs: Option<usize>,
    },
    /// Store elm-format's, elm's and elm-test's transcript of every command-line case.
    Transcripts {
        /// Only this tool's: `elm-format`, `elm` or `elm-test`.
        tool: Option<String>,
    },
    /// Store what each elm reports for every `elm.json` fixture.
    Outline,
    /// Store what each elm reports for every module-graph case.
    Modules,
    /// Store what each elm reports on the naming-error projects.
    Names,
    /// Store what each elm reports on the type-error projects.
    Types,
    /// Store what each elm reports on the pattern-error projects.
    Patterns,
    /// Store what each elm reports and documents of every
    /// documentation project.
    Docs,
    /// Store what each elm makes of every corpus package with `elm make
    /// --docs`, offline, with the releases the development cache has.
    CorpusDocs {
        /// Concurrent elm runs; the available cores by default.
        #[arg(long)]
        jobs: Option<usize>,
        /// Print the cells of the packages whose case contains this text
        /// instead.
        #[arg(long)]
        only: Option<String>,
    },
    /// Store the JavaScript each elm generates for every fixture program
    /// in each mode.
    Javascript,
    /// Store the optimized graphs each elm writes for every optimizer
    /// case.
    Optimized,
    /// Store each elm's transcript of every case of the CLI report
    /// matrix.
    Make,
    /// Store each elm's transcript of every `elm diff` and `elm bump`
    /// case, recorded online with read-only requests.
    Diff,
    /// Store each elm's transcript of every `elm publish` case, each
    /// failing before it reaches GitHub, recorded online.
    Publish,
    /// Store a digest of the dependencies' `o.dat` each elm writes for
    /// every corpus app and package, as `corpus-check` runs elm.
    CorpusObjects {
        /// Concurrent elm runs; the available cores by default.
        #[arg(long)]
        jobs: Option<usize>,
        /// Print the cells of the projects whose case contains this text
        /// instead.
        #[arg(long)]
        only: Option<String>,
    },
    /// Store a digest of the JavaScript each elm generates for every
    /// corpus app in dev, `--debug` and `--optimize`.
    CorpusJavascript {
        /// Concurrent elm runs; the available cores by default.
        #[arg(long)]
        jobs: Option<usize>,
        /// Print the cells of the apps whose case contains this text
        /// instead.
        #[arg(long)]
        only: Option<String>,
    },
    /// Store a digest of the `.elmo` of every module each elm builds for
    /// every corpus app and package, as `corpus-check` runs elm.
    /// Store a digest of each answer elm-language-server gives in the
    /// session of every sampled corpus project.
    CorpusLsp {
        /// Concurrent sessions; two by default.
        #[arg(long)]
        jobs: Option<usize>,
        /// Only projects whose case contains this text.
        #[arg(long)]
        only: Option<String>,
    },
    /// Store elm-test's outcome on every corpus app's test suite (plan
    /// 10.7), downloading its packages with elm into a sandbox.
    CorpusTests {
        /// Only apps whose name contains this text.
        #[arg(long)]
        only: Option<String>,
    },
    /// Have Helix edit a corpus app with elm-language-server, driven
    /// through tmux, and keep the session its messages make.
    EditorTrace {
        /// The app, by its name in `corpus/apps.toml`.
        app: String,
    },
    CorpusGraphs {
        /// Concurrent elm runs; the available cores by default.
        #[arg(long)]
        jobs: Option<usize>,
        /// Print the cells of the projects whose case contains this text
        /// instead.
        #[arg(long)]
        only: Option<String>,
    },
    /// Store what each elm makes of every corpus app and package with
    /// `elm make --output=/dev/null`, offline, as `corpus-docs` does.
    CorpusCheck {
        /// Concurrent elm runs; the available cores by default.
        #[arg(long)]
        jobs: Option<usize>,
        /// Print the cells of the projects whose case contains this text
        /// instead.
        #[arg(long)]
        only: Option<String>,
    },
    /// Build the solver oracles and store what upstream's solver answers
    /// for every solver scenario.
    Solver,
    /// Store elm-language-server's transcript of every language-server
    /// case.
    Lsp {
        /// Only the cases whose names start with this.
        only: Option<String>,
    },
    /// Write elm-language-server's provider tests as language-server
    /// cases.
    LspImport,
    /// Write elm-language-server's completion data: its package ranking,
    /// snippets and keywords.
    LspTables,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let result = match cli.command {
        Command::Ci => ci(),
        Command::Check => architecture(&root()),
        Command::Codegen => codegen::run(&root(), false),
        Command::Refs => references::load(&root()).and_then(|r| refs::sync(&root(), &r)),
        Command::Oracle { command } => references::load(&root()).and_then(|r| match command {
            OracleCommand::Fetch => oracle::fetch(&root(), &r),
            OracleCommand::Versions => {
                oracle::versions(&r);
                Ok(())
            }
            OracleCommand::Syntax {
                show: Some(case), ..
            } => oracle::syntax::show(&root(), &r, &case),
            OracleCommand::Syntax { inputs, jobs, .. } => {
                let jobs = jobs.unwrap_or_else(|| {
                    2 * std::thread::available_parallelism().map_or(1, |n| n.get())
                });
                oracle::syntax::run(&root(), &r, inputs, jobs)
            }
            OracleCommand::Format { inputs, jobs } => {
                let jobs = jobs.unwrap_or_else(|| {
                    2 * std::thread::available_parallelism().map_or(1, |n| n.get())
                });
                oracle::format::run(&root(), &r, inputs, jobs)
            }
            OracleCommand::Transcripts { tool } => {
                oracle::transcripts::run(&root(), &r, tool.as_deref())
            }
            OracleCommand::Outline => oracle::outline::run(&root(), &r),
            OracleCommand::Modules => oracle::modules::run(&root(), &r),
            OracleCommand::Names => oracle::names::run(&root(), &r),
            OracleCommand::Types => oracle::types::run(&root(), &r),
            OracleCommand::Patterns => oracle::types::patterns(&root(), &r),
            OracleCommand::Docs => oracle::types::docs(&root(), &r),
            OracleCommand::Make => oracle::make::run(&root(), &r),
            OracleCommand::Diff => oracle::diff::run(&root(), &r),
            OracleCommand::Publish => oracle::diff::publish(&root(), &r),
            OracleCommand::Optimized => oracle::optimized::run(&root(), &r),
            OracleCommand::Javascript => oracle::javascript::run(&root(), &r),
            OracleCommand::CorpusDocs { jobs, only } => {
                oracle::corpus_runs::run(&root(), &r, Suite::Docs, jobs, only.as_deref())
            }
            OracleCommand::CorpusJavascript { jobs, only } => {
                oracle::corpus_runs::run(&root(), &r, Suite::JavaScript, jobs, only.as_deref())
            }
            OracleCommand::CorpusObjects { jobs, only } => {
                oracle::corpus_runs::run(&root(), &r, Suite::Objects, jobs, only.as_deref())
            }
            OracleCommand::CorpusLsp { jobs, only } => {
                corpus::lsp::record(&root(), &r, jobs, only.as_deref())
            }
            OracleCommand::CorpusTests { only } => {
                corpus::test_suites::record(&root(), &r, only.as_deref())
            }
            OracleCommand::EditorTrace { app } => corpus::trace::record(&root(), &r, &app),
            OracleCommand::CorpusGraphs { jobs, only } => {
                oracle::corpus_runs::run(&root(), &r, Suite::Graphs, jobs, only.as_deref())
            }
            OracleCommand::CorpusCheck { jobs, only } => {
                oracle::corpus_runs::run(&root(), &r, Suite::Check, jobs, only.as_deref())
            }
            OracleCommand::Solver => oracle::solver::run(&root(), &r),
            OracleCommand::Lsp { only } => oracle::lsp::run(&root(), &r, only.as_deref()),
            OracleCommand::LspImport => oracle::lsp_import::run(&root()),
            OracleCommand::LspTables => oracle::lsp_tables::run(&root()),
        }),
        Command::Ratchet { suite } => ratchet(&root(), &suite),
        Command::Fuzz { target, runs, jobs } => fuzz::run(
            &root(),
            &target,
            runs,
            jobs.unwrap_or_else(|| std::thread::available_parallelism().map_or(1, |n| n.get())),
        ),
        Command::Bench { target: None } => {
            references::load(&root()).and_then(|r| bench::run(&root(), &r, None))
        }
        Command::Bench {
            target: Some(target),
        } if target == "elm" || target == "elm-format" || target == "lsp" => {
            references::load(&root()).and_then(|r| bench::run(&root(), &r, Some(&target)))
        }
        Command::Bench {
            target: Some(target),
        } => cargo(
            &root(),
            &[
                "bench",
                "--locked",
                "--package",
                &format!("nomi-{target}"),
                "--bench",
                &target,
            ],
        ),
        Command::Corpus { command } => match command {
            CorpusCommand::Pin => corpus::packages::pin(&root()),
            CorpusCommand::Fetch => {
                corpus::packages::fetch(&root()).and_then(|()| corpus::apps::fetch(&root()))
            }
            CorpusCommand::Classify => corpus::compat::classify(&root()),
            CorpusCommand::Stats => corpus::stats(&root()),
            CorpusCommand::Syntax { record } => corpus::syntax::check(&root(), record),
            CorpusCommand::Format { record } => corpus::format::check(&root(), record),
            CorpusCommand::Hir => corpus::hir::check(&root()),
            CorpusCommand::Fixtures => corpus::fixtures::run(&root()),
            CorpusCommand::Names { only } => corpus::names::check(&root(), only.as_deref()),
            CorpusCommand::Docs { only } => {
                corpus::runs::check(&root(), Suite::Docs, only.as_deref())
            }
            CorpusCommand::Check { only } => {
                corpus::runs::check(&root(), Suite::Check, only.as_deref())
            }
            CorpusCommand::Javascript { only } => {
                corpus::runs::check(&root(), Suite::JavaScript, only.as_deref())
            }
            CorpusCommand::Objects { only } => {
                corpus::runs::check(&root(), Suite::Objects, only.as_deref())
            }
            CorpusCommand::GraphDiff { release, nomi, elm } => {
                corpus::runs::graph_diff(&release, &nomi, &elm)
            }
            CorpusCommand::Lsp { record, only } => {
                corpus::lsp::check(&root(), record, only.as_deref())
            }
            CorpusCommand::Graphs { only } => {
                corpus::runs::check(&root(), Suite::Graphs, only.as_deref())
            }
            CorpusCommand::Outline => corpus::outline::check(&root()),
            CorpusCommand::Deps => corpus::deps::check(&root()),
            CorpusCommand::Tests { only } => corpus::test_suites::check(&root(), only.as_deref()),
        },
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error:#}");
            ExitCode::FAILURE
        }
    }
}

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask lives in the workspace root")
        .to_path_buf()
}

fn ci() -> Result<()> {
    let root = root();
    eprintln!("==> codegen --check");
    codegen::run(&root, true)?;
    cargo(&root, &["fmt", "--all", "--check"])?;
    cargo(
        &root,
        &[
            "clippy",
            "--workspace",
            "--all-targets",
            "--locked",
            "--",
            "-D",
            "warnings",
        ],
    )?;
    // Every failing test binary, not only the first: a platform that
    // fails in several places shows them all in one run.
    cargo(
        &root,
        &["test", "--workspace", "--locked", "--no-fail-fast"],
    )?;
    // fuzz/ is outside the workspace, for its own build flags.
    let fuzz = root.join("fuzz/Cargo.toml");
    let fuzz = fuzz.to_str().context("the checkout's path is not UTF-8")?;
    cargo(&root, &["fmt", "--manifest-path", fuzz, "--check"])?;
    cargo(
        &root,
        &[
            "clippy",
            "--manifest-path",
            fuzz,
            "--locked",
            "--",
            "-D",
            "warnings",
        ],
    )?;
    runtime_tests(&root)?;
    architecture(&root)
}

/// The JS fiber runtime's tests. Node is required: tier 1 fails rather than
/// skipping them when it is missing or too old.
fn runtime_tests(root: &Path) -> Result<()> {
    let refs = references::load(root)?;
    let version = oracle::node_version(refs.node_minimum)?;
    eprintln!("==> node --test runtime/ ({version})");
    let status = Process::new("node")
        .arg("--test")
        .current_dir(root.join("runtime"))
        .status()
        .context("running node --test")?;
    if !status.success() {
        bail!("runtime tests failed");
    }
    Ok(())
}

/// Runs the suite's test target with the ratchet allowed to record newly
/// passing cases; regressions still fail it.
fn ratchet(root: &Path, suite: &str) -> Result<()> {
    let mut owners = Vec::new();
    for entry in std::fs::read_dir(root.join("crates")).context("reading crates/")? {
        let dir = entry?.path();
        if dir.join("tests").join(format!("{suite}.rs")).is_file() {
            owners.push(dir);
        }
    }
    let [dir] = owners.as_slice() else {
        bail!(
            "expected one crates/*/tests/{suite}.rs, found {}",
            owners.len()
        );
    };
    let manifest = std::fs::read_to_string(dir.join("Cargo.toml"))?;
    let package: toml::Table = toml::from_str(&manifest)?;
    let name = package
        .get("package")
        .and_then(|p| p.get("name"))
        .and_then(|n| n.as_str())
        .context("the crate's Cargo.toml has no package name")?;
    cargo_with(
        root,
        &[
            "test",
            "--package",
            name,
            "--test",
            suite,
            "--",
            "--nocapture",
        ],
        &[(nomi_testkit::ratchet::UPDATE_VAR, "1")],
    )
}

fn architecture(root: &Path) -> Result<()> {
    eprintln!("==> architecture checks");
    let violations = checks::run(root)?;
    for violation in &violations {
        eprintln!("  {violation}");
    }
    if !violations.is_empty() {
        bail!("{} architecture violation(s)", violations.len());
    }
    Ok(())
}

fn cargo(root: &Path, args: &[&str]) -> Result<()> {
    cargo_with(root, args, &[])
}

fn cargo_with(root: &Path, args: &[&str], envs: &[(&str, &str)]) -> Result<()> {
    eprintln!("==> cargo {}", args.join(" "));
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string());
    let status = Process::new(cargo)
        .args(args)
        .envs(envs.iter().copied())
        .current_dir(root)
        .status()
        .with_context(|| format!("running cargo {}", args[0]))?;
    if !status.success() {
        bail!("cargo {} failed", args[0]);
    }
    Ok(())
}
