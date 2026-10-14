//! What the editor navigates by: every module of the project
//! that owns a file, canonicalized as far as it goes, with what each name
//! it writes refers to, and where the packages it uses keep their
//! sources. Read from the package store alone, never the network: an
//! application's releases are the ones its `elm.json` pins, and a
//! package's are the newest the store holds in each range.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use nomi_base::{Compat, TextSize, lock};
use nomi_compiler::canonicalize::module::{Canonical, canonicalize_with_references, interface};
use nomi_compiler::canonicalize::references::Reference;
use nomi_compiler::hir;
use nomi_compiler::interface::Interface;
use nomi_compiler::types::{self, ExprTypes, Inferred};
use nomi_project::{Constraint, ModuleName, Name, Outline};
use nomi_registry::Store;
use nomi_syntax::ast::{self, AstNode};
use nomi_syntax::{GreenNode, Parse, SyntaxNode};

use crate::check::env_of;
use crate::graph::{Env, Foreigns, Graph, Module, Root, Status, written_imports};
use crate::names::{self, PackageInterfaces};
use crate::paths::ELM_JSON;
use crate::project::package_outline;
use crate::vfs::{canonical, canonical_or_given};
use crate::{Snapshot, discover};

pub use crate::objects::Memo;

/// A project's modules and packages, for navigation.
#[derive(Debug)]
pub struct Index {
    pub compat: Compat,
    /// The package the project's own modules are homes in;
    /// `author/project` for an application, as upstream names it.
    pub package: String,
    /// Each module of the project's own the index could read.
    pub modules: Vec<Arc<Indexed>>,
    /// Each dependency's source directory, by package name.
    pub packages: HashMap<String, PathBuf>,
    /// The modules each direct dependency the store holds exposes, by
    /// package name.
    pub exposed: HashMap<String, Vec<String>>,
    /// The interface of every module the project's modules may import:
    /// the direct dependencies' exposed ones and the project's own.
    pub interfaces: HashMap<String, Interface>,
    /// What the editor read from the dependencies' modules so far, which
    /// the next index of the same packages keeps.
    pub package_files: Arc<PackageFiles>,
}

/// What the editor reads from dependencies' modules, which never change:
/// each one's tree, parsed the first time it's asked for, and the hint it
/// shows for each of its declarations, made the first time.
#[derive(Debug, Default)]
pub struct PackageFiles {
    trees: Mutex<HashMap<PathBuf, GreenNode>>,
    hints: Mutex<HashMap<(PathBuf, TextSize), Option<String>>>,
}

impl PackageFiles {
    /// The tree of the dependency's module at `path`.
    pub fn tree(&self, path: &Path) -> Option<SyntaxNode> {
        if let Some(green) = lock(&self.trees).get(path) {
            return Some(SyntaxNode::new_root(green.clone()));
        }
        let green = nomi_syntax::parse_module(&fs::read_to_string(path).ok()?)
            .green()
            .clone();
        lock(&self.trees).insert(path.to_owned(), green.clone());
        Some(SyntaxNode::new_root(green))
    }

    /// The hint for the declaration named at `at` in the dependency's
    /// module at `path`, as `make` makes it.
    pub fn hint(
        &self,
        path: &Path,
        at: TextSize,
        make: impl FnOnce() -> Option<String>,
    ) -> Option<String> {
        let key = (path.to_owned(), at);
        if let Some(hint) = lock(&self.hints).get(&key) {
            return hint.clone();
        }
        let hint = make();
        lock(&self.hints).insert(key, hint.clone());
        hint
    }
}

/// `ElmPackageCache`'s modules: for each module, the packages the store
/// holds whose newest release exposes it.
#[derive(Debug, Default)]
pub struct PackageModules(BTreeMap<String, Vec<String>>);

impl PackageModules {
    /// The modules of the packages in `store`, as their `elm.json`s list
    /// them.
    pub fn read(store: &Store) -> PackageModules {
        let mut modules: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for (package, sources) in store.newest().unwrap_or_default() {
            let Ok(text) = fs::read(sources.join(ELM_JSON)) else {
                continue;
            };
            if let Ok((Outline::Pkg(outline), _)) = Outline::detect(&text) {
                for module in outline.exposed.modules() {
                    modules
                        .entry(module.to_string())
                        .or_default()
                        .push(package.to_string());
                }
            }
        }
        PackageModules(modules)
    }

    /// The packages that expose `module`.
    pub fn exposing(&self, module: &str) -> &[String] {
        self.0.get(module).map_or(&[], Vec::as_slice)
    }
}

/// One of the project's own modules.
#[derive(Clone, Debug)]
pub struct Indexed {
    /// Its file, as the index read it.
    pub path: PathBuf,
    pub name: String,
    pub text: Arc<str>,
    /// Whether its text parses without errors: a module that doesn't is
    /// indexed as far as its recovered tree goes.
    pub parses: bool,
    /// Its tree, which [`Indexed::root`] makes a cursor into; shareable
    /// across threads as a cursor isn't.
    pub green: GreenNode,
    pub hir: hir::Module,
    /// What each name it writes refers to.
    pub references: Vec<Reference>,
    /// The module canonicalized, where it canonicalizes, for hover to
    /// infer its expressions' types.
    pub canonical: Option<Canonical>,
    /// Its types, inferred the first time they're asked for.
    pub typed: OnceLock<(Inferred, ExprTypes)>,
}

impl Indexed {
    /// Its tree.
    pub fn root(&self) -> SyntaxNode {
        SyntaxNode::new_root(self.green.clone())
    }

    /// Its declarations' types and its expressions', inferred once.
    pub fn types(&self, compat: Compat) -> Option<&(Inferred, ExprTypes)> {
        let canonical = self.canonical.as_ref()?;
        Some(
            self.typed
                .get_or_init(|| types::infer_types(compat, canonical)),
        )
    }
}

impl Index {
    /// The module at `path`, which the index read as given.
    pub fn module(&self, path: &Path) -> Option<&Indexed> {
        // Mostly the path as indexed; resolving each touches the disk.
        if let Some(module) = self.modules.iter().find(|m| m.path == path) {
            return Some(module);
        }
        let path = canonical_or_given(path);
        (self.modules.iter())
            .find(|m| canonical_or_given(&m.path) == path)
            .map(|m| &**m)
    }

    /// Whether `path` is in a dependency's sources.
    pub fn is_package_file(&self, path: &Path) -> bool {
        self.packages.values().any(|dir| path.starts_with(dir))
    }

    /// The project's module named `name`.
    pub fn named(&self, name: &str) -> Option<&Indexed> {
        self.modules.iter().find(|m| m.name == name).map(|m| &**m)
    }
}

/// The `elm.json` of the project that owns `file`: the nearest above it.
pub fn manifest(snapshot: &Snapshot, file: &Path) -> Option<PathBuf> {
    file.ancestors()
        .skip(1)
        .map(|dir| dir.join(ELM_JSON))
        .find(|manifest| snapshot.is_file(manifest))
}

/// A project as the editor sees it: its outline, and what the store holds
/// of its dependencies.
pub(crate) struct Setup {
    /// The directory of its `elm.json`.
    pub root: PathBuf,
    pub outline: Outline,
    pub compat: Compat,
    /// Each release the store holds, and the directory of its source.
    pub sources: Vec<(Name, PathBuf)>,
    pub direct: Vec<Name>,
    /// Whether the store holds every dependency.
    pub complete: bool,
}

impl Setup {
    /// The project whose `elm.json` is `manifest`; none where it isn't
    /// readable.
    pub(crate) fn read(snapshot: &Snapshot, store: &Store, manifest: &Path) -> Option<Setup> {
        let root = manifest.parent()?.to_owned();
        let (outline, compat) = Outline::detect(&snapshot.read_bytes(manifest).ok()?).ok()?;
        let (sources, direct) = releases(store, &outline, compat);
        let complete = match &outline {
            Outline::App(app) => sources.len() == app.direct.len() + app.indirect.len(),
            Outline::Pkg(pkg) => {
                (pkg.deps.keys()).all(|name| sources.iter().any(|(n, _)| n == name))
            }
        };
        Some(Setup {
            root,
            outline,
            compat,
            sources,
            direct,
            complete,
        })
    }

    /// The project as elm-test and elm-language-server see it: with its
    /// test dependencies.
    fn with_tests(mut self, store: &Store) -> Setup {
        let direct: Vec<Name> = match &self.outline {
            Outline::App(app) => {
                for (name, &version) in app.test_direct.iter().chain(&app.test_indirect) {
                    if !self.sources.iter().any(|(n, _)| n == name)
                        && let Ok(Some(dir)) = store.get(name, version)
                    {
                        self.sources.push((name.clone(), dir));
                    }
                }
                app.test_direct.keys().cloned().collect()
            }
            Outline::Pkg(pkg) => {
                let pending = pkg.test_deps.iter().map(|(n, c)| (n.clone(), *c));
                resolve(store, self.compat, pending.collect(), &mut self.sources);
                pkg.test_deps.keys().cloned().collect()
            }
        };
        for name in direct {
            if !self.direct.contains(&name) {
                self.direct.push(name);
            }
        }
        self
    }

    /// The modules each direct dependency the store holds exposes.
    pub(crate) fn exposing(&self) -> Vec<(Name, Vec<ModuleName>)> {
        (self.direct.iter())
            .filter_map(|name| {
                let (_, dir) = self.sources.iter().find(|(n, _)| n == name)?;
                let pkg = package_outline(dir, self.compat).ok()??;
                Some((name.clone(), pkg.exposed.modules().cloned().collect()))
            })
            .collect()
    }

    /// The project's build environment, with the modules each direct
    /// dependency exposes, as `exposing` lists them.
    pub(crate) fn env(&self, exposing: &[(Name, Vec<ModuleName>)]) -> Option<Env> {
        env_of(
            &self.root,
            &self.outline,
            self.compat,
            Foreigns::new(exposing.iter().cloned()),
        )
        .ok()
    }
}

/// The newest release in the store of each package `pending` names, in
/// its range, and of each it depends on, added to `sources`.
fn resolve(
    store: &Store,
    compat: Compat,
    mut pending: Vec<(Name, Constraint)>,
    sources: &mut Vec<(Name, PathBuf)>,
) {
    while let Some((name, constraint)) = pending.pop() {
        if sources.iter().any(|(n, _)| *n == name) {
            continue;
        }
        let versions = store.versions(&name).unwrap_or_default();
        let Some(dir) = (versions.into_iter().rev())
            .filter(|&v| constraint.satisfies(v))
            .find_map(|v| store.get(&name, v).ok().flatten())
        else {
            continue;
        };
        if let Ok(Some(dep)) = package_outline(&dir, compat) {
            pending.extend(dep.deps.iter().map(|(n, c)| (n.clone(), *c)));
        }
        sources.push((name, dir));
    }
}

/// The `.elm` files under the source directories of `env`, from the disk.
pub(crate) fn elm_files(env: &Env) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    for dir in env.src_dirs() {
        let found = discover(dir).unwrap_or_default();
        paths.extend(
            found
                .into_iter()
                .filter(|p| p.extension().is_some_and(|e| e == "elm")),
        );
    }
    paths
}

/// The index of the project that owns `file`; none where there is no
/// project, its `elm.json` isn't readable, or its modules import each
/// other in a cycle.
pub fn index(snapshot: &Snapshot, store: &Store, file: &Path, memo: &mut Memo) -> Option<Index> {
    index_from(snapshot, store, file, memo, None)
}

/// [`index`], reusing from `previous`, an index of the same project, each
/// module whose text is the same and whose project imports have the same
/// interfaces, as an editor's next index mostly is, and what it read from
/// the dependencies' modules.
pub fn index_from(
    snapshot: &Snapshot,
    store: &Store,
    file: &Path,
    memo: &mut Memo,
    previous: Option<&Index>,
) -> Option<Index> {
    let manifest = manifest(snapshot, file)?;
    let setup = Setup::read(snapshot, store, &manifest)?.with_tests(store);
    let exposing = setup.exposing();
    let mut env = setup.env(&exposing)?;
    let tests = setup.root.join("tests");
    // Directories are the disk's; the Vfs keeps files.
    let listed = canonical(&tests).is_ok_and(|t| env.src_dirs().contains(&t));
    if tests.is_dir() && !listed {
        env = env.with_src_dir(&tests).ok()?;
    }
    let Setup {
        compat,
        sources,
        direct,
        ..
    } = setup;
    // A package nomi can't canonicalize leaves the project's own modules
    // to navigate.
    let packages: PackageInterfaces =
        names::package_interfaces_with(compat, &sources, memo).unwrap_or_default();
    let mut interfaces = names::visible(&packages, &direct);
    let mut paths = elm_files(&env);
    // The file itself, which may be open in the editor and not yet
    // saved. Discovery starts from the canonical source directories.
    let wanted = canonical_or_given(file);
    if !paths.contains(&wanted) {
        paths.push(wanted);
    }
    // A file no module name fits, or a cycle, stops a build; the index
    // takes what modules it can, and reads the rest on their own below.
    let graph = (Graph::from_paths(snapshot, &env, &paths))
        .or_else(|_| Graph::from_paths(snapshot, &env, &named(&env, &paths)))
        .or_else(|_| Graph::from_paths(snapshot, &env, &[]))
        .ok()?;
    let package = env.owner().package_name();
    // Only an index of the same packages has the same interfaces of them.
    let previous = previous.filter(|p| {
        p.compat == compat
            && p.package == package
            && p.packages.len() == sources.len()
            && (sources.iter()).all(|(name, dir)| p.packages.get(&name.to_string()) == Some(dir))
    });
    let mut visit = Visit {
        graph: &graph,
        compat,
        package: &package,
        interfaces: &mut interfaces,
        seen: HashSet::new(),
        modules: Vec::new(),
        reuse: previous.map(Reuse::new),
    };
    // Each module after the local modules it imports.
    let mut order: Vec<&ModuleName> = graph.modules.keys().collect();
    order.sort_by(|a, b| a.compare(b, compat));
    for name in order {
        visit.local(name);
    }
    for root in &graph.roots {
        if let Root::Outside(Ok(module)) = root {
            visit.module(module);
        }
    }
    visit.orphans(snapshot, &paths);
    let modules = visit.modules;
    let package_files = previous.map_or_else(Default::default, |p| Arc::clone(&p.package_files));
    let exposed = (exposing.into_iter())
        .map(|(name, modules)| {
            let modules = modules.iter().map(ToString::to_string).collect();
            (name.to_string(), modules)
        })
        .collect();
    Some(Index {
        compat,
        package,
        modules,
        exposed,
        interfaces,
        package_files,
        packages: (sources.into_iter())
            .map(|(name, dir)| (name.to_string(), dir))
            .collect(),
    })
}

/// The paths of `paths` that some module name fits: under a source
/// directory, each part a capitalized name.
fn named(env: &Env, paths: &[PathBuf]) -> Vec<PathBuf> {
    let fits = |path: &Path| {
        env.src_dirs().iter().any(|dir| {
            path.strip_prefix(dir).is_ok_and(|relative| {
                let parts: Vec<_> = relative.iter().map(|p| p.to_string_lossy()).collect();
                let last = parts.len().saturating_sub(1);
                parts.iter().enumerate().all(|(i, part)| {
                    let part = if i == last {
                        part.trim_end_matches(".elm")
                    } else {
                        part
                    };
                    part.chars().next().is_some_and(|c| c.is_ascii_uppercase())
                        && part.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
                })
            })
        })
    };
    paths.iter().filter(|p| fits(p)).cloned().collect()
}

/// The releases the project builds with that the store holds, and its
/// direct dependencies.
fn releases(store: &Store, outline: &Outline, compat: Compat) -> (Vec<(Name, PathBuf)>, Vec<Name>) {
    match outline {
        Outline::App(app) => {
            let sources = (app.direct.iter().chain(&app.indirect))
                .filter_map(|(name, &version)| {
                    Some((name.clone(), store.get(name, version).ok()??))
                })
                .collect();
            (sources, app.direct.keys().cloned().collect())
        }
        Outline::Pkg(pkg) => {
            let mut sources: Vec<(Name, PathBuf)> = Vec::new();
            resolve(
                store,
                compat,
                pkg.deps.iter().map(|(n, c)| (n.clone(), *c)).collect(),
                &mut sources,
            );
            (sources, pkg.deps.keys().cloned().collect())
        }
    }
}

struct Visit<'v> {
    graph: &'v Graph,
    compat: Compat,
    package: &'v str,
    interfaces: &'v mut HashMap<String, Interface>,
    seen: HashSet<ModuleName>,
    modules: Vec<Arc<Indexed>>,
    reuse: Option<Reuse<'v>>,
}

/// An earlier index of the project, whose modules a new one reuses.
struct Reuse<'p> {
    previous: &'p Index,
    /// Its modules, by path.
    by_path: HashMap<&'p Path, &'p Arc<Indexed>>,
    /// Its modules' names.
    names: HashSet<&'p str>,
    /// The names of the modules indexed so far.
    indexed: HashSet<String>,
    /// Whether each project module's interface is as in `previous`, once
    /// it is final.
    same: HashMap<String, bool>,
}

impl<'p> Reuse<'p> {
    fn new(previous: &'p Index) -> Reuse<'p> {
        let mut by_path = HashMap::new();
        for module in &previous.modules {
            by_path.entry(module.path.as_path()).or_insert(module);
        }
        Reuse {
            previous,
            by_path,
            names: previous.modules.iter().map(|m| m.name.as_str()).collect(),
            indexed: HashSet::new(),
            same: HashMap::new(),
        }
    }
}

impl Visit<'_> {
    fn local(&mut self, name: &ModuleName) {
        if !self.seen.insert(name.clone()) {
            return;
        }
        match self.graph.modules.get(name) {
            Some(Status::Local(module)) => self.module(module),
            // A module elm won't build, misnamed, unnamed or broken, is
            // still the editor's to navigate, as far as its tree goes.
            Some(Status::BadModule(bad)) => {
                if let (Some(parse), Some((_, text))) = (&bad.parse, &bad.source) {
                    let imports: Vec<ModuleName> =
                        bad.imports().into_iter().map(|i| i.name).collect();
                    self.visit(&bad.path, None, text, parse, &imports);
                }
            }
            Some(Status::Foreign(_) | Status::Kernel | Status::BadImport(_)) | None => {}
        }
    }

    fn module(&mut self, module: &Module) {
        let imports: Vec<ModuleName> = module.imports.iter().map(|i| i.name.clone()).collect();
        self.visit(
            &module.path,
            Some(&module.name),
            &module.text,
            &module.parse,
            &imports,
        );
    }

    /// Each file of `paths` no module name fits, such as `src/main.elm`,
    /// which is in no graph, read on its own.
    fn orphans(&mut self, snapshot: &Snapshot, paths: &[PathBuf]) {
        // Each path resolved once: resolving touches the disk.
        let mut indexed: HashSet<PathBuf> = (self.modules.iter())
            .map(|m| canonical_or_given(&m.path))
            .collect();
        for path in paths {
            if indexed.contains(&canonical_or_given(path)) {
                continue;
            }
            let Ok(text) = snapshot.read(path) else {
                continue;
            };
            let text: Arc<str> = text.into();
            let parse = nomi_syntax::parse_module(&text);
            let imports: Vec<ModuleName> = (ast::Module::cast(parse.syntax()).iter())
                .flat_map(written_imports)
                .map(|import| import.name)
                .collect();
            let before = self.modules.len();
            self.visit(path, None, &text, &parse, &imports);
            indexed.extend(
                self.modules[before..]
                    .iter()
                    .map(|m| canonical_or_given(&m.path)),
            );
        }
    }

    /// The previous index's module at `path`, where its text is `text` and
    /// each of the project's modules it imports has the interface it had.
    fn reusable(
        &mut self,
        path: &Path,
        text: &Arc<str>,
        imports: &[ModuleName],
    ) -> Option<Arc<Indexed>> {
        let reuse = self.reuse.as_mut()?;
        let module = Arc::clone(reuse.by_path.get(path).filter(|m| *m.text == **text)?);
        let unchanged = imports.iter().all(|import| {
            let name = import.to_string();
            if let Some(&same) = reuse.same.get(&name) {
                return same;
            }
            let local = reuse.names.contains(name.as_str()) || reuse.indexed.contains(&name);
            let same = !local || self.interfaces.get(&name) == reuse.previous.interfaces.get(&name);
            reuse.same.insert(name, same);
            same
        });
        unchanged.then_some(module)
    }

    /// `module` among the modules indexed.
    fn push(&mut self, module: Arc<Indexed>) {
        if let Some(reuse) = &mut self.reuse {
            reuse.indexed.insert(module.name.clone());
        }
        self.modules.push(module);
    }

    /// Canonicalizes a module after its local imports, whatever became
    /// of them: an import without an interface adds nothing. A module
    /// without a `name` is named by its header, or `Main` without one.
    fn visit(
        &mut self,
        path: &Path,
        name: Option<&ModuleName>,
        text: &Arc<str>,
        parse: &Parse,
        imports: &[ModuleName],
    ) {
        for import in imports {
            self.local(import);
        }
        if let Some(reused) = self.reusable(path, text, imports) {
            let previous = self.reuse.as_ref().map(|reuse| reuse.previous);
            if reused.canonical.is_some()
                && let Some(interface) = previous.and_then(|p| p.interfaces.get(&reused.name))
            {
                (self.interfaces).insert(reused.name.clone(), interface.clone());
            }
            self.push(reused);
            return;
        }
        let root = parse.syntax();
        let Some(syntax) = ast::Module::cast(root.clone()) else {
            return;
        };
        let hir = hir::lower(&syntax);
        let name = name.map_or_else(|| hir.name().to_owned(), ToString::to_string);
        let (canonical, references) =
            canonicalize_with_references(self.compat, self.package, self.interfaces, &hir);
        let canonical = canonical.ok();
        if let Some(canonical) = &canonical {
            // Typed where it type-checks, as written where it doesn't.
            let annotations = types::infer(self.compat, canonical).unwrap_or_default();
            (self.interfaces).insert(name.clone(), interface(canonical, &annotations));
        }
        self.push(Arc::new(Indexed {
            path: path.to_owned(),
            name,
            text: text.clone(),
            parses: parse.errors().is_empty(),
            green: root.green().into_owned(),
            hir,
            references,
            canonical,
            typed: OnceLock::new(),
        }));
    }
}
