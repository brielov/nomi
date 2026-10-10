//! Canonicalizing a build (plan 5.8): package interfaces from their
//! sources, then each local module after the modules it imports, as
//! upstream's `Build.hs` checks them. A module whose import failed is
//! blocked and stays silent.

use std::collections::{BTreeMap, HashMap, HashSet};

use std::path::PathBuf;

use nomi_base::diagnostic::Problem;
use nomi_base::diagnostic::code::Source;
use nomi_base::{Compat, TextRange};
use nomi_compiler::canonicalize::foreign::with_defaults;
use nomi_compiler::canonicalize::module::{canonicalize, interface};
use nomi_compiler::canonicalize::report;
use nomi_compiler::docs;
use nomi_compiler::hir;
use nomi_compiler::interface::Interface;
use nomi_compiler::nitpick::{main as main_check, patterns, report as patterns_report};
use nomi_compiler::optimize;
use nomi_compiler::optimized::{LocalGraph, binary};
use nomi_compiler::types::localizer::Localizer;
use nomi_compiler::types::report as types_report;
use nomi_project::{ModuleName, Name};
use nomi_syntax::SyntaxNode;
use nomi_syntax::ast::{self, AstNode};

use crate::artifacts::{Artifacts, Hash, Key};
use crate::graph::{Env, Graph, Module, Owner, Root, Status};
use crate::objects;

/// Each package's exposed modules' interfaces, by package.
pub type PackageInterfaces = BTreeMap<Name, HashMap<String, Interface>>;

/// The interfaces of every exposed module of `packages`, each a package
/// name and the directory of its source, as `objects::build` builds
/// them. A module nomi can't build is an error, naming it.
pub fn package_interfaces(
    compat: Compat,
    packages: &[(Name, PathBuf)],
) -> Result<PackageInterfaces, String> {
    package_interfaces_with(compat, packages, &mut objects::Memo::new())
}

/// [`package_interfaces`], reusing and adding to `memo`. A release's
/// interfaces depend on its dependencies' releases only through the types
/// they name, so a run over many projects may treat them as fixed.
pub fn package_interfaces_with(
    compat: Compat,
    packages: &[(Name, PathBuf)],
    memo: &mut objects::Memo,
) -> Result<PackageInterfaces, String> {
    Ok(objects::build(compat, packages, memo, None)?.exposed())
}

/// The interfaces a project of these direct dependencies imports from.
pub fn visible(packages: &PackageInterfaces, direct: &[Name]) -> HashMap<String, Interface> {
    direct
        .iter()
        .filter_map(|name| packages.get(name))
        .flat_map(|exposed| exposed.iter().map(|(m, i)| (m.clone(), i.clone())))
        .collect()
}

/// Each local module's naming and type problems, by path: every module
/// the build reached whose path isn't in `failed` and none of whose local
/// imports failed or was blocked, checked after those imports.
pub fn canonicalize_graph(
    env: &Env,
    graph: &Graph,
    packages: &HashMap<String, Interface>,
    failed: &HashSet<PathBuf>,
) -> HashMap<PathBuf, Vec<Problem>> {
    check_graph(env, graph, packages, failed, None, Goals::default()).problems
}

/// What checking a graph found, and which modules it checked rather than
/// took from the artifact cache.
#[derive(Debug, Default)]
pub struct Checked {
    pub problems: HashMap<PathBuf, Vec<Problem>>,
    pub ran: Vec<PathBuf>,
    /// The documentation of each module asked for that has it.
    pub docs: Vec<docs::Module>,
    /// Each module's optimized graph, when asked for, in the order checked.
    pub optimized: Vec<(ModuleName, LocalGraph)>,
    /// Each optimized module's interface, for the debugger's types.
    pub interfaces: Vec<(ModuleName, Interface)>,
}

/// What a check makes beyond its problems.
#[derive(Clone, Copy, Debug, Default)]
pub struct Goals<'a> {
    /// The modules to document, as `elm make --docs` documents a
    /// package's exposed modules.
    pub documented: &'a [ModuleName],
    /// Whether to optimize every module, as `elm make` does before it
    /// generates code.
    pub optimize: bool,
}

/// [`canonicalize_graph`], reusing `artifacts` (architecture §6): a
/// module whose source, release, package and imports' interfaces are
/// what a clean check of it saw before takes the interface it made then,
/// unchecked. An edit that leaves a module's interface as it was leaves
/// its importers' keys as they were, so they are skipped too: the early
/// cutoff. What `goals` asks for is made after everything else checks
/// out, from modules always checked afresh.
pub fn check_graph(
    env: &Env,
    graph: &Graph,
    packages: &HashMap<String, Interface>,
    failed: &HashSet<PathBuf>,
    artifacts: Option<&Artifacts>,
    goals: Goals<'_>,
) -> Checked {
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
        artifacts,
        hashes: HashMap::new(),
        ran: Vec::new(),
        goals,
        docs: Vec::new(),
        optimized: Vec::new(),
        optimized_interfaces: Vec::new(),
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
    Checked {
        problems: walk.problems,
        ran: walk.ran,
        docs: walk.docs,
        optimized: walk.optimized,
        interfaces: walk.optimized_interfaces,
    }
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
    artifacts: Option<&'g Artifacts>,
    /// Each interface's hash, by module, made once.
    hashes: HashMap<String, Hash>,
    /// The modules checked, in order.
    ran: Vec<PathBuf>,
    goals: Goals<'g>,
    docs: Vec<docs::Module>,
    optimized: Vec<(ModuleName, LocalGraph)>,
    optimized_interfaces: Vec<(ModuleName, Interface)>,
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
        let document = self.goals.documented.contains(&module.name);
        let key = (self.artifacts.filter(|_| !document)).map(|_| self.key(module));
        if let (Some(artifacts), Some(key)) = (self.artifacts, key)
            && let Some(interface) = artifacts.get(key).as_deref().and_then(Interface::decode)
        {
            // Optimizing, the module's graph must be filed too.
            let graph = (self.goals.optimize).then(|| {
                let bytes = artifacts.get(graph_key(key))?;
                binary::decode_local_graph(&bytes, self.env.compat())
            });
            match graph {
                None => {
                    self.interfaces.insert(module.name.to_string(), interface);
                    return true;
                }
                Some(Some(graph)) => {
                    self.optimized.push((module.name.clone(), graph));
                    let entry = (module.name.clone(), interface.clone());
                    self.optimized_interfaces.push(entry);
                    self.interfaces.insert(module.name.to_string(), interface);
                    return true;
                }
                Some(None) => {}
            }
        }
        self.ran.push(module.path.clone());
        let root = module.parse.syntax();
        let hir = hir::lower(&ast::Module::cast(root.clone()).expect("a module"));
        match canonicalize(self.env.compat(), &self.package, &self.interfaces, &hir) {
            Ok(canonical) => match nomi_compiler::types::infer(self.env.compat(), &canonical) {
                Ok(annotations) => {
                    // Then the patterns, as upstream's `nitpick`.
                    let pattern_errors = patterns::check(self.env.compat(), &canonical.decls);
                    if !pattern_errors.is_empty() {
                        let source = Source::new(&module.text);
                        let trim = |range| trim(&root, range);
                        let cx = patterns_report::ReportContext {
                            source: &source,
                            compat: self.env.compat(),
                            trim: &trim,
                        };
                        let problems = (pattern_errors.iter())
                            .map(|e| patterns_report::to_problem(e, &cx))
                            .collect();
                        self.problems.insert(module.path.clone(), problems);
                        return false;
                    }
                    // Then `main`, as upstream's `optimize` checks it.
                    if let Some(error) = main_check::check(&canonical.decls, &annotations) {
                        let source = Source::new(&module.text);
                        let trim = |range| trim(&root, range);
                        let imports = with_defaults(&self.package, &hir.imports);
                        let localizer = Localizer::from_module(
                            self.env.compat(),
                            nomi_compiler::canonicalize::module::name(&hir),
                            &imports,
                        );
                        let problem = main_check::to_problem(
                            &error,
                            &source,
                            self.env.compat(),
                            &trim,
                            &localizer,
                        );
                        self.problems.insert(module.path.clone(), vec![problem]);
                        return false;
                    }
                    // Then the documentation, as `Build.compile` makes it.
                    if document {
                        match docs::from_module(self.env.compat(), &module.text, &hir, &canonical) {
                            Ok(docs) => self.docs.push(docs),
                            Err(error) => {
                                let source = Source::new(&module.text);
                                let problems =
                                    docs::report::to_problems(&error, &source, self.env.compat());
                                self.problems.insert(module.path.clone(), problems);
                                return false;
                            }
                        }
                    }
                    if self.goals.optimize {
                        let source = Source::new(&module.text);
                        let region = |range: TextRange| {
                            let kept = source.region(trim(&root, range), self.env.compat());
                            nomi_compiler::optimized::Region {
                                start: (kept.start.row, kept.start.col),
                                end: (kept.end.row, kept.end.col),
                            }
                        };
                        let graph =
                            optimize::module(self.env.compat(), &annotations, &canonical, &region);
                        if let (Some(artifacts), Some(key)) = (self.artifacts, key) {
                            let bytes = binary::encode_local_graph(&graph, self.env.compat());
                            let _ = artifacts.put(graph_key(key), &bytes);
                        }
                        self.optimized.push((module.name.clone(), graph));
                    }
                    let interface = interface(&canonical, &annotations);
                    if let (Some(artifacts), Some(key)) = (self.artifacts, key) {
                        // A cache that can't be written only costs a
                        // check next time.
                        let _ = artifacts.put(key, &interface.encode());
                    }
                    if self.goals.optimize {
                        let entry = (module.name.clone(), interface.clone());
                        self.optimized_interfaces.push(entry);
                    }
                    self.interfaces.insert(module.name.to_string(), interface);
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

impl Walk<'_> {
    /// The artifact key of `module`: everything checking it reads.
    fn key(&mut self, module: &Module) -> Hash {
        let mut key = Key::new("module")
            .part("toolchain", env!("CARGO_PKG_VERSION").as_bytes())
            .part("compat", self.env.compat().to_string().as_bytes())
            .part("package", self.package.as_bytes())
            .part("source", module.text.as_bytes());
        for import in &module.imports {
            let name = import.name.to_string();
            let hash = match self.hashes.get(&name) {
                Some(hash) => Some(*hash),
                None => self.interfaces.get(&name).map(|i| {
                    let hash = Hash::of(&i.encode());
                    self.hashes.insert(name.clone(), hash);
                    hash
                }),
            };
            key = key.part("import", name.as_bytes());
            key = match hash {
                Some(hash) => key.hash("interface", hash),
                None => key.part("interface", b"none"),
            };
        }
        key.finish()
    }
}

/// Where a module's optimized graph is filed: beside the interface filed
/// under `key`, made of the same inputs.
fn graph_key(key: Hash) -> Hash {
    Key::new("graph").hash("module", key).finish()
}

/// From the first token of `range` that isn't trivia: node ranges keep
/// the comments and spaces before them.
pub(crate) fn trim(root: &SyntaxNode, range: TextRange) -> TextRange {
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
