//! What the editor navigates by: every module of the project
//! that owns a file, canonicalized as far as it goes, with what each name
//! it writes refers to, and where the packages it uses keep their
//! sources. Read from the package store alone, never the network: an
//! application's releases are the ones its `elm.json` pins, and a
//! package's are the newest the store holds in each range.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use nomi_base::Compat;
use nomi_compiler::canonicalize::module::{Canonical, canonicalize_with_references, interface};
use nomi_compiler::canonicalize::references::Reference;
use nomi_compiler::hir;
use nomi_compiler::interface::Interface;
use nomi_project::{ModuleName, Name, Outline};
use nomi_registry::Store;
use nomi_syntax::SyntaxNode;
use nomi_syntax::ast::{self, AstNode};

use crate::check::env_of;
use crate::graph::{Foreigns, Graph, Owner, Root, Status};
use crate::names::{self, PackageInterfaces};
use crate::{Snapshot, Vfs};

/// Each package directory's build, kept between indexes.
pub type Memo = crate::objects::Memo;

/// A project's modules and packages, for navigation.
#[derive(Debug)]
pub struct Index {
    pub compat: Compat,
    /// The package the project's own modules are homes in;
    /// `author/project` for an application, as upstream names it.
    pub package: String,
    pub modules: Vec<Arc<Indexed>>,
    /// Each dependency's source directory, by package name.
    pub packages: HashMap<String, PathBuf>,
    /// The interface of every module the project's modules may import:
    /// the direct dependencies' exposed ones and the project's own.
    pub interfaces: HashMap<String, Interface>,
}

/// One of the project's own modules.
#[derive(Clone, Debug)]
pub struct Indexed {
    pub path: PathBuf,
    pub name: String,
    pub text: Arc<str>,
    /// Its tree, which [`Indexed::root`] makes a cursor into; shareable
    /// across threads as a cursor isn't.
    pub green: nomi_syntax::GreenNode,
    pub hir: hir::Module,
    pub references: Vec<Reference>,
    /// The module canonicalized, where it canonicalizes, for hover to
    /// infer its expressions' types.
    pub canonical: Option<Canonical>,
    /// Its types, inferred the first time they're asked for.
    pub typed: std::sync::OnceLock<(
        nomi_compiler::types::Inferred,
        nomi_compiler::types::ExprTypes,
    )>,
}

impl Indexed {
    /// Its tree.
    pub fn root(&self) -> SyntaxNode {
        SyntaxNode::new_root(self.green.clone())
    }

    /// Its declarations' types and its expressions', inferred once.
    pub fn types(
        &self,
        compat: Compat,
    ) -> Option<&(
        nomi_compiler::types::Inferred,
        nomi_compiler::types::ExprTypes,
    )> {
        let canonical = self.canonical.as_ref()?;
        Some(
            self.typed
                .get_or_init(|| nomi_compiler::types::infer_types(compat, canonical)),
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
        let path = canonical(path);
        (self.modules.iter())
            .find(|m| canonical(&m.path) == path)
            .map(|m| &**m)
    }

    /// The project's module named `name`.
    pub fn named(&self, name: &str) -> Option<&Indexed> {
        self.modules.iter().find(|m| m.name == name).map(|m| &**m)
    }
}

/// `path` with its symbolic links resolved as far as it exists, so a
/// buffer the editor hasn't saved has the name its directory gives it.
fn canonical(path: &Path) -> PathBuf {
    crate::vfs::canonical(path).unwrap_or_else(|_| path.to_owned())
}

/// The `elm.json` of the project that owns `file`: the nearest above it.
pub fn manifest(snapshot: &Snapshot, file: &Path) -> Option<PathBuf> {
    file.ancestors()
        .skip(1)
        .map(|dir| dir.join("elm.json"))
        .find(|manifest| snapshot.is_file(manifest))
}

/// A project as the editor sees it: its outline, its environment, and
/// what the store holds of its dependencies.
pub(crate) struct Setup {
    pub root: PathBuf,
    pub outline: Outline,
    pub compat: Compat,
    pub env: crate::graph::Env,
    pub sources: Vec<(Name, PathBuf)>,
    pub direct: Vec<Name>,
    /// Whether the store holds every dependency.
    pub complete: bool,
}

/// The project whose `elm.json` is `manifest`; none where it isn't
/// readable.
pub(crate) fn setup(snapshot: &Snapshot, store: &Store, manifest: &Path) -> Option<Setup> {
    let root = manifest.parent()?.to_owned();
    let (outline, compat) = Outline::detect(&snapshot.read_bytes(manifest).ok()?).ok()?;
    let (sources, direct) = releases(store, &outline, compat);
    let complete = match &outline {
        Outline::App(app) => sources.len() == app.direct.len() + app.indirect.len(),
        Outline::Pkg(pkg) => (pkg.deps.keys()).all(|name| sources.iter().any(|(n, _)| n == name)),
    };
    let mut exposing = Vec::new();
    for name in &direct {
        let Some((_, dir)) = sources.iter().find(|(n, _)| n == name) else {
            continue;
        };
        if let Some(Outline::Pkg(pkg)) = package_outline(dir, compat) {
            exposing.push((name.clone(), pkg.exposed.modules().cloned().collect()));
        }
    }
    let env = env_of(&root, &outline, compat, Foreigns::new(exposing)).ok()?;
    Some(Setup {
        root,
        outline,
        compat,
        env,
        sources,
        direct,
        complete,
    })
}

/// [`setup`] as elm-test and elm-language-server see the project: with its
/// test dependencies, and with `tests/` among its source directories
/// where it has one.
fn setup_with_tests(snapshot: &Snapshot, store: &Store, manifest: &Path) -> Option<Setup> {
    let mut setup = setup(snapshot, store, manifest)?;
    let (sources, direct) = match &setup.outline {
        Outline::App(app) => {
            let sources = (app.test_direct.iter().chain(&app.test_indirect))
                .filter_map(|(name, &version)| {
                    Some((name.clone(), store.get(name, version).ok()??))
                })
                .collect();
            (sources, app.test_direct.keys().cloned().collect::<Vec<_>>())
        }
        Outline::Pkg(pkg) => {
            let mut sources = setup.sources.clone();
            resolve(
                store,
                setup.compat,
                pkg.test_deps.iter().map(|(n, c)| (n.clone(), *c)).collect(),
                &mut sources,
            );
            (sources, pkg.test_deps.keys().cloned().collect())
        }
    };
    for (name, dir) in sources {
        if !setup.sources.iter().any(|(n, _)| *n == name) {
            setup.sources.push((name, dir));
        }
    }
    for name in direct {
        if !setup.direct.contains(&name) {
            setup.direct.push(name);
        }
    }
    let tests = setup.root.join("tests");
    let mut src_dirs = setup.env.src_dirs().to_vec();
    // Directories are the disk's; the Vfs keeps files.
    let listed = crate::vfs::canonical(&tests).is_ok_and(|t| src_dirs.contains(&t));
    if tests.is_dir() && !listed {
        src_dirs.push(tests);
    }
    let mut exposing = Vec::new();
    for name in &setup.direct {
        let Some((_, dir)) = setup.sources.iter().find(|(n, _)| n == name) else {
            continue;
        };
        if let Some(Outline::Pkg(pkg)) = package_outline(dir, setup.compat) {
            exposing.push((name.clone(), pkg.exposed.modules().cloned().collect()));
        }
    }
    let owner = setup.env.owner().clone();
    setup.env = crate::graph::Env::new(
        setup.compat,
        &setup.root,
        owner,
        &src_dirs,
        Foreigns::new(exposing),
    )
    .ok()?;
    Some(setup)
}

/// The newest release in the store of each package `pending` names, in
/// its range, and of each it depends on, added to `sources`.
fn resolve(
    store: &Store,
    compat: Compat,
    mut pending: Vec<(Name, nomi_project::Constraint)>,
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
        if let Some(Outline::Pkg(dep)) = package_outline(&dir, compat) {
            pending.extend(dep.deps.iter().map(|(n, c)| (n.clone(), *c)));
        }
        sources.push((name, dir));
    }
}

/// The `.elm` files under the source directories of `env`, from the disk.
pub(crate) fn discover(env: &crate::graph::Env) -> Vec<PathBuf> {
    let vfs = Vfs::new();
    let mut paths = Vec::new();
    for dir in env.src_dirs() {
        let found = vfs.discover(dir).unwrap_or_default();
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
/// interfaces, as an editor's next index mostly is.
pub fn index_from(
    snapshot: &Snapshot,
    store: &Store,
    file: &Path,
    memo: &mut Memo,
    previous: Option<&Index>,
) -> Option<Index> {
    let manifest = manifest(snapshot, file)?;
    let Setup {
        compat,
        env,
        sources,
        direct,
        ..
    } = setup_with_tests(snapshot, store, &manifest)?;
    // A package nomi can't canonicalize leaves the project's own modules
    // to navigate.
    let packages: PackageInterfaces =
        names::package_interfaces_with(compat, &sources, memo).unwrap_or_default();
    let mut interfaces = names::visible(&packages, &direct);
    let mut paths = discover(&env);
    // The file itself, which may be open in the editor and not yet
    // saved.
    // Discovery starts from the canonical source directories.
    let wanted = canonical(file);
    if !paths.contains(&wanted) {
        paths.push(wanted);
    }
    // A file no module name fits, or a cycle, stops a build; the index
    // takes what modules it can, and reads the rest on their own below.
    let graph = (Graph::from_paths(snapshot, &env, &paths))
        .or_else(|_| Graph::from_paths(snapshot, &env, &named(&env, &paths)))
        .or_else(|_| Graph::from_paths(snapshot, &env, &[]))
        .ok()?;
    let package = match env.owner() {
        Owner::Package(name) => name.to_string(),
        Owner::Application => "author/project".to_owned(),
    };
    // Each module after the local modules it imports.
    let mut order: Vec<&ModuleName> = graph.modules.keys().collect();
    order.sort_by(|a, b| a.compare(b, compat));
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
        previous,
        same: std::cell::RefCell::new(HashMap::new()),
    };
    for name in order {
        visit.local(name);
    }
    for root in &graph.roots {
        if let Root::Outside(Ok(module)) = root {
            visit.module(module);
        }
    }
    // A file no module name fits, such as `src/main.elm`, is in no graph;
    // it's read on its own.
    // Each path resolved once: resolving touches the disk.
    let mut indexed: HashSet<PathBuf> =
        (visit.modules.iter()).map(|m| canonical(&m.path)).collect();
    for path in &paths {
        if indexed.contains(&canonical(path)) {
            continue;
        }
        let Ok(text) = snapshot.read(path) else {
            continue;
        };
        let text: Arc<str> = text.into();
        let parse = nomi_syntax::parse_module(&text);
        let imports: Vec<ModuleName> = (ast::Module::cast(parse.syntax()).iter())
            .flat_map(crate::graph::written_imports)
            .map(|import| import.name)
            .collect();
        let before = visit.modules.len();
        visit.visit(path, None, &text, &parse.syntax(), &imports);
        indexed.extend(visit.modules[before..].iter().map(|m| canonical(&m.path)));
    }
    let modules = visit.modules;
    Some(Index {
        compat,
        package,
        modules,
        interfaces,
        packages: (sources.into_iter())
            .map(|(name, dir)| (name.to_string(), dir))
            .collect(),
    })
}

/// The paths of `paths` that some module name fits: under a source
/// directory, each part a capitalized name.
fn named(env: &crate::graph::Env, paths: &[PathBuf]) -> Vec<PathBuf> {
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

fn package_outline(dir: &Path, compat: Compat) -> Option<Outline> {
    Outline::decode(&std::fs::read(dir.join("elm.json")).ok()?, compat).ok()
}

struct Visit<'v> {
    graph: &'v Graph,
    compat: Compat,
    package: &'v str,
    interfaces: &'v mut HashMap<String, Interface>,
    seen: HashSet<ModuleName>,
    modules: Vec<Arc<Indexed>>,
    previous: Option<&'v Index>,
    /// Whether each project module's interface is as in `previous`, once
    /// it is final.
    same: std::cell::RefCell<HashMap<String, bool>>,
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
                    self.visit(&bad.path, None, text, &parse.syntax(), &imports);
                }
            }
            _ => {}
        }
    }

    fn module(&mut self, module: &crate::graph::Module) {
        let imports: Vec<ModuleName> = module.imports.iter().map(|i| i.name.clone()).collect();
        let syntax = module.parse.syntax();
        self.visit(
            &module.path,
            Some(&module.name),
            &module.text,
            &syntax,
            &imports,
        );
    }

    /// The previous index's module at `path`, where its text is `text` and
    /// each of the project's modules it imports has the interface it had.
    fn reusable(
        &self,
        path: &Path,
        text: &Arc<str>,
        imports: &[ModuleName],
    ) -> Option<Arc<Indexed>> {
        let previous = self.previous?;
        let module = (previous.modules.iter()).find(|m| m.path == path && *m.text == **text)?;
        let unchanged = imports.iter().all(|import| {
            let name = import.to_string();
            if let Some(&same) = self.same.borrow().get(&name) {
                return same;
            }
            let local = previous.modules.iter().any(|m| m.name == name)
                || self.modules.iter().any(|m| m.name == name);
            let same = !local || self.interfaces.get(&name) == previous.interfaces.get(&name);
            self.same.borrow_mut().insert(name, same);
            same
        });
        unchanged.then(|| Arc::clone(module))
    }

    /// Canonicalizes a module after its local imports, whatever became
    /// of them: an import without an interface adds nothing. A module
    /// without a `name` is named by its header, or `Main` without one.
    fn visit(
        &mut self,
        path: &Path,
        name: Option<&ModuleName>,
        text: &Arc<str>,
        root: &SyntaxNode,
        imports: &[ModuleName],
    ) {
        for import in imports {
            self.local(import);
        }
        if let Some(reused) = self.reusable(path, text, imports) {
            if reused.canonical.is_some()
                && let Some(interface) = self.previous.and_then(|p| p.interfaces.get(&reused.name))
            {
                (self.interfaces).insert(reused.name.clone(), interface.clone());
            }
            self.modules.push(reused);
            return;
        }
        let Some(syntax) = ast::Module::cast(root.clone()) else {
            return;
        };
        let hir = hir::lower(&syntax);
        let name = name.map_or_else(
            || nomi_compiler::canonicalize::module::name(&hir).to_owned(),
            ToString::to_string,
        );
        let (canonical, references) =
            canonicalize_with_references(self.compat, self.package, self.interfaces, &hir);
        let canonical = canonical.ok();
        if let Some(canonical) = &canonical {
            // Typed where it type-checks, as written where it doesn't.
            let annotations =
                nomi_compiler::types::infer(self.compat, canonical).unwrap_or_default();
            (self.interfaces).insert(name.clone(), interface(canonical, &annotations));
        }
        self.modules.push(Arc::new(Indexed {
            path: path.to_owned(),
            name,
            text: text.clone(),
            green: root.green().into_owned(),
            hir,
            references,
            canonical,
            typed: std::sync::OnceLock::new(),
        }));
    }
}
