//! `nomi run`: an entry of the project's `nomi.toml`, or a module named
//! on the command line, built for Node as a self-starting ES module and
//! run under `node`, which the run stands in for: the arguments are the
//! program's, the signals nomi is sent reach it, and nomi exits as it
//! does. With `--watch` it is built and run again as the project changes.
//! A browser entry is built and served until nomi is stopped. Without an
//! entry named, every entry of the project, the member or the workspace
//! runs, each under its member's name.

mod many;

use std::ffi::OsString;
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};
use std::process::{Child, Command, ExitStatus};
use std::thread;
use std::time::{Duration, Instant};

use nomi_base::{Compat, Platform, Report};
use nomi_project::workspace::Workspace;
use nomi_project::{Entry, Manifest, ModuleName, report};
use nomi_registry::Http;

use crate::artifacts::Hash;
use crate::console::{self, Console, Personality};
use crate::javascript::Target;
use crate::make::{self, Code, FfiModule, Make, NamedIn};
use crate::paths::{NOMI_TOML, Root, relative};
use crate::project::{Project, no_outline};
use crate::report::one_line;
use crate::scope::{self, Scope};
use crate::serve::{Server, Site};
use crate::watch::{Change, POLL, Snapshot, message};
use crate::{Context, workspace};

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
    /// Whether `output`'s extension is the platform's, as nothing names
    /// it.
    output_by_platform: bool,
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
fn by_scope<H: Http>(
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
fn entries(cx: &Context<'_, impl Http>, member: Option<&str>) -> Result<Entries, Report> {
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
    cx: &Context<'_, impl Http>,
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
        Scope::Named(workspace, dirs) => (workspace, dirs),
        Scope::Every(workspace) => {
            let dirs = (workspace.members().iter()).map(|m| m.dir.clone());
            let dirs = dirs.collect();
            (workspace, dirs)
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
    let _server = match serve_built(console, built, code) {
        Ok(server) => server,
        Err(report) => {
            console::report(console, &report, Compat::LATEST);
            return 1;
        }
    };
    caught.wait().code()
}

/// Starts the dev server of the browser program `built` wrote as `code`,
/// saying where.
fn serve_built(console: &mut dyn Console, built: &Built, code: &Code) -> Result<Server, Report> {
    let main = built.main.to_string_lossy();
    let server = serve(Served {
        main: &main,
        output: &built.output,
        module: code.mains.first().expect("an entry has one main"),
        static_dir: built.static_dir.as_ref().map(|dir| built.dir.join(dir)),
        port: built.port.unwrap_or(FIRST_PORT),
        from: &built.dir,
        dir: &built.dir,
    })?;
    console.err(&serving(&main, &server));
    Ok(server)
}

/// The port the first browser entry without one is served on, and the
/// next the next.
const FIRST_PORT: u16 = 8000;

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
    port: u16,
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
    Server::start(site, served.port).map_err(|e| match e.kind() {
        io::ErrorKind::AddrInUse => port_taken(served.main, served.port),
        _ => one_line(
            "PROBLEM SERVING",
            &format!(
                "I could not serve {} on port {}: {e}",
                served.main, served.port
            ),
        ),
    })
}

/// What nomi says once `server` serves the entry it names `name`.
fn serving(name: &str, server: &Server) -> String {
    format!("Serving {name} at http://localhost:{}\n", server.port())
}

/// Builds and runs `program` as [`run`] does, then again as the project
/// changes, as [`watch_one`] does; or, where no entry is named and there
/// are several, runs them all and again as the workspace changes, as
/// `many` does. It runs until nomi is sent SIGINT or SIGTERM; the code to
/// exit with is then the shell's for that signal, 130 or 143, or 1 if the
/// watch couldn't start.
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
        |console, workspace, dirs| many::watch(cx, console, member, workspace, dirs),
        |cx, console| watch_one(cx, console, program, args),
    )
}

/// Builds and runs `program` as [`one`] does, then again each time the
/// project's source directories, `elm.json`, `nomi.toml` or the entry's
/// FFI module change, stopping the program first. A failed build leaves
/// it stopped until the next change, and one that ends by itself is
/// started again on the next change. A browser program is served, its
/// page reloaded when its code changes or its static files do, which
/// needs no build; a failed build leaves it on its old code. It runs
/// until nomi is sent SIGINT or SIGTERM, which stops the program as
/// [`run`] passes signals on.
fn watch_one(
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
    let mut live = Live::default();
    let mut rebuild = true;
    loop {
        if rebuild {
            let built = build(cx, console, program);
            if let Err(report) = live.replace(console, built, args) {
                console::report(console, &report, Compat::LATEST);
                return 1;
            }
            let statics = (live.page.as_ref()).and_then(|page| page.static_dir.clone());
            snapshot.watch(watched(cx, &root, program), statics.into_iter().collect());
            console.err("Watching for changes...\n");
        }
        while queue.is_empty() {
            if let Some((signal, sent)) = caught.take() {
                if let Some(mut child) = live.child {
                    let _ = stop(console, &mut child, sent.then_some(signal));
                }
                return signal.code();
            }
            live.reap(console);
            queue.extend(snapshot.changes());
            if queue.is_empty() {
                thread::sleep(POLL);
            }
        }
        // Changes made together join the build they start.
        thread::sleep(POLL);
        queue.extend(snapshot.changes());
        rebuild = live.changed(console, &queue);
        queue.clear();
        cx.vfs.disk_changed();
    }
}

/// What runs while [`watch_one`] watches: a Node program, or a page.
#[derive(Default)]
struct Live {
    child: Option<Child>,
    page: Option<Page>,
    /// The entry's `main`, which nomi's lines name it by.
    main: String,
}

/// A browser program served while [`watch_one`] watches.
struct Page {
    server: Server,
    /// Its code, absolute.
    output: PathBuf,
    /// The hash of its code when last built.
    code: Hash,
    /// The directory of its static files, absolute.
    static_dir: Option<PathBuf>,
}

impl Page {
    /// Whether a change to `path` reloads the page without a build.
    fn serves(&self, path: &Path) -> bool {
        (self.static_dir.as_ref()).is_some_and(|dir| path.starts_with(dir))
    }
}

impl Live {
    /// Stops what runs and starts what `built` built, if anything: a Node
    /// program, with `args`, or a page, served or, if served already,
    /// reloaded where its code changed; a failed build leaves a page as
    /// it is. Why a page couldn't be served, if it couldn't.
    fn replace(
        &mut self,
        console: &mut dyn Console,
        built: Option<(Built, Code)>,
        args: &[OsString],
    ) -> Result<(), Report> {
        let old = self.child.take();
        let Some((built, code)) = built else {
            if let Some(mut old) = old {
                console.err(&format!("Stopping {}, as its build failed\n", self.main));
                let _ = stop(console, &mut old, Some(Stop::Terminate));
            }
            return Ok(());
        };
        self.main = built.main.display().to_string();
        let verb = if let Some(mut old) = old {
            let _ = stop(console, &mut old, Some(Stop::Terminate));
            "Restarting"
        } else {
            "Starting"
        };
        if code.platform == Platform::Node {
            console.err(&format!("{verb} {}\n", self.main));
            self.page = None;
            self.child = (built.node(args).spawn())
                .inspect_err(|e| console::report(console, &no_node(e), Compat::LATEST))
                .ok();
            return Ok(());
        }
        let hash = code_hash(&built.output);
        match &mut self.page {
            Some(page) => {
                if page.code != hash {
                    console.err(&format!("Reloading {}\n", self.main));
                    page.code = hash;
                    page.server.reload();
                }
            }
            None => {
                self.page = Some(Page {
                    server: serve_built(console, &built, &code)?,
                    static_dir: (built.static_dir.as_ref()).map(|dir| built.dir.join(dir)),
                    output: built.output,
                    code: hash,
                });
            }
        }
        Ok(())
    }

    /// Says so once the program has ended by itself.
    fn reap(&mut self, console: &mut dyn Console) {
        if let Some(status) = (self.child.as_mut()).and_then(|c| c.try_wait().ok().flatten()) {
            let code = exit_code(status);
            console.err(&format!("{} exited with code {code}\n", self.main));
            self.child = None;
        }
    }

    /// Says what `queue` changed, reloading the page where its static
    /// files did; and whether it needs a build: not where it changed only
    /// those, or only the page's code, which a build wrote.
    fn changed(&self, console: &mut dyn Console, queue: &[Change]) -> bool {
        let Some(page) = &self.page else {
            console.err(&format!("{}\n", message(queue)));
            return true;
        };
        let queue: Vec<Change> = (queue.iter())
            .filter(|change| change.path() != page.output)
            .cloned()
            .collect();
        if queue.is_empty() {
            return false;
        }
        console.err(&format!("{}\n", message(&queue)));
        if queue.iter().any(|change| page.serves(change.path())) {
            console.err(&format!("Reloading {}\n", self.main));
            page.server.reload();
        }
        queue.iter().any(|change| !page.serves(change.path()))
    }
}

/// The hash of the code at `output`, as a build left it.
fn code_hash(output: &Path) -> Hash {
    Hash::of(&fs::read(output).unwrap_or_default())
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
        paths.extend(built.ffi.map(|ffi| built.dir.join(ffi.path)));
    }
    paths
}

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
        output_by_platform: built.output_by_platform,
        target: Target {
            quiet: true,
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
            output_by_platform: false,
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
            let output_by_platform = entry.output.is_none();
            Built {
                ffi: entry.ffi.map(|path| FfiModule {
                    path: path.into(),
                    named_in: NamedIn::Entry(entry.main.clone()),
                }),
                main: entry.main.into(),
                output,
                output_by_platform,
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
        Some((
            stop_by(origin.signal),
            matches!(origin.cause, Cause::Sent(_)),
        ))
    }

    /// The next signal, once it comes.
    fn wait(&mut self) -> Stop {
        let origin = (self.0.forever().next()).expect("the signals are never closed");
        stop_by(origin.signal)
    }
}

/// How the signal numbered `signal`, SIGINT or SIGTERM, stops a program.
#[cfg(unix)]
fn stop_by(signal: std::ffi::c_int) -> Stop {
    if signal == signal_hook::consts::SIGINT {
        Stop::Interrupt
    } else {
        Stop::Terminate
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
