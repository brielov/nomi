//! The modules a build reads, found as upstream's `Build.hs` crawls them:
//! from the roots, each import names a module in one source directory, a
//! module a direct dependency exposes, or kernel code, and the problems
//! with them are upstream's. Before anything compiles, the modules import
//! no cycle and the roots' names are unique (`checkMidpointAndRoots`).

use std::collections::{BTreeSet, HashMap, HashSet};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use nomi_base::{Compat, FileId, TextRange, ghc, lock};
use nomi_compiler::canonicalize::foreign::default_import_names;
use nomi_compiler::graph;
use nomi_project::{ModuleName, Name};
use nomi_syntax::ast::{self, AstNode};
use nomi_syntax::{
    Parse, Source, SyntaxError, Tool, parse_module, significant_tokens, trimmed_range, validate,
};

use crate::paths::package_src;
use crate::timings::{Phase, time};
use crate::vfs::canonical;
use crate::{Pool, Snapshot};

/// Whose modules these are (`Parse.ProjectType`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Owner {
    /// An application's, which upstream homes in `author/project`.
    Application,
    Package(Name),
}

impl Owner {
    /// The package the modules are homes in: `author/project` for an
    /// application, as upstream names it.
    pub(crate) fn package_name(&self) -> String {
        match self {
            Owner::Application => "author/project".to_owned(),
            Owner::Package(name) => name.to_string(),
        }
    }

    fn source(&self) -> Source {
        match self {
            Owner::Application => Source::Application,
            Owner::Package(name) if name.is_kernel() => Source::Kernel,
            Owner::Package(_) => Source::Package,
        }
    }

    fn is_core(&self) -> bool {
        matches!(self, Owner::Package(name) if *name == Name::core())
    }

    fn is_kernel(&self) -> bool {
        matches!(self, Owner::Package(name) if name.is_kernel())
    }
}

/// The modules the direct dependencies expose, by name (`Details`'s
/// foreigns).
#[derive(Clone, Debug, Default)]
pub struct Foreigns(HashMap<ModuleName, Foreign>);

/// The package a module comes from, and any others exposing a module of
/// the same name (`Details.Foreign`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Foreign {
    pub dep: Name,
    /// The others, which make the name ambiguous.
    pub rivals: Vec<Name>,
}

impl Foreigns {
    /// `gatherForeigns` over each direct dependency with the modules it
    /// exposes. Where packages share a name, upstream's right fold puts
    /// them in reverse order of their names, so the last names the module.
    pub fn new(direct: impl IntoIterator<Item = (Name, Vec<ModuleName>)>) -> Foreigns {
        let mut exposing: HashMap<ModuleName, BTreeSet<Name>> = HashMap::new();
        for (pkg, modules) in direct {
            for module in modules {
                exposing.entry(module).or_default().insert(pkg.clone());
            }
        }
        let foreigns = exposing.into_iter().map(|(module, pkgs)| {
            let mut pkgs = pkgs.into_iter().rev();
            let dep = pkgs.next().expect("a module comes with its package");
            let rivals = pkgs.collect();
            (module, Foreign { dep, rivals })
        });
        Foreigns(foreigns.collect())
    }

    /// The package exposing `module`, and any others that do.
    pub(crate) fn get(&self, module: &ModuleName) -> Option<&Foreign> {
        self.0.get(module)
    }

    /// Every module a direct dependency exposes, in no particular order.
    pub(crate) fn names(&self) -> impl Iterator<Item = &ModuleName> {
        self.0.keys()
    }
}

/// What a crawl knows of the project (`Build.Env`).
#[derive(Clone, Debug)]
pub struct Env {
    compat: Compat,
    root: PathBuf,
    owner: Owner,
    src_dirs: Vec<PathBuf>,
    foreigns: Foreigns,
    /// What paths given to [`Graph::from_paths`] are relative to; empty
    /// for the process's working directory.
    cwd: PathBuf,
}

impl Env {
    /// The project at `root`, whose modules are in `src_dirs`, relative to
    /// it or absolute. Both are resolved as upstream's `canonicalizePath`
    /// resolves them, so a path compares with them by its components.
    pub fn new(
        compat: Compat,
        root: &Path,
        owner: Owner,
        src_dirs: &[PathBuf],
        foreigns: Foreigns,
    ) -> io::Result<Env> {
        let root = canonical(root)?;
        let src_dirs = src_dirs
            .iter()
            .map(|dir| canonical(&root.join(dir)))
            .collect::<io::Result<_>>()?;
        Ok(Env {
            compat,
            root,
            owner,
            src_dirs,
            foreigns,
            cwd: PathBuf::new(),
        })
    }

    /// The environment with `dir` among its source directories too,
    /// resolved as [`Env::new`] resolves them.
    pub(crate) fn with_src_dir(mut self, dir: &Path) -> io::Result<Env> {
        self.src_dirs.push(canonical(&self.root.join(dir))?);
        Ok(self)
    }

    /// Resolves the paths given to [`Graph::from_paths`] against `cwd`,
    /// as elm resolves them against its working directory. Reports still
    /// name them as given.
    pub fn with_cwd(mut self, cwd: &Path) -> Env {
        self.cwd = cwd.to_owned();
        self
    }

    /// What paths given to [`Graph::from_paths`] are relative to.
    pub(crate) fn cwd(&self) -> &Path {
        &self.cwd
    }

    /// Whose modules these are.
    pub(crate) fn owner(&self) -> &Owner {
        &self.owner
    }

    /// The release the project is built as.
    pub(crate) fn compat(&self) -> Compat {
        self.compat
    }

    /// What kind of source the project's modules are.
    pub(crate) fn source(&self) -> Source {
        self.owner.source()
    }

    /// The modules the direct dependencies expose.
    pub(crate) fn foreigns(&self) -> &Foreigns {
        &self.foreigns
    }

    /// The project's source directories, absolute.
    pub fn src_dirs(&self) -> &[PathBuf] {
        &self.src_dirs
    }
}

/// The modules a build reads, and where it starts.
#[derive(Debug)]
pub struct Graph {
    /// What each name the crawl reached led to.
    pub modules: HashMap<ModuleName, Status>,
    pub roots: Vec<Root>,
}

impl Graph {
    /// Each root's name and whether it defines `main`, in order
    /// (`Build.getRootNames`, and `getMain`'s answer).
    pub(crate) fn root_mains(&self) -> Vec<RootMain> {
        let main = |module: &Module| RootMain {
            name: module.name.clone(),
            has_main: module.has_main,
        };
        (self.roots.iter())
            .filter_map(|root| match root {
                Root::Inside(name) => match self.modules.get(name)? {
                    Status::Local(module) => Some(main(module)),
                    Status::Foreign(_)
                    | Status::Kernel
                    | Status::BadImport(_)
                    | Status::BadModule(_) => None,
                },
                Root::Outside(Ok(module)) => Some(main(module)),
                Root::Outside(Err(_)) => None,
            })
            .collect()
    }
}

/// A root of a build, by name, and whether it defines `main`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RootMain {
    pub name: ModuleName,
    pub has_main: bool,
}

/// What a module's name led to (`Build.Status`).
#[derive(Clone, Debug)]
pub enum Status {
    /// A module of the project's own.
    Local(Module),
    /// A module this direct dependency exposes.
    Foreign(Name),
    /// Kernel code, which only `elm/` and `elm-explorations/` packages
    /// have.
    Kernel,
    BadImport(ImportProblem),
    BadModule(BadModule),
}

/// A module of the project's own, parsed.
#[derive(Clone, Debug)]
pub struct Module {
    /// Its source directory joined with its name's path, or the path given
    /// for a root outside them.
    pub path: PathBuf,
    pub file: FileId,
    pub text: Arc<str>,
    /// Its tree, which has no syntax errors.
    pub parse: Parse,
    /// The name in its header; `Main` for a root with none.
    pub name: ModuleName,
    /// What it imports, the default imports first unless it is elm/core's.
    pub imports: Vec<Import>,
    /// Whether it defines `main`.
    pub has_main: bool,
}

/// A module an import names.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Import {
    pub name: ModuleName,
    /// Where the import names it; `None` for a default import.
    pub range: Option<TextRange>,
}

/// Why an import names no one module (`Import.Problem`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ImportProblem {
    /// Neither a source directory nor a dependency has it.
    NotFound,
    /// A source directory has the module, and so does a dependency.
    Ambiguous {
        path: PathBuf,
        dep: Name,
        rivals: Vec<Name>,
    },
    /// Two or more source directories have it; the paths, relative to the
    /// project.
    AmbiguousLocal(Vec<PathBuf>),
    /// Two or more dependencies expose it.
    AmbiguousForeign { dep: Name, rivals: Vec<Name> },
}

/// A module that can't be read as the one its path names.
#[derive(Clone, Debug)]
pub struct BadModule {
    pub path: PathBuf,
    /// The file and its text, unless it couldn't be read.
    pub source: Option<BadSource>,
    /// Why it can't be read.
    pub problem: ModuleProblem,
}

/// The file of a module that can't be read as the one its path names.
#[derive(Clone, Debug)]
pub struct BadSource {
    pub file: FileId,
    pub text: Arc<str>,
    /// The tree read from the text, recovered past any syntax errors,
    /// for the editor (R4), unless the text isn't UTF-8; the build reads
    /// no further than the problem.
    pub parse: Option<Box<Parse>>,
}

impl BadModule {
    /// The imports the recovered tree names, in order.
    pub fn imports(&self) -> Vec<Import> {
        (self.source.as_ref())
            .and_then(|source| source.parse.as_ref())
            .and_then(|parse| ast::Module::cast(parse.syntax()))
            .map(|syntax| written_imports(&syntax).collect())
            .unwrap_or_default()
    }
}

/// Why a module can't be read as the one its path names.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ModuleProblem {
    /// Reading the file failed; a file that isn't UTF-8 is
    /// [`io::ErrorKind::InvalidData`]. Upstream reads bytes, and stops
    /// on others.
    Unreadable(io::ErrorKind),
    /// The file isn't UTF-8: its bytes, which elm's parser reads as they
    /// are, and the source shows with replacement characters.
    Encoding(Vec<u8>),
    /// What nomi's grammar rejects, in order.
    Syntax(Vec<SyntaxError>),
    /// No header, where the path says the name (`ModuleNameUnspecified`).
    NameUnspecified,
    /// The header names another module (`ModuleNameMismatch`).
    NameMismatch {
        actual: ModuleName,
        range: TextRange,
    },
}

/// Where a build from paths starts.
#[derive(Clone, Debug)]
pub enum Root {
    /// A module in a source directory, by the name its path gives.
    Inside(ModuleName),
    /// A file outside them, any name in its header, as given.
    Outside(Result<Module, BadModule>),
}

/// Why the modules can't be built: the part of
/// `Exit.BuildProjectProblem` the crawl finds, each variant upstream's
/// `BP_` one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProjectProblem {
    PathUnknown(PathBuf),
    WithBadExtension(PathBuf),
    WithAmbiguousSrcDir {
        path: PathBuf,
        first: PathBuf,
        second: PathBuf,
    },
    MainPathDuplicate {
        first: PathBuf,
        second: PathBuf,
    },
    RootNameDuplicate {
        name: ModuleName,
        first: PathBuf,
        second: PathBuf,
    },
    RootNameInvalid {
        path: PathBuf,
        src_dir: PathBuf,
        names: Vec<String>,
    },
    /// Modules importing each other, each the one before imports: the
    /// whole component in 0.19.1 and 0.19.2, one shortest cycle through
    /// its first module in 0.19.3.
    Cycle {
        first: ModuleName,
        rest: Vec<ModuleName>,
    },
}

impl Graph {
    /// `fromExposed`: the modules `exposed` names and all they import, as
    /// upstream builds a package.
    pub fn from_exposed(
        snapshot: &Snapshot,
        env: &Env,
        exposed: &[ModuleName],
    ) -> Result<Graph, ProjectProblem> {
        let mut modules = HashMap::new();
        crawl(snapshot, env, &mut modules, exposed.to_vec());
        check_cycles(env.compat, &modules)?;
        let roots = exposed.iter().cloned().map(Root::Inside).collect();
        Ok(Graph { modules, roots })
    }

    /// `fromPaths`: the files at `paths` and all they import, as
    /// `elm make` builds them.
    pub fn from_paths(
        snapshot: &Snapshot,
        env: &Env,
        paths: &[PathBuf],
    ) -> Result<Graph, ProjectProblem> {
        let locations = find_roots(snapshot, env, paths)?;
        let roots = Pool::global().map(locations, |location| match location {
            Location::Inside(name) => Root::Inside(name),
            Location::Outside(path) => {
                // Read where it is, named as given.
                let read = read(snapshot, env, env.cwd.join(&path), None);
                Root::Outside(match read {
                    Ok(module) => Ok(Module { path, ..module }),
                    Err(bad) => Err(BadModule { path, ..bad }),
                })
            }
        });
        let mut wave = Vec::new();
        for root in &roots {
            match root {
                Root::Inside(name) => wave.push(name.clone()),
                Root::Outside(Ok(module)) => {
                    wave.extend(module.imports.iter().map(|i| i.name.clone()));
                }
                Root::Outside(Err(_)) => {}
            }
        }
        let mut modules = HashMap::new();
        crawl(snapshot, env, &mut modules, wave);
        check_cycles(env.compat, &modules)?;
        check_unique_roots(env.compat, &modules, &roots)?;
        Ok(Graph { modules, roots })
    }
}

/// `crawlDeps`: every module `wave` names and every module they import,
/// in waves across the pool.
fn crawl(
    snapshot: &Snapshot,
    env: &Env,
    modules: &mut HashMap<ModuleName, Status>,
    wave: Vec<ModuleName>,
) {
    let mut queued = HashSet::new();
    let mut wave: Vec<_> = wave
        .into_iter()
        .filter(|n| queued.insert(n.clone()))
        .collect();
    while !wave.is_empty() {
        let statuses = Pool::global().map(wave, |name| {
            let status = crawl_module(snapshot, env, &name);
            (name, status)
        });
        let mut imports = Vec::new();
        for (name, status) in statuses {
            if let Status::Local(module) = &status {
                imports.extend(module.imports.iter().map(|i| i.name.clone()));
            }
            modules.insert(name, status);
        }
        wave = imports
            .into_iter()
            .filter(|name| !modules.contains_key(name) && queued.insert(name.clone()))
            .collect();
    }
}

/// `crawlModule`: what `name` leads to.
fn crawl_module(snapshot: &Snapshot, env: &Env, name: &ModuleName) -> Status {
    let paths: Vec<PathBuf> = match module_file(name, "elm") {
        Some(file) => env
            .src_dirs
            .iter()
            .map(|dir| dir.join(&file))
            .filter(|path| snapshot.is_file(path))
            .collect(),
        None => Vec::new(),
    };
    let foreign = env.foreigns.get(name);
    match (paths.as_slice(), foreign) {
        ([path], Some(foreign)) => Status::BadImport(ImportProblem::Ambiguous {
            path: path.clone(),
            dep: foreign.dep.clone(),
            rivals: foreign.rivals.clone(),
        }),
        ([path], None) => match read(snapshot, env, path.clone(), Some(name)) {
            Ok(module) => Status::Local(module),
            Err(bad) => Status::BadModule(bad),
        },
        ([], Some(foreign)) if foreign.rivals.is_empty() => Status::Foreign(foreign.dep.clone()),
        ([], Some(foreign)) => Status::BadImport(ImportProblem::AmbiguousForeign {
            dep: foreign.dep.clone(),
            rivals: foreign.rivals.clone(),
        }),
        ([], None) => {
            let js = module_file(name, "js").map(|file| package_src(&env.root).join(file));
            if name.is_kernel()
                && env.owner.is_kernel()
                && js.is_some_and(|js| snapshot.is_file(&js))
            {
                Status::Kernel
            } else {
                Status::BadImport(ImportProblem::NotFound)
            }
        }
        (paths, _) => Status::BadImport(ImportProblem::AmbiguousLocal(
            paths
                .iter()
                .map(|path| path.strip_prefix(&env.root).unwrap_or(path).to_owned())
                .collect(),
        )),
    }
}

/// `Module.toFilePath name <.> ext`; a name that isn't UTF-8 has none.
fn module_file(name: &ModuleName, ext: &str) -> Option<PathBuf> {
    Some(module_path(name.to_str()?, ext))
}

/// `Module.toFilePath name <.> ext`: the path below a source directory of
/// the module `name`.
pub fn module_path(name: &str, ext: &str) -> PathBuf {
    let mut file: PathBuf = name.split('.').collect();
    file.set_extension(ext);
    file
}

/// What decides how a file's text reads, beside the text.
#[derive(PartialEq, Eq)]
struct ReadAs {
    tool: Tool,
    /// Whether the module is elm/core's, which imports nothing by default.
    core: bool,
    expected: Option<ModuleName>,
}

/// What each path's latest text read as, kept for the next build, as an
/// editor's checks read a project's unchanged files again and again.
struct Read {
    text: Arc<str>,
    read_as: ReadAs,
    read: Result<Module, BadModule>,
}

/// Whether reads are kept, as a long-lived process such as the language
/// server wants and a one-off build doesn't, whose trees go when it's done.
static KEEP: AtomicBool = AtomicBool::new(false);

/// Keeps each file's read for the next build while its text is the same.
pub fn keep_reads() {
    KEEP.store(true, Ordering::Relaxed);
}

impl Read {
    fn kept() -> bool {
        KEEP.load(Ordering::Relaxed)
    }

    fn all() -> &'static Mutex<HashMap<PathBuf, Read>> {
        static ALL: OnceLock<Mutex<HashMap<PathBuf, Read>>> = OnceLock::new();
        ALL.get_or_init(Default::default)
    }

    fn cached(path: &Path, text: &str, read_as: &ReadAs) -> Option<Result<Module, BadModule>> {
        if !Self::kept() {
            return None;
        }
        let all = lock(Self::all());
        let kept = all.get(path)?;
        (*kept.text == *text && kept.read_as == *read_as).then(|| kept.read.clone())
    }

    fn keep(path: PathBuf, text: Arc<str>, read_as: ReadAs, read: &Result<Module, BadModule>) {
        if !Self::kept() {
            return;
        }
        let kept = Read {
            text,
            read_as,
            read: read.clone(),
        };
        lock(Self::all()).insert(path, kept);
    }
}

/// `crawlFile`, or with no name `expected`, `crawlRoot`'s parse of a file
/// outside the source directories.
fn read(
    snapshot: &Snapshot,
    env: &Env,
    path: PathBuf,
    expected: Option<&ModuleName>,
) -> Result<Module, BadModule> {
    time(Phase::Parse, || read_file(snapshot, env, path, expected))
}

fn read_file(
    snapshot: &Snapshot,
    env: &Env,
    path: PathBuf,
    expected: Option<&ModuleName>,
) -> Result<Module, BadModule> {
    let unreadable = |path: &Path, e: io::Error| BadModule {
        path: path.to_owned(),
        source: None,
        problem: ModuleProblem::Unreadable(e.kind()),
    };
    let file = snapshot.file_id(&path).map_err(|e| unreadable(&path, e))?;
    let text = match snapshot.text(file) {
        Ok(text) => text,
        Err(e) if e.kind() == io::ErrorKind::InvalidData => {
            let bytes = snapshot
                .read_bytes(&path)
                .map_err(|e| unreadable(&path, e))?;
            return Err(BadModule {
                path,
                source: Some(BadSource {
                    file,
                    text: Arc::from(ghc::lenient_utf8(&bytes)),
                    parse: None,
                }),
                problem: ModuleProblem::Encoding(bytes),
            });
        }
        Err(e) => return Err(unreadable(&path, e)),
    };
    let read_as = ReadAs {
        tool: Tool::Elm(env.owner.source(), env.compat),
        core: env.owner.is_core(),
        expected: expected.cloned(),
    };
    if let Some(read) = Read::cached(&path, &text, &read_as) {
        // Its file as this snapshot numbers it.
        return match read {
            Ok(module) => Ok(Module { file, ..module }),
            Err(bad) => Err(BadModule {
                source: bad.source.map(|source| BadSource { file, ..source }),
                ..bad
            }),
        };
    }
    let read = read_text(
        env,
        path.clone(),
        expected,
        file,
        text.clone(),
        read_as.tool,
    );
    Read::keep(path, text, read_as, &read);
    read
}

/// What a file's text reads as, where its text decides it all.
fn read_text(
    env: &Env,
    path: PathBuf,
    expected: Option<&ModuleName>,
    file: FileId,
    text: Arc<str>,
    tool: Tool,
) -> Result<Module, BadModule> {
    let parse = parse_module(&text);
    let validated = validate(&parse, tool);
    let bad = |problem| BadModule {
        path: path.clone(),
        source: Some(BadSource {
            file,
            text: text.clone(),
            parse: Some(Box::new(parse.clone())),
        }),
        problem,
    };
    let mut errors = parse.errors().to_vec();
    errors.extend(validated);
    if !errors.is_empty() {
        errors.sort_by_key(|e| e.range.start());
        return Err(bad(ModuleProblem::Syntax(errors)));
    }
    let syntax = ast::Module::cast(parse.syntax()).expect("a module parses to a module");
    let header = syntax
        .module_header()
        .and_then(|header| header.module_name())
        .map(|name| name_of(&name));
    let name = match (expected, header) {
        (Some(_), None) => return Err(bad(ModuleProblem::NameUnspecified)),
        (Some(expected), Some((actual, range))) if actual != *expected => {
            return Err(bad(ModuleProblem::NameMismatch { actual, range }));
        }
        (_, Some((name, _))) => name,
        (None, None) => ModuleName::new("Main"),
    };
    // `Imports.defaults`: what every module but elm/core's imports first.
    let mut imports = Vec::new();
    if !env.owner.is_core() {
        imports.extend(default_import_names().map(|name| Import {
            name: ModuleName::new(name),
            range: None,
        }));
    }
    imports.extend(written_imports(&syntax));
    let has_main = syntax.decls().any(|decl| {
        let ast::Decl::ValueDecl(value) = decl else {
            return false;
        };
        (value.definition().and_then(|d| d.lower_ident_token()))
            .is_some_and(|name| name.text() == "main")
    });
    Ok(Module {
        path,
        file,
        text,
        parse,
        name,
        imports,
        has_main,
    })
}

/// The imports `syntax` writes, each where it names its module.
pub(crate) fn written_imports(syntax: &ast::Module) -> impl Iterator<Item = Import> {
    syntax.imports().filter_map(|import| {
        let (name, range) = name_of(&import.module_name()?);
        Some(Import {
            name,
            range: Some(range),
        })
    })
}

/// A module name as written, without the trivia around it, and where.
fn name_of(name: &ast::ModuleName) -> (ModuleName, TextRange) {
    // Its segments and the dots between them, written joined.
    let mut text = String::new();
    for token in significant_tokens(name.syntax()) {
        text.push_str(token.text());
    }
    (ModuleName::new(&text), trimmed_range(name.syntax()))
}

/// `RootLocation`.
enum Location {
    Inside(ModuleName),
    Outside(PathBuf),
}

/// `findRoots`: each path's location, the first problem in the order the
/// paths come, then `checkRoots`: no file given twice.
fn find_roots(
    snapshot: &Snapshot,
    env: &Env,
    paths: &[PathBuf],
) -> Result<Vec<Location>, ProjectProblem> {
    let mut infos = Vec::with_capacity(paths.len());
    for path in paths {
        infos.push(root_info(snapshot, env, path)?);
    }
    let mut by_file: HashMap<&[u8], Vec<&Path>> = HashMap::new();
    for (given, (absolute, _)) in paths.iter().zip(&infos) {
        by_file
            .entry(absolute.as_os_str().as_encoded_bytes())
            .or_default()
            .push(given);
    }
    // Upstream's map orders the paths as strings, so by their bytes.
    let mut files: Vec<_> = by_file.into_iter().collect();
    files.sort_by(|a, b| a.0.cmp(b.0));
    if let Some((_, given)) = files.into_iter().find(|(_, given)| given.len() > 1) {
        // `fromListWith` puts the later path first.
        let n = given.len();
        return Err(ProjectProblem::MainPathDuplicate {
            first: given[n - 1].to_owned(),
            second: given[n - 2].to_owned(),
        });
    }
    Ok(infos.into_iter().map(|(_, location)| location).collect())
}

/// A source directory a root lies in, the names its path there makes,
/// and whether each is a good one.
struct Inside<'a> {
    src_dir: &'a Path,
    names: Vec<String>,
    good: bool,
}

/// `getRootInfo`: the file's resolved path, and where it lies.
fn root_info(
    snapshot: &Snapshot,
    env: &Env,
    path: &Path,
) -> Result<(PathBuf, Location), ProjectProblem> {
    let unknown = || ProjectProblem::PathUnknown(path.to_owned());
    let at = env.cwd.join(path);
    if !snapshot.is_file(&at) {
        return Err(unknown());
    }
    let absolute = canonical(&at).map_err(|_| unknown())?;
    let (Some(dir), Some(file)) = (absolute.parent(), absolute.file_name()) else {
        return Err(unknown());
    };
    // `splitExtension`: from the last dot.
    let file = file.as_encoded_bytes();
    let dot = file.iter().rposition(|&b| b == b'.');
    let Some(dot) = dot.filter(|&dot| &file[dot..] == b".elm") else {
        return Err(ProjectProblem::WithBadExtension(path.to_owned()));
    };
    let stem = String::from_utf8_lossy(&file[..dot]).into_owned();
    let stem_good = std::str::from_utf8(&file[..dot]).is_ok_and(is_good_name);
    let inside: Vec<Inside<'_>> = (env.src_dirs.iter())
        .filter_map(|src_dir| {
            let rest = dir.strip_prefix(src_dir).ok()?;
            let mut names = Vec::new();
            let mut good = stem_good;
            for segment in rest {
                good &= segment.to_str().is_some_and(is_good_name);
                names.push(segment.to_string_lossy().into_owned());
            }
            names.push(stem.clone());
            Some(Inside {
                src_dir,
                names,
                good,
            })
        })
        .collect();
    let location = match inside.as_slice() {
        [] => Location::Outside(path.to_owned()),
        [
            Inside {
                names, good: true, ..
            },
        ] => {
            let name = names.join(".");
            let file = module_path(&name, "elm");
            let mut matching = (env.src_dirs.iter())
                .map(|src_dir| src_dir.join(&file))
                .filter(|path| snapshot.is_file(path));
            let name = ModuleName::new(&name);
            if let (Some(first), Some(second)) = (matching.next(), matching.next()) {
                return Err(ProjectProblem::RootNameDuplicate {
                    name,
                    first,
                    second,
                });
            }
            Location::Inside(name)
        }
        [
            Inside {
                src_dir,
                names,
                good: false,
            },
        ] => {
            return Err(ProjectProblem::RootNameInvalid {
                path: path.to_owned(),
                src_dir: src_dir.to_path_buf(),
                names: names.clone(),
            });
        }
        [first, second, ..] => {
            return Err(ProjectProblem::WithAmbiguousSrcDir {
                path: path.to_owned(),
                first: first.src_dir.to_path_buf(),
                second: second.src_dir.to_path_buf(),
            });
        }
    };
    Ok((absolute, location))
}

/// `isGoodName`: a capital, then letters, numbers and underscores, as
/// GHC's `Data.Char` classifies them.
pub(crate) fn is_good_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars.next().is_some_and(ghc::is_upper) && chars.all(|c| ghc::is_alpha_num(c) || c == '_')
}

/// `checkForCycles`: the first strongly connected component that holds a
/// cycle, as `Data.Graph` finds them over the modules in upstream's order,
/// each with the edges its imports make to the others crawled.
fn check_cycles(
    compat: Compat,
    modules: &HashMap<ModuleName, Status>,
) -> Result<(), ProjectProblem> {
    let (names, index) = ordered(compat, modules);
    let edges: Vec<Vec<usize>> = (names.iter())
        .map(|name| match &modules[*name] {
            Status::Local(module) => (module.imports.iter())
                .filter_map(|import| index.get(&import.name).copied())
                .collect(),
            Status::Foreign(_) | Status::Kernel | Status::BadImport(_) | Status::BadModule(_) => {
                Vec::new()
            }
        })
        .collect();
    for component in graph::components(&edges) {
        let first = component[0];
        if component.len() == 1 && !edges[first].contains(&first) {
            continue;
        }
        let cycle = match compat {
            Compat::V0_19_1 | Compat::V0_19_2 => component,
            Compat::V0_19_3 => graph::minimal_cycle(&edges, &component),
        };
        return Err(ProjectProblem::Cycle {
            first: names[cycle[0]].clone(),
            rest: cycle[1..].iter().map(|&i| names[i].clone()).collect(),
        });
    }
    Ok(())
}

/// The names of `modules` in the release's order, and each one's place in
/// it.
pub(crate) fn ordered(
    compat: Compat,
    modules: &HashMap<ModuleName, Status>,
) -> (Vec<&ModuleName>, HashMap<&ModuleName, usize>) {
    let mut names: Vec<&ModuleName> = modules.keys().collect();
    names.sort_by(|a, b| a.compare(b, compat));
    let index = names.iter().enumerate().map(|(i, &n)| (n, i)).collect();
    (names, index)
}

/// `checkUniqueRoots`: no two roots outside the source directories have a
/// name, and none has a crawled module's.
fn check_unique_roots(
    compat: Compat,
    modules: &HashMap<ModuleName, Status>,
    roots: &[Root],
) -> Result<(), ProjectProblem> {
    let mut outsides: HashMap<&ModuleName, Vec<&Path>> = HashMap::new();
    for root in roots {
        if let Root::Outside(Ok(module)) = root {
            outsides.entry(&module.name).or_default().push(&module.path);
        }
    }
    let mut outsides: Vec<_> = outsides.into_iter().collect();
    outsides.sort_by(|a, b| a.0.compare(b.0, compat));
    for (name, paths) in &outsides {
        if let [.., second, first] = paths.as_slice() {
            // `fromListWith` puts the later path first.
            return Err(ProjectProblem::RootNameDuplicate {
                name: (*name).clone(),
                first: first.to_path_buf(),
                second: second.to_path_buf(),
            });
        }
    }
    for (name, paths) in &outsides {
        if let Some(Status::Local(module)) = modules.get(*name) {
            return Err(ProjectProblem::RootNameDuplicate {
                name: (*name).clone(),
                first: paths[0].to_path_buf(),
                second: module.path.clone(),
            });
        }
    }
    Ok(())
}
