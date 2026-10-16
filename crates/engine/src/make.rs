//! `elm make` (`Make.hs`, `run`), which `nomi check` is the
//! `--output=/dev/null` of: the project is found, the flags checked, the
//! project and its dependencies loaded, and the modules the paths name, or
//! a package's exposed ones, built; then the documentation is written, or
//! the code generated.

use std::collections::BTreeMap;
use std::fmt::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use nomi_base::diagnostic::doc::{Color, Doc, fill_sep, reflow, simple_note, text, vcat, words};
use nomi_base::{Compat, Platform, Report};
use nomi_codegen_js::ffi::{self, Ffi};
use nomi_codegen_js::platform::ffi_specifier;
use nomi_project::{ModuleName, Name, Outline};
use nomi_registry::Http;

use crate::artifacts::Artifacts;
use crate::check::{
    self, Build, Built, Checked, Goal, Made, Sources, Style, env_of, problems_summary, show,
    show_reports,
};
use crate::console::{Console, Personality};
use crate::graph::{Env, RootMain};
use crate::names::{Building, ModuleInterface, Packages};
use crate::objects::{Dependencies, Memo, PackageBuild};
use crate::packages::Resolve;
use crate::paths::{Cache, Root, relative};
use crate::project::{self, Project, check_source_dirs};
use crate::report::{self, one_line};
use crate::timings::{self, Phase};
use crate::workspace::MemberBuild;
use crate::{Context, Pool, Vfs, javascript};

/// What `elm make` was asked for (`Make.Flags`, with the paths).
#[derive(Clone, Debug)]
pub struct Make {
    /// The modules to build; none for a package's exposed ones.
    pub paths: Vec<PathBuf>,
    /// Whose words the reports are in.
    pub personality: Personality,
    pub style: Style,
    /// What `--debug` and `--optimize` ask for.
    pub kind: Result<javascript::Kind, ClashingFlags>,
    /// What the code is generated for; upstream's is the default.
    pub target: javascript::Target,
    /// Whether the roots' `main`s choose the platform in place of
    /// `target`'s: Node for nomi/core's `Program`.
    pub platform_by_main: bool,
    /// Which packages the dependencies resolve to: nomi's commands, `nomi
    /// test` among them, know more than upstream's.
    pub resolve: Resolve,
    /// The ES module whose functions `Nomi.Ffi` calls.
    pub ffi: Option<FfiModule>,
    /// The project whose workspace's member packages the dependencies
    /// may name, where the project built is none of its members, as the
    /// project generated for a member's tests is.
    pub members_of: Option<PathBuf>,
    /// `--output`, if given.
    pub output: Option<Output>,
    /// Where to write a package's `docs.json`.
    pub docs: Option<PathBuf>,
}

/// An FFI module, and where it was named.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FfiModule {
    /// Its path, from the directory.
    pub path: PathBuf,
    pub named_in: NamedIn,
}

/// Where an FFI module was named, which its reports point to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NamedIn {
    /// `--ffi`.
    Flag,
    /// The entry of `nomi.toml` whose `main` is this.
    Entry(String),
}

/// That `--debug` and `--optimize` were both given.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClashingFlags;

/// The code `--debug` and `--optimize` ask for.
pub fn kind(debug: bool, optimize: bool) -> Result<javascript::Kind, ClashingFlags> {
    match (debug, optimize) {
        (true, true) => Err(ClashingFlags),
        (true, false) => Ok(javascript::Kind::Debug),
        (false, true) => Ok(javascript::Kind::Optimize),
        (false, false) => Ok(javascript::Kind::Dev),
    }
}

/// `Make.Output`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Output {
    /// A JavaScript file.
    Js(PathBuf),
    /// An HTML page that starts the one root.
    Html(PathBuf),
    /// Nothing: the build is checked, and generates no code.
    DevNull,
}

impl Output {
    /// `parseOutput`: `/dev/null` and its Windows spellings, or a file
    /// whose name ends in `.html` or `.js` after more.
    pub fn parse(name: &str) -> Option<Output> {
        if ["/dev/null", "NUL", "$null"].contains(&name) {
            Some(Output::DevNull)
        } else if has_ext(".html", name) {
            Some(Output::Html(name.into()))
        } else if has_ext(".js", name) {
            Some(Output::Js(name.into()))
        } else {
            None
        }
    }
}

/// `hasExt`: whether `path`'s extension is `ext`, with a name before it.
pub fn has_ext(ext: &str, path: &str) -> bool {
    take_extension(path) == ext && path.len() > ext.len()
}

/// `FilePath.takeExtension`: from the last dot of the file's name on,
/// which may be its first character.
pub fn take_extension(path: &str) -> &str {
    let name = path
        .rsplit(['/', std::path::MAIN_SEPARATOR])
        .next()
        .unwrap_or(path);
    name.rfind('.').map_or("", |at| &name[at..])
}

impl Make {
    /// What `nomi check` asks for: `elm make --output=/dev/null`.
    pub fn check(paths: Vec<PathBuf>, style: Style) -> Make {
        Make {
            paths,
            personality: Personality::Nomi,
            style,
            kind: Ok(javascript::Kind::Dev),
            target: javascript::Target::default(),
            platform_by_main: false,
            resolve: Resolve::Nomi,
            ffi: None,
            members_of: None,
            output: Some(Output::DevNull),
            docs: None,
        }
    }

    /// What `nomi build` asks for of `paths`, its other flags unset: code
    /// written to `output`, an ES module where it ends in `.mjs`, for
    /// `platform` or, without one, the platform the roots' `main`s choose;
    /// `None` where `output` names no file code is written to.
    pub fn build(
        paths: Vec<PathBuf>,
        output: Option<&str>,
        platform: Option<Platform>,
    ) -> Option<Make> {
        let module = output.is_some_and(|o| has_ext(".mjs", o));
        let output = match output {
            Some(o) if module => Some(Output::Js(o.into())),
            Some(o) => Some(Output::parse(o)?),
            None => None,
        };
        Some(Make {
            paths,
            personality: Personality::Nomi,
            style: Style::Terminal,
            kind: Ok(javascript::Kind::Dev),
            target: javascript::Target {
                platform: platform.unwrap_or(Platform::Browser),
                format: if module {
                    javascript::Format::EsModule
                } else {
                    javascript::Format::Iife
                },
                dom: javascript::Dom::Native,
                ffi: None,
            },
            platform_by_main: platform.is_none(),
            resolve: Resolve::Nomi,
            ffi: None,
            members_of: None,
            output,
            docs: None,
        })
    }

    /// What `make` reports where there is no project.
    fn no_outline(&self) -> Report {
        match self.personality {
            Personality::Elm => no_outline(),
            Personality::Nomi if self.output == Some(Output::DevNull) => {
                project::no_outline("to check")
            }
            Personality::Nomi => project::no_outline("to build"),
        }
    }

    /// What the build makes before it writes anything.
    fn goal(&self) -> Goal {
        if self.documenting() {
            Goal::Docs
        } else if self.generating() {
            Goal::Optimize
        } else {
            Goal::Check
        }
    }

    /// Whether the build shows elm's terminal style, with its progress.
    fn terminal(&self) -> bool {
        self.personality == Personality::Elm && self.style == Style::Terminal
    }

    /// Whether a package's `docs.json` is asked for.
    fn documenting(&self) -> bool {
        self.docs.is_some() && self.paths.is_empty()
    }

    /// Whether code is to be generated.
    fn generating(&self) -> bool {
        !self.paths.is_empty() && self.output != Some(Output::DevNull)
    }
}

/// Makes what `make` asks for of the project around the directory, and
/// shows how it went. Whether nothing was wrong. With `NOMI_TIMINGS`,
/// where the time went follows on standard error.
pub fn run(cx: &Context<'_, impl Http + Sync>, console: &mut dyn Console, make: &Make) -> bool {
    wrote(cx, console, make).is_some()
}

/// Makes what `make` asks for as [`run`] does; what it wrote, if nothing
/// was wrong.
pub(crate) fn wrote(
    cx: &Context<'_, impl Http + Sync>,
    console: &mut dyn Console,
    make: &Make,
) -> Option<Wrote> {
    let started = Instant::now();
    // The packages' builds are freed before the code is generated, which
    // then takes their graphs rather than copying them.
    let built = build(cx, console, make, make.goal(), &Shared::new(cx.cache, None));
    let wrote = built.and_then(|clean| write(cx.vfs, console, cx.dir, make, clean, None));
    trim(cx.cache);
    if timings::on() {
        console.err(&timings::report(started.elapsed()));
    }
    wrote
}

/// Makes what `make` asks for as [`run`] does, with what the command's
/// other builds share; what it wrote, if nothing was wrong.
pub(crate) fn made(
    cx: &Context<'_, impl Http + Sync>,
    console: &mut dyn Console,
    make: &Make,
    shared: &Shared,
) -> Option<Wrote> {
    let clean = build(cx, console, make, make.goal(), shared)?;
    write(cx.vfs, console, cx.dir, make, clean, shared.root)
}

/// What a build in which nothing was wrong did.
#[derive(Debug)]
pub(crate) struct Wrote {
    /// The modules checked rather than taken from the artifact cache.
    pub ran: Vec<PathBuf>,
    /// The code written, if any was.
    pub code: Option<Code>,
}

/// The code a build wrote.
#[derive(Debug)]
pub(crate) struct Code {
    /// Where it runs.
    pub platform: Platform,
    /// The modules whose `main`s it starts.
    pub mains: Vec<ModuleName>,
}

/// The interfaces of the modules of a project around the directory,
/// built as [`run`] builds `make`'s paths, which it shows as it does;
/// `None` if anything was wrong.
pub fn interfaces(
    cx: &Context<'_, impl Http + Sync>,
    console: &mut dyn Console,
    make: &Make,
) -> Option<Vec<ModuleInterface>> {
    let goal = if make.documenting() {
        Goal::Docs
    } else {
        Goal::Optimize
    };
    let built = build(cx, console, make, goal, &Shared::new(cx.cache, None));
    trim(cx.cache);
    let clean = built?;
    let interfaces = match &clean.built.made {
        Some(Made::Optimized(optimized)) => optimized.interfaces.clone(),
        _ => Vec::new(),
    };
    write(cx.vfs, console, cx.dir, make, clean, None).map(|_| interfaces)
}

/// What the builds of one command share: the artifact cache, each
/// package's build, and where their reports name paths from.
#[derive(Debug)]
pub(crate) struct Shared<'a> {
    pub artifacts: Artifacts,
    pub memo: Mutex<Memo>,
    /// Each member package built, by name and the elm release built for,
    /// once in a command: its workspace resolves once, and a command builds
    /// them for one goal, or a workspace's optimized for all. A package
    /// alone builds for the latest release its range allows, an app for
    /// its own, and code names kernels as its release does.
    pub members: Mutex<BTreeMap<(Name, Compat), Arc<PackageBuild>>>,
    /// The workspace's root, which a workspace's build names every
    /// module's path from, in place of each project's own directory.
    pub root: Option<&'a Path>,
}

impl<'a> Shared<'a> {
    pub(crate) fn new(cache: &Cache, root: Option<&'a Path>) -> Shared<'a> {
        Shared {
            artifacts: Artifacts::new(cache),
            memo: Mutex::new(Memo::new()),
            members: Mutex::new(BTreeMap::new()),
            root,
        }
    }
}

/// Trims the artifact cache as a build ends, as `go build` trims its own:
/// at most once a day, and a trim that fails costs only disk space.
pub(crate) fn trim(cache: &Cache) {
    let _ = Artifacts::new(cache).trim();
}

/// A build in which nothing was wrong, and what writing its output reads.
struct Clean {
    /// The directory of the project's `elm.json`.
    root: PathBuf,
    compat: Compat,
    env: Env,
    /// How code is generated.
    kind: javascript::Kind,
    /// Every package, and the directory of its source.
    packages: Vec<(Name, PathBuf)>,
    dependencies: Dependencies,
    built: Built,
}

/// The project at `root`, loaded, and what it builds with.
pub(crate) struct Loaded {
    pub compat: Compat,
    pub outline: Outline,
    pub sources: Sources,
}

/// Builds `goal` for `make` of the project around the directory, showing
/// what was wrong if anything was.
fn build(
    cx: &Context<'_, impl Http + Sync>,
    console: &mut dyn Console,
    make: &Make,
    goal: Goal,
    shared: &Shared,
) -> Option<Clean> {
    let dir = cx.dir;
    let personality = make.personality;
    let fail = |console: &mut dyn Console, root: &Path, report, compat| {
        show(console, root, &Checked::Project(report), compat, make.style);
        None
    };
    // What fails before the dependencies are ready, which elm's progress
    // says they are.
    let unready = |console: &mut dyn Console, root: &Path, checked, compat| {
        if make.terminal() {
            details_progress(console, 0);
        }
        show(console, root, &checked, compat, make.style);
        None
    };
    let Some(root) = Root::find(cx.vfs, dir) else {
        return fail(console, dir, make.no_outline(), Compat::LATEST);
    };
    let Ok(kind) = make.kind else {
        return fail(console, dir, cannot_optimize_and_debug(), Compat::LATEST);
    };
    let members_of = make.members_of.as_deref();
    let loaded = match load(cx, &root, personality, make.resolve, members_of) {
        Ok(loaded) => loaded,
        Err((report, compat)) => return unready(console, dir, Checked::Project(report), compat),
    };
    let (compat, outline, sources) = (loaded.compat, loaded.outline, loaded.sources);
    let env = match env_of(root.dir(), &outline, compat, sources.foreigns.clone()) {
        Ok(env) => env.with_cwd(dir),
        Err(report) => return unready(console, dir, Checked::Project(report), compat),
    };
    let root = root.dir().to_owned();
    let snapshot = cx.vfs.snapshot();
    let members = match &sources.members {
        None => BTreeMap::new(),
        Some(members) => {
            let build = MemberBuild {
                snapshot: &snapshot,
                compat,
                personality,
                packages: &sources.packages,
                artifacts: Some(&shared.artifacts),
                memo: &shared.memo,
                goal,
            };
            match build.in_command(members, shared) {
                Ok(built) => built.builds,
                Err(checked) => return unready(console, &members.root, checked, compat),
            }
        }
    };
    // The dependencies build as the modules are found and checked, each
    // module once the packages it imports are built.
    let packages = sources.packages.clone();
    let building = Building::new(
        compat,
        sources.packages,
        members,
        Some(&shared.artifacts),
        &shared.memo,
        sources.direct,
    );
    let built = timings::time(Phase::Modules, || {
        let build = Build {
            snapshot: &snapshot,
            env: &env,
            outline: &outline,
            paths: &make.paths,
            personality,
            packages: Packages::Building(&building),
            artifacts: Some(&shared.artifacts),
        };
        build
            .run(goal)
            .expect("nothing revises the files while make builds")
    });
    // What the modules make counts only once the dependencies are built,
    // as their problems come first.
    let dependencies = match building.finish() {
        Ok(dependencies) => dependencies,
        Err(why) => {
            let report = check::packages_problem(&why);
            return unready(console, dir, Checked::Project(report), compat);
        }
    };
    let built_packages = dependencies.packages.len();
    shown(
        console,
        make,
        shared.root.unwrap_or(&root),
        compat,
        &outline,
        built_packages,
        &built,
    )
    .then(|| Clean {
        root,
        compat,
        env,
        kind,
        packages,
        dependencies,
        built,
    })
}

/// Shows how the build of the project at `root` went, in `make`'s style:
/// elm's progress over the dependencies, `dependencies` of them, and the
/// modules, and what was wrong if anything was. Whether nothing was.
fn shown(
    console: &mut dyn Console,
    make: &Make,
    root: &Path,
    compat: Compat,
    outline: &Outline,
    dependencies: usize,
    built: &Built,
) -> bool {
    let terminal = make.terminal();
    if terminal {
        details_progress(console, dependencies);
    }
    // `elm make` without files, in an application or a package exposing
    // nothing, stops before it builds.
    let no_input = make.paths.is_empty()
        && match outline {
            Outline::App(_) => true,
            Outline::Pkg(pkg) => pkg.exposed.modules().next().is_none(),
        };
    if terminal && !no_input {
        build_progress(console, built.compiled, &built.checked);
    }
    if matches!(built.checked, Checked::Fine) {
        return true;
    }
    if terminal {
        show_reports(console, root, &built.checked, compat);
    } else {
        show(console, root, &built.checked, compat, make.style);
    }
    false
}

/// The project at `root`, and what it builds with, every release
/// downloaded, the member packages of `members_of`'s workspace among them
/// where given; or the report on why not, in the words of the release it
/// names, where it names one.
pub(crate) fn load(
    cx: &Context<'_, impl Http + Sync>,
    root: &Root,
    personality: Personality,
    resolve: Resolve,
    members_of: Option<&Path>,
) -> Result<Loaded, (Report, Compat)> {
    let project = timings::time(Phase::Load, || Project::read(cx.vfs, root, personality))
        .map_err(|report| (report, Compat::LATEST))?;
    let compat = project.compat;
    let fail = |report| (report, compat);
    check_source_dirs(root.dir(), &project.outline).map_err(fail)?;
    let sources = check::sources(cx, &project, personality, resolve, members_of).map_err(fail)?;
    Ok(Loaded {
        compat,
        outline: project.outline,
        sources,
    })
}

/// Writes what the clean build made, as `make` asks: the documentation,
/// or the code and, in the terminal style, a diagram of where it went,
/// naming the output from `from` where given. What it wrote, if nothing
/// was wrong.
fn write(
    vfs: &Vfs,
    console: &mut dyn Console,
    dir: &Path,
    make: &Make,
    clean: Clean,
    from: Option<&Path>,
) -> Option<Wrote> {
    let Clean {
        root,
        compat,
        env,
        kind,
        packages,
        dependencies,
        mut built,
    } = clean;
    let ran = std::mem::take(&mut built.ran);
    let fail = |console: &mut dyn Console, report| {
        show(
            console,
            &root,
            &Checked::Project(report),
            compat,
            make.style,
        );
        None
    };
    if let (Some(path), Some(Made::Docs(docs))) = (&make.docs, &built.made) {
        let path = dir.join(path);
        if let Err(e) = vfs.create(&path, docs.as_bytes()) {
            let report = one_line(
                "PROBLEM WRITING DOCS",
                &format!("I could not write {}: {e}", path.display()),
            );
            return fail(console, report);
        }
    }
    let mut diagram = None;
    let mut code = None;
    if make.generating()
        && let Some(Made::Optimized(optimized)) = built.made
    {
        let target = match target(make.output.as_ref(), &optimized.roots) {
            Ok(target) => target,
            Err(report) => return fail(console, report),
        };
        if let Some(target) = target {
            if kind == javascript::Kind::Optimize {
                let uses = javascript::debug_uses(compat, &optimized);
                if !uses.is_empty() {
                    return fail(console, report::debug_remnants(&uses));
                }
            }
            let mut made_for = make.target.clone();
            if make.platform_by_main && javascript::runs_on_node(&optimized) {
                made_for.platform = Platform::Node;
            }
            if let Some(ffi) = &make.ffi {
                match ffi_module(vfs, dir, ffi, &made_for, &target) {
                    Ok(ffi) => made_for.ffi = Some(ffi),
                    Err(report) => return fail(console, report),
                }
            }
            let package = env.owner().package_name();
            let bytes = timings::time(Phase::Generate, || {
                let types = dependencies.into_graph_and_types();
                let js = javascript::program(
                    compat,
                    kind,
                    &made_for,
                    make.ffi.as_ref(),
                    &package,
                    &packages,
                    types,
                    optimized,
                )?;
                Ok(match &target.html {
                    Some(module) => nomi_codegen_js::html::sandwich(&module.to_string(), &js),
                    None => js,
                })
            });
            let bytes = match bytes {
                Ok(bytes) => bytes,
                Err(report) => return fail(console, report),
            };
            let path = dir.join(&target.path);
            if let Err(report) = create(vfs, &path, &bytes) {
                return fail(console, report);
            }
            let shown = from.map_or_else(|| target.path.clone(), |from| relative(&path, from));
            diagram = Some(gen_diagram(&target.names, &shown.to_string_lossy()));
            code = Some(Code {
                platform: made_for.platform,
                mains: target.names,
            });
        }
    }
    // What the build checked is freed while it reports, or not at all
    // when the process ends first.
    Pool::global().drop_later((built.checked, env));
    // elm's success line came with the build's progress; nomi's comes now.
    if !make.terminal() {
        show(console, &root, &Checked::Fine, compat, make.style);
    }
    if let Some(diagram) = diagram
        && make.style == Style::Terminal
    {
        console.out(&format!("\n{diagram}\n"));
    }
    Some(Wrote { ran, code })
}

/// Writes `bytes` to the file at `path`, and the directories it is in.
fn create(vfs: &Vfs, path: &Path, bytes: &[u8]) -> Result<(), Report> {
    let written = timings::time(Phase::Write, || {
        (path.parent().map_or(Ok(()), std::fs::create_dir_all))
            .and_then(|()| vfs.create(path, bytes))
    });
    written.map_err(|e| {
        one_line(
            "PROBLEM WRITING OUTPUT",
            &format!("I could not write {}: {e}", path.display()),
        )
    })
}

/// What elm's terminal style shows of the dependencies' verification
/// (`Reporting.detailsLoop`): each step, then the line cleared and
/// `Dependencies ready!`. elm's steps interleave as its threads do; nomi
/// shows the dependencies found, then each built.
fn details_progress(console: &mut dyn Console, total: usize) {
    let step = |built: usize| format!("Verifying dependencies ({built}/{total})");
    if total > 0 {
        let mut steps = String::new();
        for built in 0..=total {
            steps.push('\r');
            steps.push_str(&step(built));
        }
        console.out(&steps);
    }
    let cleared = " ".repeat(step(total).chars().count());
    console.out(&format!("\r{cleared}\rDependencies ready!\n"));
}

/// What elm's terminal style shows of the build (`Reporting.buildLoop`):
/// `Compiling ...`, a count for each module compiled, then the final
/// message over them.
fn build_progress(console: &mut dyn Console, compiled: usize, checked: &Checked) {
    let mut out = String::from("Compiling ...");
    for done in 1..=compiled {
        let _ = write!(out, "\rCompiling ({done})");
    }
    let message = match checked {
        Checked::Fine => match compiled {
            0 => "Success!".to_owned(),
            1 => "Success! Compiled 1 module.".to_owned(),
            n => format!("Success! Compiled {n} modules."),
        },
        Checked::Modules(modules) => problems_summary(modules.len()),
        Checked::Project(_) => "Detected a problem.".to_owned(),
    };
    let width = 12 + compiled.to_string().len();
    if message.chars().count() < width {
        let _ = write!(out, "\r{}\r{message}\n", " ".repeat(width));
    } else {
        let _ = write!(out, "\r{message}\n");
    }
    console.out(&out);
}

/// `toGenDiagram`: each module given, an arrow to the file they went to.
fn gen_diagram(names: &[ModuleName], output: &str) -> String {
    let Glyphs {
        bar,
        top,
        middle,
        bottom,
    } = GLYPHS;
    let names: Vec<String> = names.iter().map(ToString::to_string).collect();
    let width = 3 + names.iter().map(|n| n.chars().count()).max().unwrap_or(0);
    let line = |name: &str, end: &str| {
        let bars: String = std::iter::repeat_n(bar, width - name.chars().count()).collect();
        format!("    {name} {bars}{end}")
    };
    match names.as_slice() {
        [] => String::new(),
        [name] => line(name, &format!("> {output}\n")),
        [first, rest @ ..] => {
            let mut lines = vec![line(first, &format!("{top}{bar}{bar}> {output}"))];
            for (i, name) in rest.iter().enumerate() {
                let end = if i + 1 == rest.len() { bottom } else { middle };
                lines.push(line(name, &end.to_string()));
            }
            lines.iter().map(|l| format!("{l}\n")).collect()
        }
    }
}

/// The lines [`gen_diagram`] draws with, which are ASCII on Windows.
struct Glyphs {
    bar: char,
    top: char,
    middle: char,
    bottom: char,
}

const GLYPHS: Glyphs = if cfg!(windows) {
    Glyphs {
        bar: '-',
        top: '+',
        middle: '+',
        bottom: '+',
    }
} else {
    Glyphs {
        bar: '─',
        top: '┬',
        middle: '┤',
        bottom: '┘',
    }
};

/// Where `run` writes the code, the module an HTML page starts, and the
/// modules the terminal style says went there.
struct Target {
    path: PathBuf,
    html: Option<ModuleName>,
    names: Vec<ModuleName>,
}

/// What `Make.hs` generates for `output` from `roots`: without
/// `--output`, `index.html` for one `main` and `elm.js` for more;
/// JavaScript only when every root has a `main`; HTML only for one root,
/// which does.
fn target(output: Option<&Output>, roots: &[RootMain]) -> Result<Option<Target>, Report> {
    let named = |main: bool| -> Vec<&ModuleName> {
        (roots.iter())
            .filter(|root| root.has_main == main)
            .map(|root| &root.name)
            .collect()
    };
    let mains = named(true);
    match output {
        Some(Output::DevNull) => Ok(None),
        None => Ok(match mains.as_slice() {
            [] => None,
            [name] => Some(Target {
                path: "index.html".into(),
                html: Some((*name).clone()),
                names: vec![(*name).clone()],
            }),
            _ => Some(Target {
                path: "elm.js".into(),
                html: None,
                names: mains.iter().map(|n| (*n).clone()).collect(),
            }),
        }),
        Some(Output::Js(path)) => match named(false).as_slice() {
            [] => Ok(Some(Target {
                path: path.clone(),
                html: None,
                names: roots.iter().map(|root| root.name.clone()).collect(),
            })),
            names => Err(non_main_files_into_javascript(names)),
        },
        Some(Output::Html(path)) => match roots {
            [
                RootMain {
                    name,
                    has_main: true,
                },
            ] => Ok(Some(Target {
                path: path.clone(),
                html: Some(name.clone()),
                names: vec![name.clone()],
            })),
            [
                RootMain {
                    has_main: false, ..
                },
            ] => Err(no_main()),
            _ => Err(multiple_files_into_html()),
        },
    }
}

/// The FFI module at `path`, from `dir`, as the program made for
/// `made_for` and written to `target` imports it; or why it can't.
fn ffi_module(
    vfs: &Vfs,
    dir: &Path,
    ffi: &FfiModule,
    made_for: &javascript::Target,
    target: &Target,
) -> Result<Ffi, Report> {
    if made_for.platform != Platform::Node {
        return Err(ffi_off_node());
    }
    if made_for.format != javascript::Format::EsModule || target.html.is_some() {
        return Err(ffi_outside_module());
    }
    let module = dir.join(&ffi.path);
    let source = vfs.read(&module).map_err(|e| ffi_unreadable(ffi, &e))?;
    Ok(Ffi {
        specifier: ffi_specifier(&dir.join(&target.path), &module),
        exports: ffi::exports(&source),
    })
}

/// That an FFI module was given for a program built for the browser.
fn ffi_off_node() -> Report {
    Report::doc(
        "FFI NEEDS NODE",
        None,
        reflow("An FFI module was given, but this program is built for the browser."),
        [reflow(
            "Nomi.Ffi calls JavaScript on Node. Build with --platform=node, or give `main` \
             the type `Nomi.Program`, which builds for Node by itself.",
        )],
    )
}

/// That `ffi` could not be read.
fn ffi_unreadable(ffi: &FfiModule, error: &std::io::Error) -> Report {
    let path = ffi.path.display();
    if error.kind() != std::io::ErrorKind::NotFound {
        return one_line(
            "PROBLEM READING FFI MODULE",
            &format!("I could not read {path}: {error}"),
        );
    }
    let (named, from) = match &ffi.named_in {
        NamedIn::Flag => (
            format!("The FFI module given with --ffi, {path}, does not exist."),
            "from the directory nomi runs in",
        ),
        NamedIn::Entry(main) => (
            format!("The FFI module nomi.toml gives the entry {main}, {path}, does not exist."),
            "from the directory of nomi.toml",
        ),
    };
    Report::doc(
        "FFI MODULE NOT FOUND",
        None,
        reflow(&named),
        [reflow(&format!(
            "Give the path of the ES module that exports the functions Nomi.Ffi.task calls, \
             {from}."
        ))],
    )
}

/// That an FFI module was given for output that isn't an ES module.
fn ffi_outside_module() -> Report {
    Report::doc(
        "FFI NEEDS AN ES MODULE",
        None,
        reflow("An FFI module was given, but the output is not an ES module, which can import it."),
        [reflow(
            "Write the program to a file ending in .mjs, as with --output=dist/main.mjs.",
        )],
    )
}

/// `MakeMultipleFilesIntoHtml`.
fn multiple_files_into_html() -> Report {
    let flagged = |before: &'static str, flag: &'static str, after: &'static str| {
        let flag = text(flag).color(Color::DullYellow);
        fill_sep(words(before).chain([flag]).chain(words(after)))
    };
    Report::new(
        "TOO MANY FILES",
        None,
        "When producing an HTML file, I can only handle one file.",
        [
            flagged(
                "Switch to",
                "--output=/dev/null",
                "if you just want to get compile errors. This skips the code gen phase, so it \
                 can be a bit faster than other options sometimes.",
            ),
            flagged(
                "Switch to",
                "--output=elm.js",
                "if you want multiple `main` values available in a single JavaScript file. \
                 Then you can make your own customized HTML file that embeds multiple Elm \
                 nodes. The generated JavaScript also shares dependencies between modules, so \
                 it should be smaller than compiling each module separately.",
            ),
        ],
    )
}

/// The few lines a `main` can be, which the `NO MAIN` reports show.
fn brief_main() -> Doc {
    vcat([
        fill_sep([text("import").color(Color::Cyan), text("Html")]),
        text(""),
        fill_sep([text("main").color(Color::Green), text("=")]),
        fill_sep([
            text("Html").color(Color::Cyan).append(text(".text")),
            text("\"Hello!\"").color(Color::DullYellow),
        ])
        .indent(2),
    ])
}

/// `MakeNoMain`.
fn no_main() -> Report {
    Report::new(
        "NO MAIN",
        None,
        "When producing an HTML file, I require that the given file has a `main` value. That \
         way I have something to show on screen!",
        [
            reflow(
                "Try adding a `main` value to your file? Or if you just want to verify that this \
                 module compiles, switch to --output=/dev/null to skip the code gen phase \
                 altogether.",
            ),
            simple_note("Adding a `main` value can be as brief as adding something like this:"),
            brief_main(),
            reflow(
                "From there I can create an HTML file that says \"Hello!\" on screen. I \
                 recommend looking through https://guide.elm-lang.org for more guidance on how \
                 to fill in the `main` value.",
            ),
        ],
    )
}

/// `MakeNonMainFilesIntoJavaScript`.
fn non_main_files_into_javascript(names: &[&ModuleName]) -> Report {
    let worker = reflow(
        "Or use https://package.elm-lang.org/packages/elm/core/latest/Platform#worker to make a \
         `main` with no user interface.",
    );
    let note = simple_note("Adding a `main` value can be as brief as adding something like this:");
    match names {
        [name] => Report::new(
            "NO MAIN",
            None,
            &format!(
                "When producing a JS file, I require that the given file has a `main` value. \
                 That way Elm.{name}.init() is definitely defined in the resulting file!"
            ),
            [
                reflow(
                    "Try adding a `main` value to your file? Or if you just want to verify that \
                     this module compiles, switch to --output=/dev/null to skip the code gen \
                     phase altogether.",
                ),
                note,
                brief_main(),
                worker,
            ],
        ),
        names => Report::new(
            "NO MAIN",
            None,
            &format!(
                "When producing a JS file, I require that given files all have `main` values. \
                 That way functions like Elm.{}.init() are definitely defined in the resulting \
                 file. I am missing `main` values in:",
                names[0]
            ),
            [
                vcat(names.iter().map(|n| text(n.to_string())))
                    .color(Color::Red)
                    .indent(4),
                reflow(
                    "Try adding a `main` value to them? Or if you just want to verify that these \
                     modules compile, switch to --output=/dev/null to skip the code gen phase \
                     altogether.",
                ),
                note,
                brief_main(),
                worker,
            ],
        ),
    }
}

/// `MakeNoOutline`.
fn no_outline() -> Report {
    Report::new(
        "NO elm.json FILE",
        None,
        "It looks like you are starting a new Elm project. Very exciting! Try running:",
        [
            text("elm init").color(Color::Green).indent(4),
            reflow("It will help you get set up. It is really simple!"),
        ],
    )
}

/// `MakeCannotOptimizeAndDebug`.
pub(crate) fn cannot_optimize_and_debug() -> Report {
    let flags = [
        text("--optimize").color(Color::Red),
        text("and"),
        text("--debug").color(Color::Red),
    ];
    let first = words("I cannot compile with")
        .chain(flags)
        .chain(words("at the same time."));
    Report::doc(
        "CLASHING FLAGS",
        None,
        fill_sep(first),
        [reflow(
            "I need to take away information to optimize things, and I need to add \
             information to add the debugger. It is impossible to do both at once though! \
             Pick just one of those flags and it should work!",
        )],
    )
}
