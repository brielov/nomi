//! `nomi run`: an entry of the project's `nomi.toml`, or a module named
//! on the command line, built for Node as a self-starting ES module and
//! run under `node`, which the run stands in for: the arguments are the
//! program's, the signals nomi is sent reach it, and nomi exits as it
//! does. With `--watch` it is built and run again as the project changes.

use std::ffi::OsString;
use std::io;
use std::path::{Component, Path, PathBuf};
use std::process::{Child, Command, ExitStatus};
use std::thread;
use std::time::{Duration, Instant};

use nomi_base::{Compat, Platform, Report};
use nomi_project::{Entry, Manifest, report};
use nomi_registry::Http;

use crate::check::Style;
use crate::console::{self, Console, Personality};
use crate::make::{self, Make, Output};
use crate::packages::Resolve;
use crate::paths::{NOMI_TOML, Root, relative};
use crate::project::{Project, no_outline};
use crate::report::one_line;
use crate::watch::{Change, POLL, Snapshot, message};
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
    let Some(built) = build(cx, &mut Stderr(console), program) else {
        return 1;
    };
    match supervise(&mut node(&built.output, args)) {
        Ok(status) => exit_code(status),
        Err(e) => {
            console::report(console, &no_node(&e), Compat::LATEST);
            1
        }
    }
}

/// Builds and runs `program` as [`run`] does, then again each time the
/// project's source directories, `elm.json`, `nomi.toml` or the entry's
/// FFI module change, stopping the program first. A failed build leaves
/// it stopped until the next change, and one that ends by itself is
/// started again on the next change. It runs until nomi is sent SIGINT or
/// SIGTERM, which stops the program as [`run`] passes signals on; the
/// code to exit with is then the shell's for that signal, 130 or 143,
/// whatever the program's, or 1 if the watch couldn't start.
pub fn watch(
    cx: &Context<'_, impl Http + Sync>,
    console: &mut dyn Console,
    program: &Program,
    args: &[OsString],
) -> u8 {
    let console = &mut Stderr(console);
    let Some(root) = Root::find(cx.vfs, cx.dir) else {
        console::report(console, &no_outline("to run"), Compat::LATEST);
        return 1;
    };
    let mut caught = match Caught::new() {
        Ok(caught) => caught,
        Err(e) => {
            console.err(&format!("I could not watch for signals: {e}\n"));
            return 1;
        }
    };
    let manifests = vec![root.elm_json(), root.dir().join(NOMI_TOML)];
    let mut snapshot = Snapshot::new(root.dir(), manifests);
    let mut queue: Vec<Change> = Vec::new();
    let mut child: Option<Child> = None;
    loop {
        if !queue.is_empty() {
            // Changes made together join the build they start.
            thread::sleep(POLL);
            queue.extend(snapshot.changes());
            console.err(&format!("{}\n", message(&queue)));
            queue.clear();
            cx.vfs.disk_changed();
        }
        let built = build(cx, console, program);
        snapshot.watch(watched(cx, &root, program));
        if let Some(mut old) = child.take() {
            let _ = stop(console, &mut old, Some(Stop::Terminate));
        }
        if let Some(built) = built {
            child = (node(&built.output, args).spawn())
                .inspect_err(|e| console::report(console, &no_node(e), Compat::LATEST))
                .ok();
        }
        if child.is_none() {
            console.err("Watching for changes...\n");
        }
        while queue.is_empty() {
            if let Some((signal, sent)) = caught.take() {
                if let Some(mut child) = child {
                    let _ = stop(console, &mut child, sent.then_some(signal));
                }
                return signal.code();
            }
            if let Some(status) = child.as_mut().and_then(|c| c.try_wait().ok().flatten()) {
                let code = exit_code(status);
                console.err(&format!(
                    "The program exited with code {code}. Watching for changes...\n"
                ));
                child = None;
            }
            queue.extend(snapshot.changes());
            if queue.is_empty() {
                thread::sleep(POLL);
            }
        }
    }
}

/// `node` running `output` with `args`.
fn node(output: &Path, args: &[OsString]) -> Command {
    let mut node = Command::new("node");
    node.arg(output).args(args);
    node
}

/// What changes [`watch`] watches for besides the manifests: the
/// project's source directories, the module run and its FFI module, as
/// the project now names them.
fn watched(cx: &Context<'_, impl Http + Sync>, root: &Root, program: &Program) -> Vec<PathBuf> {
    let mut paths: Vec<PathBuf> = Project::read(cx.vfs, root, Personality::Nomi)
        .map(|project| (project.outline.source_dirs().into_iter()).map(|d| root.dir().join(d)))
        .into_iter()
        .flatten()
        .collect();
    if let Ok(built) = chosen(cx, program) {
        paths.push(built.dir.join(&built.main));
        paths.extend(built.ffi.map(|ffi| built.dir.join(ffi)));
    }
    paths
}

/// Builds `program`, showing how it went; what was built, if it was.
fn build(
    cx: &Context<'_, impl Http + Sync>,
    console: &mut dyn Console,
    program: &Program,
) -> Option<Built> {
    let built = chosen(cx, program)
        .inspect_err(|report| console::report(console, report, Compat::LATEST))
        .ok()?;
    let make = Make {
        paths: vec![built.main.clone()],
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
        resolve: Resolve::Nomi,
        ffi: built.ffi.clone(),
        output: Some(Output::Js(built.output.clone())),
        docs: None,
    };
    let cx = Context {
        dir: &built.dir,
        ..*cx
    };
    make::run(&cx, console, &make).then_some(built)
}

/// What `program` builds, from `nomi.toml`'s entry where it names one.
fn chosen(cx: &Context<'_, impl Http + Sync>, program: &Program) -> Result<Built, Report> {
    let root = Root::find(cx.vfs, cx.dir).ok_or_else(|| no_outline("to run"))?;
    // Kept by the module's path in the project, so that no two share one.
    let default_output = |main: &Path| {
        let mut name = relative(main, root.dir());
        if !(name.components()).all(|c| matches!(c, Component::Normal(_))) {
            name = main.file_name().unwrap_or(main.as_os_str()).into();
        }
        (root.dir().join("elm-stuff/nomi/run").join(name)).with_extension("mjs")
    };
    let chosen = match program {
        Program::Module(main) => Built {
            main: main.clone(),
            output: default_output(&cx.dir.join(main)),
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
                None => default_output(&root.dir().join(&entry.main)),
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
    let forwarding = thread::spawn(move || {
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

/// A signal that stops a program, as nomi passes it on.
#[derive(Clone, Copy, Debug)]
enum Stop {
    Interrupt,
    Terminate,
}

impl Stop {
    /// The code a shell gives a process this signal stopped.
    fn code(self) -> u8 {
        match self {
            Stop::Interrupt => 130,
            Stop::Terminate => 143,
        }
    }
}

/// How long a program has to stop before it is killed: longer than the
/// runtime's own shutdown deadline, which a server lengthens to its drain
/// and 5 seconds more.
const GRACE: Duration = Duration::from_secs(15);

/// Stops `child`, sending it `signal` if one is given (a terminal's
/// reaches it already), and kills it if it hasn't ended within [`GRACE`].
fn stop(
    console: &mut dyn Console,
    child: &mut Child,
    signal: Option<Stop>,
) -> io::Result<ExitStatus> {
    if let Some(signal) = signal {
        send(child, signal)?;
    }
    let start = Instant::now();
    while start.elapsed() < GRACE {
        if let Some(status) = child.try_wait()? {
            return Ok(status);
        }
        thread::sleep(Duration::from_millis(10));
    }
    console.err(&format!(
        "The program did not stop within {} seconds, so I killed it.\n",
        GRACE.as_secs()
    ));
    child.kill()?;
    child.wait()
}

#[cfg(unix)]
fn send(child: &Child, signal: Stop) -> io::Result<()> {
    use rustix::process::{Pid, Signal, kill_process};
    let signal = match signal {
        Stop::Interrupt => Signal::INT,
        Stop::Terminate => Signal::TERM,
    };
    Ok(kill_process(Pid::from_child(child), signal)?)
}

/// Windows has no signal a program can finish on, so it ends at once.
#[cfg(not(unix))]
fn send(child: &mut Child, _: Stop) -> io::Result<()> {
    child.kill()
}

/// The SIGINT and SIGTERM nomi is sent while it watches, caught.
#[cfg(unix)]
struct Caught(signal_hook::iterator::SignalsInfo<signal_hook::iterator::exfiltrator::WithOrigin>);

#[cfg(unix)]
impl Caught {
    fn new() -> io::Result<Caught> {
        use signal_hook::consts::{SIGINT, SIGTERM};
        Ok(Caught(signal_hook::iterator::SignalsInfo::new([
            SIGINT, SIGTERM,
        ])?))
    }

    /// The last signal since the last call, and whether a process sent
    /// it rather than the terminal.
    fn take(&mut self) -> Option<(Stop, bool)> {
        use signal_hook::low_level::siginfo::Cause;
        let origin = self.0.pending().last()?;
        let signal = if origin.signal == signal_hook::consts::SIGINT {
            Stop::Interrupt
        } else {
            Stop::Terminate
        };
        Some((signal, matches!(origin.cause, Cause::Sent(_))))
    }
}

/// A console's Ctrl-C ends nomi and the program together, so nothing is
/// caught.
#[cfg(not(unix))]
struct Caught;

#[cfg(not(unix))]
impl Caught {
    fn new() -> io::Result<Caught> {
        Ok(Caught)
    }

    fn take(&mut self) -> Option<(Stop, bool)> {
        None
    }
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
