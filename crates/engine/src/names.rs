//! Canonicalizing a build (plan 5.8): package interfaces from their
//! sources, then each local module after the modules it imports, as
//! upstream's `Build.hs` checks them. A module whose import failed is
//! blocked and stays silent.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use nomi_base::diagnostic::Problem;
use nomi_base::diagnostic::code::Source;
use nomi_base::{Compat, TextRange};
use nomi_compiler::canonicalize::foreign::with_defaults;
use nomi_compiler::canonicalize::module::{canonicalize, interface};
use nomi_compiler::canonicalize::report;
use nomi_compiler::hir;
use nomi_compiler::interface::Interface;
use nomi_compiler::types::localizer::Localizer;
use nomi_compiler::types::report as types_report;
use nomi_project::{ModuleName, Name, Outline};
use nomi_syntax::ast::{self, AstNode};
use nomi_syntax::{SyntaxNode, parse_module};

use crate::graph::{Env, Graph, Module, Owner, Root, Status};

/// Each package's exposed modules' interfaces, by package.
pub type PackageInterfaces = BTreeMap<Name, HashMap<String, Interface>>;

/// The interfaces of every exposed module of `packages`, each a package
/// name and the directory of its source, canonicalized from the source
/// in dependency order. A module nomi can't canonicalize is an error,
/// naming it.
pub fn package_interfaces(
    compat: Compat,
    packages: &[(Name, PathBuf)],
) -> Result<PackageInterfaces, String> {
    package_interfaces_with(compat, packages, &mut HashMap::new())
}

/// [`package_interfaces`], reusing and adding to `memo`, each package's
/// exposed interfaces by its directory. A release's interfaces depend on
/// its dependencies' releases only through the types they name, so a run
/// over many projects may treat them as fixed.
pub fn package_interfaces_with(
    compat: Compat,
    packages: &[(Name, PathBuf)],
    memo: &mut HashMap<PathBuf, HashMap<String, Interface>>,
) -> Result<PackageInterfaces, String> {
    let mut outlines = BTreeMap::new();
    for (name, dir) in packages {
        let bytes = fs::read(dir.join("elm.json")).map_err(|e| format!("{name}: {e}"))?;
        match Outline::decode(&bytes, compat) {
            Ok(Outline::Pkg(pkg)) => {
                outlines.insert(name.clone(), (pkg, dir.clone()));
            }
            _ => return Err(format!("{name}'s elm.json isn't a package's")),
        }
    }
    let mut done: PackageInterfaces = BTreeMap::new();
    // Dependencies first.
    let mut pending: Vec<Name> = outlines.keys().cloned().collect();
    while !pending.is_empty() {
        let ready = pending
            .iter()
            .position(|name| {
                outlines[name]
                    .0
                    .deps
                    .keys()
                    .all(|d| done.contains_key(d) || !outlines.contains_key(d))
            })
            .ok_or("the packages depend on each other in a cycle")?;
        let name = pending.remove(ready);
        let (pkg, dir) = &outlines[&name];
        if let Some(exposed) = memo.get(dir) {
            done.insert(name.clone(), exposed.clone());
            continue;
        }
        let mut visible: HashMap<String, Interface> = HashMap::new();
        for dep in pkg.deps.keys() {
            if let Some(exposed) = done.get(dep) {
                visible.extend(exposed.iter().map(|(m, i)| (m.clone(), i.clone())));
            }
        }
        let exposed: Vec<String> = pkg.exposed.modules().map(|m| m.to_string()).collect();
        let own = package(
            compat,
            &name.to_string(),
            &dir.join("src"),
            &exposed,
            visible,
        )?;
        let exposed: HashSet<String> = exposed.into_iter().collect();
        let own: HashMap<String, Interface> = (own.into_iter())
            .filter(|(m, _)| exposed.contains(m))
            .collect();
        memo.insert(dir.clone(), own.clone());
        done.insert(name.clone(), own);
    }
    Ok(done)
}

/// The interfaces of the modules under `src` that `exposed` reaches,
/// each canonicalized after the modules of the package it imports. A
/// package builds from its exposed modules, so a file nothing exposed
/// imports is never read, as upstream's crawl never reads it.
fn package(
    compat: Compat,
    package: &str,
    src: &Path,
    exposed: &[String],
    mut interfaces: HashMap<String, Interface>,
) -> Result<HashMap<String, Interface>, String> {
    let mut modules: BTreeMap<String, hir::Module> = BTreeMap::new();
    let mut pending: Vec<String> = exposed.to_vec();
    while let Some(name) = pending.pop() {
        if modules.contains_key(&name) {
            continue;
        }
        let file = src.join(format!("{}.elm", name.replace('.', "/")));
        let Ok(text) = fs::read_to_string(&file) else {
            continue;
        };
        let parse = parse_module(&text);
        let module = hir::lower(&ast::Module::cast(parse.syntax()).expect("a module"));
        pending.extend(module.imports.iter().map(|i| i.name.text.clone()));
        modules.insert(name, module);
    }
    let mut own = HashMap::new();
    let mut state: HashMap<String, bool> = HashMap::new();
    for name in exposed {
        if modules.contains_key(name) {
            visit(
                compat,
                package,
                name,
                &modules,
                &mut state,
                &mut interfaces,
                &mut own,
            )?;
        }
    }
    Ok(own)
}

fn visit(
    compat: Compat,
    package: &str,
    name: &str,
    modules: &BTreeMap<String, hir::Module>,
    state: &mut HashMap<String, bool>,
    interfaces: &mut HashMap<String, Interface>,
    own: &mut HashMap<String, Interface>,
) -> Result<(), String> {
    if state.contains_key(name) {
        return Ok(());
    }
    state.insert(name.to_owned(), false);
    let module = &modules[name];
    for import in &module.imports {
        if modules.contains_key(&import.name.text) {
            visit(
                compat,
                package,
                &import.name.text,
                modules,
                state,
                interfaces,
                own,
            )?;
        }
    }
    let canonical = canonicalize(compat, package, interfaces, module)
        .map_err(|errors| format!("{package} {name}: {errors:?}"))?;
    let annotations = nomi_compiler::types::infer(compat, &canonical)
        .map_err(|errors| format!("{package} {name}: type errors {errors:?}"))?;
    let iface = interface(&canonical, &annotations);
    interfaces.insert(name.to_owned(), iface.clone());
    own.insert(name.to_owned(), iface);
    state.insert(name.to_owned(), true);
    Ok(())
}

/// The interfaces a project of these direct dependencies imports from.
pub fn visible(packages: &PackageInterfaces, direct: &[Name]) -> HashMap<String, Interface> {
    direct
        .iter()
        .filter_map(|name| packages.get(name))
        .flat_map(|exposed| exposed.iter().map(|(m, i)| (m.clone(), i.clone())))
        .collect()
}

/// Each local module's naming problems, by path: every module the build
/// reached whose path isn't in `failed` and none of whose local imports
/// failed or was blocked, canonicalized after those imports.
pub fn canonicalize_graph(
    env: &Env,
    graph: &Graph,
    packages: &HashMap<String, Interface>,
    failed: &HashSet<PathBuf>,
) -> HashMap<PathBuf, Vec<Problem>> {
    let package = match env.owner() {
        Owner::Package(name) => name.to_string(),
        Owner::Application => "author/project".to_owned(),
    };
    let mut walk = Walk {
        env,
        graph,
        package,
        interfaces: packages.clone(),
        failed,
        state: HashMap::new(),
        problems: HashMap::new(),
    };
    let mut names: Vec<&ModuleName> = graph.modules.keys().collect();
    names.sort_by(|a, b| a.compare(b, env.compat()));
    for name in names {
        walk.local(name);
    }
    for root in &graph.roots {
        if let Root::Outside(Ok(module)) = root {
            walk.module(module);
        }
    }
    walk.problems
}

struct Walk<'g> {
    env: &'g Env,
    graph: &'g Graph,
    package: String,
    interfaces: HashMap<String, Interface>,
    failed: &'g HashSet<PathBuf>,
    /// Whether each module visited canonicalized cleanly.
    state: HashMap<ModuleName, bool>,
    problems: HashMap<PathBuf, Vec<Problem>>,
}

impl Walk<'_> {
    /// Whether the local module `name` is fine to import.
    fn local(&mut self, name: &ModuleName) -> bool {
        if let Some(&ok) = self.state.get(name) {
            return ok;
        }
        // A cycle is reported before canonicalization; mark it to stop.
        self.state.insert(name.clone(), false);
        let ok = match self.graph.modules.get(name) {
            Some(Status::Local(module)) => self.module(module),
            Some(Status::BadModule(_)) => false,
            _ => true,
        };
        self.state.insert(name.clone(), ok);
        ok
    }

    /// Canonicalizes `module` if it and its local imports are fine.
    fn module(&mut self, module: &Module) -> bool {
        if self.failed.contains(&module.path) {
            return false;
        }
        let mut blocked = false;
        for import in &module.imports {
            if matches!(
                self.graph.modules.get(&import.name),
                Some(Status::Local(_) | Status::BadModule(_))
            ) && !self.local(&import.name)
            {
                blocked = true;
            }
        }
        if blocked {
            return false;
        }
        let root = module.parse.syntax();
        let hir = hir::lower(&ast::Module::cast(root.clone()).expect("a module"));
        match canonicalize(self.env.compat(), &self.package, &self.interfaces, &hir) {
            Ok(canonical) => match nomi_compiler::types::infer(self.env.compat(), &canonical) {
                Ok(annotations) => {
                    (self.interfaces)
                        .insert(module.name.to_string(), interface(&canonical, &annotations));
                    true
                }
                Err(errors) => {
                    let source = Source::new(&module.text);
                    let imports = with_defaults(&self.package, &hir.imports);
                    let localizer = Localizer::from_module(
                        self.env.compat(),
                        nomi_compiler::canonicalize::module::name(&hir),
                        &imports,
                    );
                    let trim = |range| trim(&root, range);
                    let cx = types_report::ReportContext {
                        source: &source,
                        compat: self.env.compat(),
                        trim: &trim,
                        localizer: &localizer,
                    };
                    // In the solver's order, the latest first, as
                    // upstream reports them.
                    let problems = errors
                        .iter()
                        .map(|e| types_report::to_problem(e, &cx))
                        .collect();
                    self.problems.insert(module.path.clone(), problems);
                    false
                }
            },
            Err(errors) => {
                let source = Source::new(&module.text);
                let trim = |range| trim(&root, range);
                let cx = report::Context {
                    source: &source,
                    module: &hir,
                    compat: self.env.compat(),
                    trim: &trim,
                };
                let problems = errors.iter().map(|e| report::to_problem(e, &cx)).collect();
                self.problems.insert(module.path.clone(), problems);
                false
            }
        }
    }
}

/// From the first token of `range` that isn't trivia: node ranges keep
/// the comments and spaces before them.
fn trim(root: &SyntaxNode, range: TextRange) -> TextRange {
    let Some(mut token) = root.token_at_offset(range.start()).right_biased() else {
        return range;
    };
    while token.kind().is_trivia() {
        match token.next_token() {
            Some(next) if next.text_range().start() < range.end() => token = next,
            _ => return range,
        }
    }
    let start = token.text_range().start().max(range.start());
    TextRange::new(start, range.end().max(start))
}
