//! `nomi run`: an entry of the project's `nomi.toml`, or a module named
//! on the command line, built for Node as a self-starting ES module and
//! run under `node`, which the run stands in for: the arguments are the
//! program's, the signals nomi is sent reach it, and nomi exits as it
//! does. With `--watch` it is built and run again as the project changes.
//! A browser entry is built and served until nomi is stopped. Without an
//! entry named, every entry of the project, the member or the workspace
//! runs, each under its member's name.

mod many;
mod port;

use std::ffi::OsString;
use std::io;
use std::path::{Component, Path, PathBuf};
use std::process::{Child, Command, ExitStatus};
use std::thread;
use std::time::Duration;

use nomi_base::{Compat, Platform, Report};
use nomi_project::workspace::Workspace;
use nomi_project::{Entry, Manifest, ModuleName, report};
use nomi_registry::Http;

use crate::console::{self, Console};
use crate::javascript::{Advice, Start, Target};
use crate::make::{self, Code, FfiModule, Make, NamedIn, OutputName};
use crate::paths::{NOMI_TOML, Root, lexical, relative};
use crate::project::no_outline;
use crate::report::one_line;
use crate::scope::{self, Scope};
use crate::serve::{Reloads, Server, Site};
use crate::{Context, workspace};
use port::{FIRST_PORT, Port};

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
    /// The code to write, absolute: an ES module on Node.
    output: PathBuf,
    /// What decides `output`'s extension: the platform, where nothing
    /// names it.
    output_name: OutputName,
    /// The FFI module, its path from `dir`.
    ffi: Option<FfiModule>,
    /// What `main` and `ffi` are relative to.
    dir: PathBuf,
    /// Where it runs: by `main`'s type where nothing says.
    platform: Option<Platform>,
    /// The arguments the entry gives the program.
    args: Vec<String>,
    /// The directory of a browser program's static files, from `dir`.
    static_dir: Option<String>,
    /// The port a browser program is served on.
    port: Option<u16>,
}

impl Built {
    /// `node` running the program with `args`, or the entry's own without
    /// them.
    fn node(&self, args: &[OsString]) -> Command {
        if args.is_empty() {
            let args: Vec<OsString> = self.args.iter().map(OsString::from).collect();
            node(&self.output, &args)
        } else {
            node(&self.output, args)
        }
    }
}

/// Builds `program` and runs it under `node` with `args`, in the member
/// `member` names or the project around the directory, as [`one`] does;
/// or, where no entry is named and there are several, builds them all
/// and runs every Node program among them together, each with its
/// entry's arguments, as `many` does. The code to exit with.
pub fn run(
    cx: &Context<'_, impl Http + Sync>,
    console: &mut dyn Console,
    program: &Program,
    member: Option<&str>,
    args: &[OsString],
) -> u8 {
    by_scope(
        cx,
        console,
        program,
        member,
        args,
        |console, workspace, dirs| many::run(cx, console, &workspace, &dirs),
        |cx, console| one(cx, console, program, args),
    )
}

/// Runs `one` in the project that runs `program`, in the member `member`
/// names or the project around the directory; or, where no entry is
/// named and the members in scope have several, `several` with the
/// workspace and those members' directories, unless `args` are given;
/// what it gives, or 1 once what was wrong is shown.
fn by_scope<H: Http + Sync>(
    cx: &Context<'_, H>,
    console: &mut dyn Console,
    program: &Program,
    member: Option<&str>,
    args: &[OsString],
    several: impl FnOnce(&mut dyn Console, Workspace, Vec<PathBuf>) -> u8,
    one: impl FnOnce(&Context<'_, H>, &mut dyn Console) -> u8,
) -> u8 {
    let failed = |console: &mut dyn Console, report: &Report| {
        console::report(console, report, Compat::LATEST);
        1
    };
    if let Program::Entry(None) = program {
        match entries(cx, member) {
            Ok(Entries::Several(..)) if !args.is_empty() => {
                return failed(console, &arguments_for_several());
            }
            Ok(Entries::Several(workspace, dirs)) => return several(console, workspace, dirs),
            Ok(Entries::One(dir)) => return one(&Context { dir: &dir, ..*cx }, console),
            Err(report) => return failed(console, &report),
        }
    }
    scope::in_one(cx, console, member, "run", 1, one)
}

/// The entries `nomi run` without one named runs.
enum Entries {
    /// The one entry, or none, which [`one`] says, of the project in this
    /// directory.
    One(PathBuf),
    /// The entries of these members of the workspace, more than one.
    Several(Workspace, Vec<PathBuf>),
}

/// The entries of the member `member` names, or of the project around
/// the directory, or, outside every member, of the whole workspace.
fn entries(cx: &Context<'_, impl Http + Sync>, member: Option<&str>) -> Result<Entries, Report> {
    let Some((workspace, dirs)) = scoped(cx, member)? else {
        return Ok(Entries::One(cx.dir.to_owned()));
    };
    let counts = counted(&workspace, &dirs);
    match (counts.as_slice(), &dirs[..]) {
        ([(dir, 1)], _) => Ok(Entries::One(workspace.root().join(dir))),
        ([], [dir]) => Ok(Entries::One(workspace.root().join(dir))),
        ([], _) => Err(no_member_entries()),
        _ => {
            let dirs = counts.into_iter().map(|(dir, _)| dir.clone()).collect();
            Ok(Entries::Several(workspace, dirs))
        }
    }
}

/// The workspace around the directory and the directories of its members
/// `member` names, or the member the directory is in, or, outside every
/// member, all of them; or the project around the directory alone, as
/// a workspace of its one member where its `nomi.toml` has entries.
/// `None` where it has none, or is missing or wrong, which [`one`] says.
fn scoped(
    cx: &Context<'_, impl Http + Sync>,
    member: Option<&str>,
) -> Result<Option<(Workspace, Vec<PathBuf>)>, Report> {
    let specs: Vec<String> = member.iter().map(|m| (*m).to_owned()).collect();
    Ok(Some(match scope::find(cx, &specs)? {
        Scope::Project => {
            let alone = Root::find(cx.vfs, cx.dir)
                .and_then(|root| workspace::alone(&cx.vfs.snapshot(), root.dir()));
            match alone {
                Some(Ok(workspace)) => (workspace, vec![PathBuf::new()]),
                None | Some(Err(_)) => return Ok(None),
            }
        }
        Scope::Named(workspace, dirs) => (workspace::resolved(cx, workspace)?, dirs),
        Scope::Every(workspace) => {
            let dirs = (workspace.members().iter()).map(|m| m.dir.clone());
            let dirs = dirs.collect();
            (workspace::resolved(cx, workspace)?, dirs)
        }
    }))
}

/// Those of the members of `workspace` in `dirs` with entries, and how
/// many each has.
fn counted<'w>(workspace: &'w Workspace, dirs: &[PathBuf]) -> Vec<(&'w PathBuf, usize)> {
    (workspace.members().iter())
        .filter(|member| dirs.contains(&member.dir))
        .map(|member| {
            let entries = member.manifest.as_ref().map_or(0, |m| m.entries.len());
            (&member.dir, entries)
        })
        .filter(|&(_, entries)| entries > 0)
        .collect()
}

/// Builds `program` in the project around the directory and runs it
/// under `node` with `args`, or serves it if it runs in the browser,
/// showing on standard error how the build went and why it didn't run.
/// The code to exit with: the program's, 128 plus the signal that killed
/// it or stopped the server, or 1 if it didn't run.
fn one(
    cx: &Context<'_, impl Http + Sync>,
    console: &mut dyn Console,
    program: &Program,
    args: &[OsString],
) -> u8 {
    let Some((built, code)) = build(cx, &mut Stderr(console), program) else {
        return 1;
    };
    match code.platform {
        Platform::Node => match supervise(&mut built.node(args)) {
            Ok(status) => exit_code(status),
            Err(e) => {
                console::report(console, &no_node(&e), Compat::LATEST);
                1
            }
        },
        Platform::Browser => serve_one(console, &built, &code),
    }
}

/// Serves the browser program `built` wrote as `code` until nomi is sent
/// SIGINT or SIGTERM; the code to exit with, 130 or 143, or 1 if it
/// couldn't be served.
fn serve_one(console: &mut dyn Console, built: &Built, code: &Code) -> u8 {
    let mut caught = match Caught::new() {
        Ok(caught) => caught,
        Err(e) => {
            console.err(&format!("I could not watch for signals: {e}\n"));
            return 1;
        }
    };
    // Served until dropped, once nomi is stopped.
    let _server = match serve_built(console, built, code, Reloads::Never) {
        Ok(server) => server,
        Err(report) => {
            console::report(console, &report, Compat::LATEST);
            return 1;
        }
    };
    caught.wait().code()
}

/// Starts the dev server of the browser program `built` wrote as `code`,
/// its pages listening for reloads as `reloads` says, saying where.
fn serve_built(
    console: &mut dyn Console,
    built: &Built,
    code: &Code,
    reloads: Reloads,
) -> Result<Server, Report> {
    let main = built.main.to_string_lossy();
    let port = built.port.map_or(Port::Assigned(FIRST_PORT), Port::Given);
    let server = serve(Served {
        main: &main,
        output: &built.output,
        module: code.mains.first().expect("an entry has one main"),
        static_dir: built.static_dir.as_ref().map(|dir| built.dir.join(dir)),
        port,
        ports: &[Some(port)],
        reloads,
        from: &built.dir,
        dir: &built.dir,
    })?;
    console.err(&serving(&main, &server, port));
    Ok(server)
}

/// A browser program to serve.
struct Served<'a> {
    /// Its entry's `main`, as `nomi.toml` gives it.
    main: &'a str,
    /// Its code, absolute.
    output: &'a Path,
    /// The module whose `main` it starts.
    module: &'a ModuleName,
    /// The directory of its static files, absolute.
    static_dir: Option<PathBuf>,
    port: Port,
    /// The ports of every browser entry of the run, which an assigned
    /// `port` moves past.
    ports: &'a [Option<Port>],
    reloads: Reloads,
    /// Where the directory is named from when it is missing.
    from: &'a Path,
    /// Its project's directory, absolute, whose name titles a page the
    /// server makes.
    dir: &'a Path,
}

/// Starts the dev server of `served`.
fn serve(served: Served<'_>) -> Result<Server, Report> {
    let static_dir = match served.static_dir {
        Some(dir) => Some(
            dir.canonicalize()
                .map_err(|_| no_static_dir(served.main, &relative(&dir, served.from)))?,
        ),
        None => None,
    };
    let site = Site {
        code: served.output.to_owned(),
        module: served.module.to_string(),
        title: (served.dir.file_name())
            .map_or_else(String::new, |name| name.to_string_lossy().into_owned()),
        static_dir,
    };
    let ports = port::tries(served.port, served.ports);
    let port = served.port.number();
    Server::start(site, ports, served.reloads).map_err(|e| match e.kind() {
        io::ErrorKind::AddrInUse => port_taken(served.main, port),
        _ => one_line(
            "PROBLEM SERVING",
            &format!("I could not serve {} on port {port}: {e}", served.main),
        ),
    })
}

/// What nomi says once `server` serves the entry it names `name`, on
/// `port` or past it.
fn serving(name: &str, server: &Server, port: Port) -> String {
    let at = server.port();
    let moved = if at == port.number() {
        String::new()
    } else {
        format!(", as port {} is taken", port.number())
    };
    format!("Serving {name} at http://localhost:{at}{moved}\n")
}

/// Builds and runs `program` as [`run`] does, then again as the project
/// changes; or, where no entry is named and there are several, runs them
/// all and again as the workspace changes. One watch does both, as
/// `many` has it: one program is watched as a workspace whose one member
/// runs it alone. It runs until nomi is sent SIGINT or SIGTERM; the
/// code to exit with is then the shell's for that signal, 130 or 143, or
/// 1 if the watch couldn't start.
pub fn watch(
    cx: &Context<'_, impl Http + Sync>,
    console: &mut dyn Console,
    program: &Program,
    member: Option<&str>,
    args: &[OsString],
) -> u8 {
    by_scope(
        cx,
        console,
        program,
        member,
        args,
        |console, workspace, dirs| {
            many::watch(cx, console, &Watched::Members(member), workspace, dirs)
        },
        |cx, console| match alone(cx, program) {
            Ok((workspace, dir)) => {
                let watched = Watched::One { program, args };
                many::watch(cx, console, &watched, workspace, vec![dir])
            }
            Err(report) => {
                console::report(console, &report, Compat::LATEST);
                1
            }
        },
    )
}

/// What a watch runs, read again as the manifests change.
#[derive(Clone, Copy)]
enum Watched<'a> {
    /// The entries of the members in scope, as `-p` names them if given.
    Members(Option<&'a str>),
    /// The one program `nomi run` was asked for, in the project around
    /// the directory, with `args` in place of its entry's where given;
    /// its streams are its own, and nomi's lines bare.
    One {
        program: &'a Program,
        args: &'a [OsString],
    },
}

impl Watched<'_> {
    /// The workspace and the directories of the members whose entries run,
    /// as the files now say; `None` where nothing runs.
    fn read(
        self,
        cx: &Context<'_, impl Http + Sync>,
    ) -> Result<Option<(Workspace, Vec<PathBuf>)>, Report> {
        match self {
            Watched::Members(member) => Ok(scoped(cx, member)?.map(|(workspace, dirs)| {
                let dirs = counted(&workspace, &dirs);
                let dirs = dirs.into_iter().map(|(dir, _)| dir.clone()).collect();
                (workspace, dirs)
            })),
            Watched::One { program, .. } => {
                let (workspace, dir) = alone(cx, program)?;
                Ok(Some((workspace, vec![dir])))
            }
        }
    }
}

/// The workspace `program` runs in, the project around the directory
/// being a member of the one around it or standing alone, with that
/// project running `program` alone; and the project's directory in it.
fn alone(
    cx: &Context<'_, impl Http + Sync>,
    program: &Program,
) -> Result<(Workspace, PathBuf), Report> {
    let root = Root::find(cx.vfs, cx.dir).ok_or_else(|| no_outline("to run"))?;
    let entry = match program {
        Program::Entry(chosen) => entry(cx, &root, chosen.as_deref())?,
        Program::Module(main) => Entry {
            main: (relative(&lexical(&cx.dir.join(main)), root.dir()).to_string_lossy())
                .into_owned(),
            output: None,
            platform: Some(Platform::Node),
            ffi: None,
            args: Vec::new(),
            static_dir: None,
            port: None,
        },
    };
    let snapshot = cx.vfs.snapshot();
    let workspace = match workspace::discover(&snapshot, root.dir()) {
        Some(found) => workspace::resolved(cx, found?)?,
        None => workspace::of_project(&snapshot, root.dir())?,
    };
    let dir = workspace::member_dir(&workspace, root.dir());
    Ok((workspace.with_entries(&dir, vec![entry]), dir))
}

/// `node` running `output` with `args`, ending with nomi as
/// [`dies_with_nomi`] has it.
fn node(output: &Path, args: &[OsString]) -> Command {
    let mut node = Command::new("node");
    node.arg(output).args(args);
    dies_with_nomi(&mut node);
    node
}

/// Has the process `command` starts sent SIGTERM when nomi ends first,
/// however nomi ends, SIGKILL included, so that a program finalizes
/// rather than runs on unwatched. Linux sends it once the thread that
/// started the process ends, and nomi starts its programs on the thread
/// that runs the command, which outlives them.
#[cfg(target_os = "linux")]
#[expect(
    unsafe_code,
    reason = "a parent-death signal is set between fork and exec, which only `pre_exec` reaches"
)]
fn dies_with_nomi(command: &mut Command) {
    use std::os::unix::process::CommandExt as _;

    use rustix::process::{Signal, getpid, getppid, set_parent_process_death_signal};

    let nomi = getpid();
    // SAFETY: the closure runs in the child between fork and exec, where
    // only async-signal-safe work is sound: it makes two system calls,
    // allocating nothing and taking no lock, and on failure returns an
    // error built from a code, which allocates nothing either.
    unsafe {
        command.pre_exec(move || {
            set_parent_process_death_signal(Some(Signal::TERM))?;
            // nomi may have ended before the signal was asked for.
            if getppid() == Some(nomi) {
                Ok(())
            } else {
                Err(rustix::io::Errno::SRCH.into())
            }
        });
    }
}

/// Elsewhere a program stays in nomi's process group, so whatever stops
/// the group, as a terminal's Ctrl-C does, stops it too.
#[cfg(not(target_os = "linux"))]
fn dies_with_nomi(_: &mut Command) {}

/// Builds `program`, showing how it went; what was built and the code
/// written, if it was.
fn build(
    cx: &Context<'_, impl Http + Sync>,
    console: &mut dyn Console,
    program: &Program,
) -> Option<(Built, Code)> {
    let mut built = chosen(cx, program)
        .inspect_err(|report| console::report(console, report, Compat::LATEST))
        .ok()?;
    let output = built.output.to_string_lossy();
    let make = Make::build(vec![built.main.clone()], Some(&output), built.platform)
        .expect("a program run is written to an ES module");
    let make = Make {
        ffi: built.ffi.clone(),
        output_name: built.output_name,
        target: Target {
            start: Start::Entry,
            advice: Advice::Omitted,
            ..make.target
        },
        ..make
    };
    let cx = Context {
        dir: &built.dir,
        ..*cx
    };
    let code = make::wrote(&cx, console, &make)?.code?;
    built.output.clone_from(&code.output);
    let output = built.output.to_string_lossy();
    if code.platform == Platform::Node && !make::has_ext(".mjs", &output) {
        let main = built.main.display().to_string();
        let output = relative(&built.output, &built.dir);
        let report = not_a_module(&main, &output.display().to_string());
        console::report(console, &report, Compat::LATEST);
        return None;
    }
    Some((built, code))
}

/// What `program` builds, from `nomi.toml`'s entry where it names one.
fn chosen(cx: &Context<'_, impl Http + Sync>, program: &Program) -> Result<Built, Report> {
    let root = Root::find(cx.vfs, cx.dir).ok_or_else(|| no_outline("to run"))?;
    let output_of =
        |main: &Path, extension| root.dir().join(default_output(root.dir(), main, extension));
    let chosen = match program {
        Program::Module(main) => Built {
            main: main.clone(),
            output: output_of(&cx.dir.join(main), "mjs"),
            output_name: OutputName::Given,
            ffi: None,
            dir: cx.dir.to_owned(),
            platform: Some(Platform::Node),
            args: Vec::new(),
            static_dir: None,
            port: None,
        },
        Program::Entry(chosen) => {
            let entry = entry(cx, &root, chosen.as_deref())?;
            let output = match &entry.output {
                Some(output) => root.dir().join(output),
                None => output_of(&root.dir().join(&entry.main), extension(&entry)),
            };
            let output_name = if entry.output.is_none() {
                OutputName::ByPlatform
            } else {
                OutputName::Given
            };
            Built {
                ffi: entry.ffi.map(|path| FfiModule {
                    path: path.into(),
                    named_in: NamedIn::Entry(entry.main.clone()),
                }),
                main: entry.main.into(),
                output,
                output_name,
                dir: root.dir().to_owned(),
                platform: entry.platform,
                args: entry.args,
                static_dir: entry.static_dir,
                port: entry.port,
            }
        }
    };
    Ok(chosen)
}

/// The extension of the code of `entry` when nothing names its output:
/// a script for the browser, an ES module otherwise, until a build
/// without a platform finds it is a browser program's.
pub(crate) fn extension(entry: &Entry) -> &'static str {
    match entry.platform {
        Some(Platform::Browser) => "js",
        Some(Platform::Node) | None => "mjs",
    }
}

/// Where the program whose `main` is the module at `main`, in the
/// project at `root`, is written when nothing names its output, from the
/// root: by the module's path in the project, so that no two share one,
/// with the extension `extension`.
pub(crate) fn default_output(root: &Path, main: &Path, extension: &str) -> PathBuf {
    let mut name = relative(main, root);
    if !(name.components()).all(|c| matches!(c, Component::Normal(_))) {
        name = main.file_name().unwrap_or(main.as_os_str()).into();
    }
    Path::new("elm-stuff/nomi/run")
        .join(name)
        .with_extension(extension)
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
    use rustix::process::Pid;
    use signal_hook::consts::{SIGINT, SIGTERM};
    use signal_hook::iterator::SignalsInfo;

    // Caught before the child starts, so neither signal can stop nomi
    // alone once it has.
    let mut signals = SignalsInfo::<WithOrigin>::new([SIGINT, SIGTERM])?;
    let handle = signals.handle();
    let mut child = command.spawn()?;
    let pid = Pid::from_child(&child);
    let forwarding = thread::spawn(move || {
        for origin in signals.forever() {
            if let Some(stop) = signalled(&origin).passed_on() {
                let _ = signal(pid, stop);
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
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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

/// Who sent nomi a signal.
#[derive(Clone, Copy, Debug)]
enum Origin {
    /// The terminal, which sends its signals to the programs too.
    Terminal,
    /// A process, which sent it to nomi alone.
    Process,
}

/// A signal nomi was sent, and by whom.
#[derive(Clone, Copy, Debug)]
struct Signalled {
    stop: Stop,
    origin: Origin,
}

impl Signalled {
    /// The signal to pass on to the programs: none where the terminal
    /// sent it, as it reached them already.
    fn passed_on(self) -> Option<Stop> {
        match self.origin {
            Origin::Terminal => None,
            Origin::Process => Some(self.stop),
        }
    }
}

/// How long a program has to stop before it is killed: longer than the
/// runtime's own shutdown deadline, which a server lengthens to its drain
/// and 5 seconds more.
const GRACE: Duration = Duration::from_secs(15);

/// Sends `child` `signal`.
#[cfg(unix)]
fn send(child: &Child, signal: Stop) -> io::Result<()> {
    self::signal(rustix::process::Pid::from_child(child), signal)
}

/// Sends the process `pid` `stop`'s signal.
#[cfg(unix)]
fn signal(pid: rustix::process::Pid, stop: Stop) -> io::Result<()> {
    use rustix::process::{Signal, kill_process};
    let signal = match stop {
        Stop::Interrupt => Signal::INT,
        Stop::Terminate => Signal::TERM,
    };
    Ok(kill_process(pid, signal)?)
}

/// Windows has no signal a program can finish on, so it ends at once.
#[cfg(not(unix))]
fn send(child: &mut Child, _: Stop) -> io::Result<()> {
    child.kill()
}

#[cfg(unix)]
use signal_hook::iterator::exfiltrator::WithOrigin;

/// The SIGINT and SIGTERM nomi is sent while it watches, caught.
#[cfg(unix)]
struct Caught(signal_hook::iterator::SignalsInfo<WithOrigin>);

#[cfg(unix)]
impl Caught {
    fn new() -> io::Result<Caught> {
        use signal_hook::consts::{SIGINT, SIGTERM};
        Ok(Caught(signal_hook::iterator::SignalsInfo::new([
            SIGINT, SIGTERM,
        ])?))
    }

    /// The last signal since the last call.
    fn take(&mut self) -> Option<Signalled> {
        self.0.pending().last().map(|origin| signalled(&origin))
    }

    /// The next signal, once it comes.
    fn wait(&mut self) -> Stop {
        let origin = (self.0.forever().next()).expect("the signals are never closed");
        signalled(&origin).stop
    }
}

/// The signal `origin` tells of, SIGINT or SIGTERM: sent by a process
/// where it names one, and otherwise by the terminal.
#[cfg(unix)]
fn signalled(origin: &signal_hook::low_level::siginfo::Origin) -> Signalled {
    use signal_hook::low_level::siginfo::Cause;
    Signalled {
        stop: if origin.signal == signal_hook::consts::SIGINT {
            Stop::Interrupt
        } else {
            Stop::Terminate
        },
        origin: match origin.cause {
            Cause::Sent(_) => Origin::Process,
            _ => Origin::Terminal,
        },
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

    fn take(&mut self) -> Option<Signalled> {
        None
    }

    fn wait(&mut self) -> Stop {
        loop {
            thread::park();
        }
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

fn no_member_entries() -> Report {
    one_line(
        "NO ENTRY",
        "No member of the workspace has an entry in its nomi.toml to run. Add one, as \
         [[entry]] with main = \"src/Main.elm\".",
    )
}

fn arguments_for_several() -> Report {
    one_line(
        "ARGUMENTS FOR WHICH ENTRY?",
        "The arguments after -- go to one program, but there are several entries to run. \
         Choose one by its main, as in nomi run src/Main.elm, or its member, with -p, or \
         give each its arguments with args in nomi.toml.",
    )
}

fn port_taken(main: &str, port: u16) -> Report {
    one_line(
        "PORT TAKEN",
        &format!(
            "The entry for {main} is served on port {port}, but something else is listening \
             there. Stop it, or give the entry another port in nomi.toml, as port = 8080."
        ),
    )
}

fn no_static_dir(main: &str, dir: &Path) -> Report {
    one_line(
        "NO STATIC DIRECTORY",
        &format!(
            "The entry for {main} serves its static files from {}, but there is no such \
             directory.",
            dir.display()
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
