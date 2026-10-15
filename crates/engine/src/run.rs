//! `nomi run`: an entry of the project's `nomi.toml`, or a module named
//! on the command line, built for Node as a self-starting ES module and
//! run under `node`, which the run stands in for: the arguments are the
//! program's, the signals nomi is sent reach it, and nomi exits as it
//! does.

use std::ffi::OsString;
use std::io;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, ExitStatus};

use nomi_base::{Compat, Platform, Report};
use nomi_project::{Entry, Manifest, report};
use nomi_registry::Http;

use crate::check::Style;
use crate::console::{self, Console, Personality};
use crate::make::{self, Make, Output};
use crate::paths::{NOMI_TOML, Root};
use crate::project::no_outline;
use crate::report::one_line;
use crate::{Context, javascript};

/// What `nomi run` was asked to run.
#[derive(Clone, Debug)]
pub enum Program {
    /// The entry of `nomi.toml` whose `main` is this path, from the
    /// directory; the only one if none is given.
    Entry(Option<PathBuf>),
    /// This module's `main`, from the directory, without `nomi.toml`.
    Module(PathBuf),
}

/// Where a program to run comes from, and how it is built.
struct Built {
    /// The module's path, from `dir`.
    main: PathBuf,
    /// The ES module to write, absolute.
    output: PathBuf,
    /// The FFI module's path, from `dir`.
    ffi: Option<PathBuf>,
    /// What `main` and `ffi` are relative to.
    dir: PathBuf,
}

/// Builds `program` in the project around the directory and runs it
/// under `node` with `args`, showing on standard error how the build went
/// and why it didn't run. The code to exit with: the program's, 128 plus
/// the signal that killed it, or 1 if it didn't run.
pub fn run(
    cx: &Context<'_, impl Http + Sync>,
    console: &mut dyn Console,
    program: &Program,
    args: &[OsString],
) -> u8 {
    let Some(output) = build(cx, &mut Stderr(console), program) else {
        return 1;
    };
    let mut node = Command::new("node");
    node.arg(output).args(args);
    match supervise(&mut node) {
        Ok(status) => exit_code(status),
        Err(e) => {
            console::report(console, &no_node(&e), Compat::LATEST);
            1
        }
    }
}

/// Builds `program`, showing how it went; the ES module written, if one
/// was.
fn build(
    cx: &Context<'_, impl Http + Sync>,
    console: &mut dyn Console,
    program: &Program,
) -> Option<PathBuf> {
    let built = chosen(cx, program)
        .inspect_err(|report| console::report(console, report, Compat::LATEST))
        .ok()?;
    let make = Make {
        paths: vec![built.main],
        personality: Personality::Nomi,
        style: Style::Terminal,
        kind: Ok(javascript::Kind::Dev),
        target: javascript::Target {
            platform: Platform::Node,
            format: javascript::Format::EsModule,
            dom: javascript::Dom::Native,
            ffi: None,
        },
        platform_by_main: false,
        ffi: built.ffi,
        output: Some(Output::Js(built.output.clone())),
        docs: None,
    };
    let cx = Context {
        dir: &built.dir,
        ..*cx
    };
    make::run(&cx, console, &make).then_some(built.output)
}

/// What `program` builds, from `nomi.toml`'s entry where it names one.
fn chosen(cx: &Context<'_, impl Http + Sync>, program: &Program) -> Result<Built, Report> {
    let root = Root::find(cx.vfs, cx.dir).ok_or_else(|| no_outline("to run"))?;
    let default_output = |main: &Path| {
        let name = main.file_stem().unwrap_or(main.as_os_str());
        (root.dir().join("elm-stuff/nomi/run").join(name)).with_extension("mjs")
    };
    let chosen = match program {
        Program::Module(main) => Built {
            main: main.clone(),
            output: default_output(main),
            ffi: None,
            dir: cx.dir.to_owned(),
        },
        Program::Entry(chosen) => {
            let entry = entry(cx, &root, chosen.as_deref())?;
            if entry.platform == Some(Platform::Browser) {
                return Err(browser_entry(&entry.main));
            }
            let output = match &entry.output {
                Some(output) if !make::has_ext(".mjs", output) => {
                    return Err(not_a_module(&entry.main, output));
                }
                Some(output) => root.dir().join(output),
                None => default_output(Path::new(&entry.main)),
            };
            Built {
                main: entry.main.into(),
                output,
                ffi: entry.ffi.map(PathBuf::from),
                dir: root.dir().to_owned(),
            }
        }
    };
    Ok(chosen)
}

/// The entry of the `nomi.toml` at `root` whose `main` is `chosen`, from
/// the directory, or its only entry.
fn entry(
    cx: &Context<'_, impl Http + Sync>,
    root: &Root,
    chosen: Option<&Path>,
) -> Result<Entry, Report> {
    let path = root.dir().join(NOMI_TOML);
    let src = cx.vfs.read_bytes(&path).map_err(|e| match e.kind() {
        io::ErrorKind::NotFound => no_manifest(),
        _ => one_line(
            "PROBLEM READING NOMI.TOML",
            &format!("I could not read nomi.toml: {e}"),
        ),
    })?;
    let manifest =
        Manifest::parse(&src).map_err(|error| report::manifest(&error, &src, Compat::LATEST))?;
    let mains = || {
        let mains: Vec<&str> = manifest.entries.iter().map(|e| e.main.as_str()).collect();
        mains.join(", ")
    };
    match (chosen, &manifest.entries[..]) {
        (None, []) => Err(no_entries()),
        (None, [entry]) => Ok(entry.clone()),
        (None, _) => Err(which_entry(&mains())),
        (Some(chosen), entries) => {
            let wanted = lexical(&cx.dir.join(chosen));
            (entries.iter())
                .find(|entry| lexical(&root.dir().join(&entry.main)) == wanted)
                .cloned()
                .ok_or_else(|| unknown_entry(&chosen.display().to_string(), &mains()))
        }
    }
}

/// `path` without its `.` components, as entries are compared.
fn lexical(path: &Path) -> PathBuf {
    path.components()
        .filter(|c| *c != Component::CurDir)
        .collect()
}

/// A console whose standard output is the other's standard error, so
/// standard output is the program's alone.
struct Stderr<'a>(&'a mut dyn Console);

impl Console for Stderr<'_> {
    fn out(&mut self, text: &str) {
        self.0.err(text);
    }

    fn err(&mut self, text: &str) {
        self.0.err(text);
    }

    fn read_line(&mut self) -> io::Result<Option<String>> {
        self.0.read_line()
    }

    fn read_all(&mut self) -> io::Result<Vec<u8>> {
        self.0.read_all()
    }

    fn out_is_terminal(&self) -> bool {
        self.0.err_is_terminal()
    }

    fn err_is_terminal(&self) -> bool {
        self.0.err_is_terminal()
    }
}

/// Runs `command` to its end, passing on the SIGINT and SIGTERM nomi is
/// sent. A terminal's own signals aren't passed on: the terminal sends
/// them to its whole foreground process group, the child included, and
/// a second signal stops a Node program without its finalizers.
#[cfg(unix)]
fn supervise(command: &mut Command) -> io::Result<ExitStatus> {
    use rustix::process::{Pid, Signal, kill_process};
    use signal_hook::consts::{SIGINT, SIGTERM};
    use signal_hook::iterator::SignalsInfo;
    use signal_hook::iterator::exfiltrator::WithOrigin;
    use signal_hook::low_level::siginfo::Cause;

    // Caught before the child starts, so neither signal can stop nomi
    // alone once it has.
    let mut signals = SignalsInfo::<WithOrigin>::new([SIGINT, SIGTERM])?;
    let handle = signals.handle();
    let mut child = command.spawn()?;
    let pid = Pid::from_child(&child);
    let forwarding = std::thread::spawn(move || {
        for origin in signals.forever() {
            if let Cause::Sent(_) = origin.cause {
                let signal = if origin.signal == SIGINT {
                    Signal::INT
                } else {
                    Signal::TERM
                };
                let _ = kill_process(pid, signal);
            }
        }
    });
    let status = child.wait();
    handle.close();
    let _ = forwarding.join();
    status
}

/// Runs `command` to its end. A console's Ctrl-C reaches every process
/// attached to it, the child included, so there is nothing to pass on.
#[cfg(not(unix))]
fn supervise(command: &mut Command) -> io::Result<ExitStatus> {
    command.status()
}

/// The code a shell gives a process that ended as `status` did.
fn exit_code(status: ExitStatus) -> u8 {
    #[cfg(unix)]
    if let Some(signal) = std::os::unix::process::ExitStatusExt::signal(&status) {
        return u8::try_from(128 + signal).unwrap_or(u8::MAX);
    }
    (status.code()).map_or(1, |code| u8::try_from(code).unwrap_or(1))
}

fn no_node(error: &io::Error) -> Report {
    if error.kind() == io::ErrorKind::NotFound {
        one_line(
            "NO NODE",
            "nomi run runs the program under Node, but I cannot find `node` on the PATH. \
             Install Node 22 or later, or add its directory to the PATH.",
        )
    } else {
        one_line(
            "PROBLEM RUNNING NODE",
            &format!("I could not run node: {error}"),
        )
    }
}

fn no_manifest() -> Report {
    one_line(
        "NO NOMI.TOML",
        "There is no nomi.toml next to elm.json, so I do not know what to run. Add an entry \
         to one, as [[entry]] with main = \"src/Main.elm\", or name the module to run, as in \
         nomi run -e src/Main.elm",
    )
}

fn no_entries() -> Report {
    one_line(
        "NO ENTRY",
        "nomi.toml has no entry to run. Add one, as [[entry]] with main = \"src/Main.elm\".",
    )
}

fn which_entry(mains: &str) -> Report {
    one_line(
        "WHICH ENTRY?",
        &format!(
            "nomi.toml has several entries, so I need to know which one to run. Name it by its \
             main, as in nomi run src/Main.elm. The entries are: {mains}"
        ),
    )
}

fn unknown_entry(chosen: &str, mains: &str) -> Report {
    one_line(
        "UNKNOWN ENTRY",
        &format!(
            "No entry of nomi.toml has {chosen} as its main. The entries are: {mains}. To run \
             a module that isn't an entry, give it with -e."
        ),
    )
}

fn browser_entry(main: &str) -> Report {
    one_line(
        "BROWSER ENTRY",
        &format!(
            "The entry for {main} runs in the browser, so there is nothing for nomi run to run \
             under Node. Build it with nomi build instead."
        ),
    )
}

fn not_a_module(main: &str, output: &str) -> Report {
    one_line(
        "NOT AN ES MODULE",
        &format!(
            "The entry for {main} is written to {output}, but nomi run runs an ES module, a \
             file ending in .mjs. Change its output, or leave it out to have nomi choose one."
        ),
    )
}
