//! What the editor navigates by (plan 5.16): every module of the project
//! that owns a file, canonicalized as far as it goes, with what each name
//! it writes refers to, and where the packages it uses keep their
//! sources. Read from the package store alone, never the network: an
//! application's releases are the ones its `elm.json` pins, and a
//! package's are the newest the store holds in each range.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use nomi_base::Compat;
use nomi_compiler::canonicalize::module::{canonicalize_with_references, interface};
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

/// Each package directory's exposed interfaces, kept between indexes.
pub type Memo = HashMap<PathBuf, HashMap<String, Interface>>;

/// A project's modules and packages, for navigation.
#[derive(Debug)]
pub struct Index {
    pub compat: Compat,
    /// The package the project's own modules are homes in;
    /// `author/project` for an application, as upstream names it.
    pub package: String,
    pub modules: Vec<Indexed>,
    /// Each dependency's source directory, by package name.
    pub packages: HashMap<String, PathBuf>,
}

/// One of the project's own modules.
#[derive(Debug)]
pub struct Indexed {
    pub path: PathBuf,
    pub name: String,
    pub text: Arc<str>,
    pub root: SyntaxNode,
    pub hir: hir::Module,
    pub references: Vec<Reference>,
}

impl Index {
    /// The module at `path`, which the index read as given.
    pub fn module(&self, path: &Path) -> Option<&Indexed> {
        let path = canonical(path);
        self.modules.iter().find(|m| canonical(&m.path) == path)
    }

    /// The project's module named `name`.
    pub fn named(&self, name: &str) -> Option<&Indexed> {
        self.modules.iter().find(|m| m.name == name)
    }
}

fn canonical(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_owned())
}

/// The `elm.json` of the project that owns `file`: the nearest above it
/// (R7).
pub fn manifest(snapshot: &Snapshot, file: &Path) -> Option<PathBuf> {
    file.ancestors()
        .skip(1)
        .map(|dir| dir.join("elm.json"))
        .find(|manifest| snapshot.is_file(manifest))
}

/// The index of the project that owns `file`; none where there is no
/// project, its `elm.json` isn't readable, or its modules import each
/// other in a cycle.
pub fn index(snapshot: &Snapshot, store: &Store, file: &Path, memo: &mut Memo) -> Option<Index> {
    let manifest = manifest(snapshot, file)?;
    let root = manifest.parent()?.to_owned();
    let (outline, compat) = Outline::detect(&snapshot.read_bytes(&manifest).ok()?).ok()?;
    let (sources, direct) = releases(store, &outline, compat);
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
    // A package nomi can't canonicalize leaves the project's own modules
    // to navigate.
    let packages: PackageInterfaces =
        names::package_interfaces_with(compat, &sources, memo).unwrap_or_default();
    let mut interfaces = names::visible(&packages, &direct);
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
    // The file itself, which may be open in the editor and not yet
    // saved.
    // Discovery starts from the canonical source directories.
    let wanted = canonical(file);
    if !paths.contains(&wanted) {
        paths.push(file.to_owned());
    }
    let graph = Graph::from_paths(snapshot, &env, &paths).ok()?;
    let package = match env.owner() {
        Owner::Package(name) => name.to_string(),
        Owner::Application => "author/project".to_owned(),
    };
    // Each module after the local modules it imports.
    let mut order: Vec<&ModuleName> = graph.modules.keys().collect();
    order.sort_by(|a, b| a.compare(b, compat));
    let mut visit = Visit {
        graph: &graph,
        compat,
        package: &package,
        interfaces: &mut interfaces,
        seen: HashSet::new(),
        modules: Vec::new(),
    };
    for name in order {
        visit.local(name);
    }
    for root in &graph.roots {
        if let Root::Outside(Ok(module)) = root {
            visit.module(module);
        }
    }
    let modules = visit.modules;
    Some(Index {
        compat,
        package,
        modules,
        packages: (sources.into_iter())
            .map(|(name, dir)| (name.to_string(), dir))
            .collect(),
    })
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
            let mut pending: Vec<_> = pkg.deps.iter().map(|(n, c)| (n.clone(), *c)).collect();
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
    modules: Vec<Indexed>,
}

impl Visit<'_> {
    fn local(&mut self, name: &ModuleName) {
        if !self.seen.insert(name.clone()) {
            return;
        }
        if let Some(Status::Local(module)) = self.graph.modules.get(name) {
            self.module(module);
        }
    }

    /// Canonicalizes `module` after its local imports, whatever became
    /// of them: an import without an interface adds nothing.
    fn module(&mut self, module: &crate::graph::Module) {
        for import in &module.imports {
            self.local(&import.name);
        }
        let root = module.parse.syntax();
        let Some(syntax) = ast::Module::cast(root.clone()) else {
            return;
        };
        let hir = hir::lower(&syntax);
        let (canonical, references) =
            canonicalize_with_references(self.compat, self.package, self.interfaces, &hir);
        if let Ok(canonical) = canonical {
            (self.interfaces).insert(module.name.to_string(), interface(&canonical));
        }
        self.modules.push(Indexed {
            path: module.path.clone(),
            name: module.name.to_string(),
            text: module.text.clone(),
            root,
            hir,
            references,
        });
    }
}
