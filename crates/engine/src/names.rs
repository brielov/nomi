//! Canonicalizing a build: package interfaces from their
//! sources, then each local module after the modules it imports, as
//! upstream's `Build.hs` checks them. A module whose import failed is
//! blocked and stays silent.

use crate::timings::{Phase, time};
use std::collections::{BTreeMap, HashMap, HashSet};

use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};

use nomi_base::diagnostic::Problem;
use nomi_base::diagnostic::code::Source;
use nomi_base::{Compat, TextRange};
use nomi_compiler::canonicalize::foreign::with_defaults;
use nomi_compiler::canonicalize::module::{canonicalize, interface};
use nomi_compiler::canonicalize::report;
use nomi_compiler::docs;
use nomi_compiler::hir;
use nomi_compiler::interface::{Interface, Interfaces};
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
use crate::schedule::{self, Outcome};

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
    let exposed = objects::build(compat, packages, memo, None)?.exposed();
    // What's kept for later calls is only what they read: across a corpus
    // of packages, their graphs would fill memory.
    for build in memo.values_mut() {
        if !build.graph.nodes.is_empty() || !build.types.is_empty() {
            *build = Arc::new(objects::PackageBuild {
                exposed: build.exposed.clone(),
                ..objects::PackageBuild::default()
            });
        }
    }
    Ok(exposed)
}

/// The interfaces a project of these direct dependencies imports from.
pub fn visible(packages: &PackageInterfaces, direct: &[Name]) -> HashMap<String, Interface> {
    direct
        .iter()
        .filter_map(|name| packages.get(name))
        .flat_map(|exposed| exposed.iter().map(|(m, i)| (m.clone(), i.clone())))
        .collect()
}

/// Where a check finds the interfaces of the modules the direct
/// dependencies expose.
#[derive(Clone, Copy, Debug)]
pub enum Packages<'a> {
    /// Built before it.
    Built(&'a HashMap<String, Interface>),
    /// Built alongside it, each module starting once the packages it
    /// imports are built.
    Building(&'a Building<'a>),
}

impl<'a> From<&'a HashMap<String, Interface>> for Packages<'a> {
    fn from(built: &'a HashMap<String, Interface>) -> Packages<'a> {
        Packages::Built(built)
    }
}

impl<'a> Packages<'a> {
    fn get(self, module: &str) -> Option<&'a Interface> {
        match self {
            Packages::Built(built) => built.get(module),
            Packages::Building(building) => building.interface(module),
        }
    }
}

/// A project's dependencies, to build alongside its modules: the
/// packages, and the modules each direct dependency exposes.
#[derive(Debug)]
pub struct Building<'a> {
    compat: Compat,
    sources: Vec<(Name, PathBuf)>,
    artifacts: Option<&'a Artifacts>,
    direct: Vec<Name>,
    /// Each exposed module's direct dependencies, by their place in
    /// `direct`; where there are several, the last is the one whose
    /// interface is read, as [`visible`] reads them.
    exposers: HashMap<String, Vec<usize>>,
    /// Each direct dependency's build, once it is built.
    builds: Vec<OnceLock<Arc<objects::PackageBuild>>>,
    built: Mutex<Option<Result<objects::Dependencies, String>>>,
}

impl<'a> Building<'a> {
    /// `sources`, each a package and the directory of its source, to
    /// build; `direct`, each direct dependency and the modules it exposes.
    pub fn new(
        compat: Compat,
        sources: Vec<(Name, PathBuf)>,
        artifacts: Option<&'a Artifacts>,
        direct: Vec<(Name, Vec<ModuleName>)>,
    ) -> Building<'a> {
        let mut exposers: HashMap<String, Vec<usize>> = HashMap::new();
        for (i, (_, modules)) in direct.iter().enumerate() {
            for module in modules {
                exposers.entry(module.to_string()).or_default().push(i);
            }
        }
        Building {
            compat,
            sources,
            artifacts,
            builds: direct.iter().map(|_| OnceLock::new()).collect(),
            direct: direct.into_iter().map(|(name, _)| name).collect(),
            exposers,
            built: Mutex::new(None),
        }
    }

    /// Builds the packages, once, opening each direct dependency's gate,
    /// its place among them, as soon as it is built.
    pub fn build(&self, open: &(dyn Fn(usize) + Sync)) {
        let mut built = self.built.lock().unwrap();
        if built.is_some() {
            return;
        }
        let done = |name: &Name, build: &Arc<objects::PackageBuild>| {
            if let Some(i) = self.direct.iter().position(|d| d == name) {
                let _ = self.builds[i].set(build.clone());
                open(i);
            }
        };
        let mut memo = objects::Memo::new();
        *built = Some(time(Phase::Packages, || {
            objects::build_each(self.compat, &self.sources, &mut memo, self.artifacts, &done)
        }));
    }

    /// The direct dependencies a module importing `module` waits on.
    fn gates(&self, module: &str) -> &[usize] {
        self.exposers.get(module).map_or(&[], Vec::as_slice)
    }

    fn interface(&self, module: &str) -> Option<&Interface> {
        let &i = self.gates(module).last()?;
        self.builds[i].get()?.exposed.get(module)
    }

    /// What building the packages came to.
    pub fn finish(self) -> Result<objects::Dependencies, String> {
        self.build(&|_| {});
        (self.built.into_inner().unwrap()).expect("the packages built")
    }
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
    check_graph(env, graph, packages.into(), failed, None, Goals::default()).problems
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
    pub interfaces: Vec<(ModuleName, Arc<LazyInterface>)>,
    /// How many modules came out clean, checked or reused: what elm
    /// counts as compiled in a project of no `elm-stuff`.
    pub compiled: usize,
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
    packages: Packages<'_>,
    failed: &HashSet<PathBuf>,
    artifacts: Option<&Artifacts>,
    goals: Goals<'_>,
) -> Checked {
    let package = match env.owner() {
        Owner::Package(name) => name.to_string(),
        Owner::Application => "author/project".to_owned(),
    };
    // The local modules in the release's order of names, then the roots
    // outside the source directories.
    let mut names: Vec<&ModuleName> = graph.modules.keys().collect();
    names.sort_by(|a, b| a.compare(b, env.compat()));
    let mut nodes: Vec<Node<'_>> = (names.iter())
        .map(|name| match &graph.modules[*name] {
            Status::Local(module) => Node::Module(module),
            Status::BadModule(_) => Node::Bad,
            _ => Node::Foreign,
        })
        .collect();
    let index: HashMap<&ModuleName, usize> =
        names.iter().enumerate().map(|(i, n)| (*n, i)).collect();
    for root in &graph.roots {
        if let Root::Outside(Ok(module)) = root {
            nodes.push(Node::Module(module));
        }
    }
    // Each module waits on the local modules it imports; one already
    // failed waits on nothing, as it won't run.
    let mut imports: Vec<Vec<usize>> = (nodes.iter())
        .map(|node| match node {
            Node::Module(module) if !failed.contains(&module.path) => {
                let mut deps: Vec<usize> = (module.imports.iter())
                    .filter_map(|import| index.get(&import.name).copied())
                    .filter(|&d| !matches!(nodes[d], Node::Foreign))
                    .collect();
                deps.dedup();
                deps
            }
            _ => Vec::new(),
        })
        .collect();
    // A cycle was reported before canonicalization; its modules stay
    // silent, as upstream's walk leaves them.
    let cyclic = cyclic(&imports);
    for &i in &cyclic {
        imports[i].clear();
    }
    let walk = Walk {
        env,
        package,
        packages,
        failed,
        artifacts,
        goals,
        nodes: &nodes,
        imports: &imports,
        cyclic: &cyclic,
        package_hashes: Mutex::new(HashMap::new()),
        problems: Mutex::new(HashMap::new()),
        ran: Mutex::new(Vec::new()),
        docs: Mutex::new(Vec::new()),
        optimized: Mutex::new(Vec::new()),
        optimized_interfaces: Mutex::new(Vec::new()),
    };
    let snapshot = crate::Vfs::new().snapshot();
    let job = |i, deps: &[&Made]| walk.job(i, deps);
    let outcomes = match packages {
        Packages::Built(_) => schedule::run(&snapshot, &imports, job),
        // Each module waits on the packages exposing what it imports.
        Packages::Building(building) => {
            let gates: Vec<Vec<usize>> = (nodes.iter())
                .map(|node| match node {
                    Node::Module(module) => {
                        let mut gates: Vec<usize> = (module.imports.iter())
                            .flat_map(|import| building.gates(&import.name.to_string()))
                            .copied()
                            .collect();
                        gates.sort_unstable();
                        gates.dedup();
                        gates
                    }
                    _ => Vec::new(),
                })
                .collect();
            schedule::run_gated(
                &snapshot,
                &imports,
                &gates,
                |open| building.build(open),
                job,
            )
        }
    };
    let outcomes = outcomes.expect("a snapshot nothing edits");
    let compiled = (nodes.iter().zip(&outcomes))
        .filter(|(node, outcome)| {
            matches!(node, Node::Module(..)) && matches!(outcome, Outcome::Done(_))
        })
        .count();
    Checked {
        problems: walk.problems.into_inner().unwrap(),
        ran: by_node(walk.ran.into_inner().unwrap()),
        docs: by_node(walk.docs.into_inner().unwrap()),
        optimized: by_node(walk.optimized.into_inner().unwrap()),
        interfaces: by_node(walk.optimized_interfaces.into_inner().unwrap()),
        compiled,
    }
}

/// `items` in the order of the nodes that made them.
fn by_node<T>(mut items: Vec<(usize, T)>) -> Vec<T> {
    items.sort_by_key(|(i, _)| *i);
    items.into_iter().map(|(_, item)| item).collect()
}

/// The modules on an import cycle, by Kahn's algorithm.
pub(crate) fn cyclic(imports: &[Vec<usize>]) -> Vec<usize> {
    let mut waiting: Vec<usize> = imports.iter().map(Vec::len).collect();
    let mut importers = vec![Vec::new(); imports.len()];
    for (i, deps) in imports.iter().enumerate() {
        for &d in deps {
            importers[d].push(i);
        }
    }
    let mut ready: Vec<usize> = (0..imports.len()).filter(|&i| waiting[i] == 0).collect();
    let mut done = vec![false; imports.len()];
    while let Some(i) = ready.pop() {
        done[i] = true;
        for &importer in &importers[i] {
            waiting[importer] -= 1;
            if waiting[importer] == 0 {
                ready.push(importer);
            }
        }
    }
    (0..imports.len()).filter(|&i| !done[i]).collect()
}

/// A module of the build, as the walk sees it.
enum Node<'g> {
    /// A module to check.
    Module(&'g Module),
    /// One that didn't parse or isn't where its name says.
    Bad,
    /// A name the graph resolved elsewhere.
    Foreign,
}

/// What a module made that its importers need: its interface and that
/// interface's hash.
struct Made {
    interface: Arc<LazyInterface>,
    hash: Hash,
}

/// A module's interface, decoded from the artifact cache only when
/// something reads it: a warm build reads none.
#[derive(Debug)]
pub struct LazyInterface {
    bytes: Vec<u8>,
    decoded: OnceLock<Interface>,
}

impl LazyInterface {
    fn ready(interface: Interface) -> LazyInterface {
        LazyInterface {
            bytes: Vec::new(),
            decoded: OnceLock::from(interface),
        }
    }

    fn encoded(bytes: Vec<u8>) -> LazyInterface {
        LazyInterface {
            bytes,
            decoded: OnceLock::new(),
        }
    }

    /// The interface. The artifact cache checks each entry's hash, so a
    /// cached interface decodes.
    pub fn get(&self) -> &Interface {
        self.decoded.get_or_init(|| {
            Interface::decode(&self.bytes).expect("an interface the cache vouches for")
        })
    }
}

/// A module and its interface.
type ModuleInterface = (ModuleName, Arc<LazyInterface>);

/// The interfaces one module reads: its local imports' over the
/// packages'.
struct Layered<'a> {
    local: HashMap<&'a str, &'a LazyInterface>,
    packages: Packages<'a>,
}

impl Interfaces for Layered<'_> {
    fn get(&self, module: &str) -> Option<&Interface> {
        match self.local.get(module) {
            Some(lazy) => Some(lazy.get()),
            None => self.packages.get(module),
        }
    }
}

struct Walk<'g> {
    env: &'g Env,
    package: String,
    packages: Packages<'g>,
    failed: &'g HashSet<PathBuf>,
    artifacts: Option<&'g Artifacts>,
    goals: Goals<'g>,
    nodes: &'g [Node<'g>],
    imports: &'g [Vec<usize>],
    cyclic: &'g [usize],
    /// Each package interface's hash, made once.
    package_hashes: Mutex<HashMap<String, Hash>>,
    problems: Mutex<HashMap<PathBuf, Vec<Problem>>>,
    /// The modules checked rather than reused, by node.
    ran: Mutex<Vec<(usize, PathBuf)>>,
    docs: Mutex<Vec<(usize, docs::Module)>>,
    optimized: Mutex<Vec<(usize, (ModuleName, LocalGraph))>>,
    optimized_interfaces: Mutex<Vec<(usize, ModuleInterface)>>,
}

impl Walk<'_> {
    /// Node `i`'s work, given what its local imports made.
    fn job(&self, i: usize, deps: &[&Made]) -> Result<Made, ()> {
        let Node::Module(module) = &self.nodes[i] else {
            return Err(());
        };
        if self.failed.contains(&module.path) || self.cyclic.contains(&i) {
            return Err(());
        }
        let local: HashMap<&str, &Made> = (self.imports[i].iter().zip(deps))
            .filter_map(|(&d, made)| match &self.nodes[d] {
                Node::Module(dep) => Some((dep.name.to_str()?, *made)),
                _ => None,
            })
            .collect();
        let name = module.name.to_string();
        crate::timings::module(&name, || self.module(i, module, &local))
    }

    /// Canonicalizes `module`, its local imports having made `local`.
    fn module(&self, i: usize, module: &Module, local: &HashMap<&str, &Made>) -> Result<Made, ()> {
        let interfaces = Layered {
            local: local
                .iter()
                .map(|(name, made)| (*name, &*made.interface))
                .collect(),
            packages: self.packages,
        };
        let document = self.goals.documented.contains(&module.name);
        let key = (self.artifacts.filter(|_| !document)).map(|_| self.key(module, local));
        if let (Some(artifacts), Some(key)) = (self.artifacts, key)
            && let Some((bytes, hash)) = time(Phase::CacheRead, || artifacts.get_hashed(key))
        {
            // The encoding is canonical, so its bytes hash as the
            // interface's own encoding would.
            let made = Made {
                interface: Arc::new(LazyInterface::encoded(bytes)),
                hash,
            };
            // Optimizing, the module's graph must be filed too.
            let graph = (self.goals.optimize).then(|| {
                time(Phase::CacheRead, || {
                    let bytes = artifacts.get(graph_key(key))?;
                    binary::decode_local_graph(&bytes, self.env.compat())
                })
            });
            match graph {
                None => return Ok(made),
                Some(Some(graph)) => {
                    self.optimized
                        .lock()
                        .unwrap()
                        .push((i, (module.name.clone(), graph)));
                    let entry = (module.name.clone(), made.interface.clone());
                    self.optimized_interfaces.lock().unwrap().push((i, entry));
                    return Ok(made);
                }
                Some(None) => {}
            }
        }
        self.ran.lock().unwrap().push((i, module.path.clone()));
        let root = module.parse.syntax();
        let hir = time(Phase::Lower, || {
            hir::lower(&ast::Module::cast(root.clone()).expect("a module"))
        });
        let canonical = time(Phase::Canonicalize, || {
            canonicalize(self.env.compat(), &self.package, &interfaces, &hir)
        });
        match canonical {
            Ok(canonical) => match time(Phase::Types, || {
                let (inferred, leftovers) =
                    nomi_compiler::types::infer_leaving(self.env.compat(), &canonical);
                crate::trash::defer(leftovers);
                inferred
            }) {
                Ok(annotations) => {
                    // Then the patterns, as upstream's `nitpick`.
                    let pattern_errors = time(Phase::Checks, || {
                        patterns::check(self.env.compat(), &canonical.decls)
                    });
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
                        self.problems
                            .lock()
                            .unwrap()
                            .insert(module.path.clone(), problems);
                        return Err(());
                    }
                    // Then `main`, as upstream's `optimize` checks it.
                    let main = time(Phase::Checks, || {
                        main_check::check(&canonical.decls, &annotations)
                    });
                    if let Some(error) = main {
                        let source = Source::new(&module.text);
                        let trim = |range| trim(&root, range);
                        let imports = with_defaults(&self.package, &hir.imports);
                        let localizer =
                            Localizer::from_module(self.env.compat(), hir.name(), &imports);
                        let problem = main_check::to_problem(
                            &error,
                            &source,
                            self.env.compat(),
                            &trim,
                            &localizer,
                        );
                        self.problems
                            .lock()
                            .unwrap()
                            .insert(module.path.clone(), vec![problem]);
                        return Err(());
                    }
                    // Then the documentation, as `Build.compile` makes it.
                    if document {
                        let docs = time(Phase::Checks, || {
                            docs::from_module(self.env.compat(), &module.text, &hir, &canonical)
                        });
                        match docs {
                            Ok(docs) => self.docs.lock().unwrap().push((i, docs)),
                            Err(error) => {
                                let source = Source::new(&module.text);
                                let problems =
                                    docs::report::to_problems(&error, &source, self.env.compat());
                                self.problems
                                    .lock()
                                    .unwrap()
                                    .insert(module.path.clone(), problems);
                                return Err(());
                            }
                        }
                    }
                    if self.goals.optimize {
                        let source = Source::new(&module.text);
                        let region =
                            |range: TextRange| source.region(trim(&root, range), self.env.compat());
                        let graph = time(Phase::Optimize, || {
                            optimize::module(self.env.compat(), &annotations, &canonical, &region)
                        });
                        if let (Some(artifacts), Some(key)) = (self.artifacts, key) {
                            time(Phase::CacheWrite, || {
                                let bytes = binary::encode_local_graph(&graph, self.env.compat());
                                let _ = artifacts.put(graph_key(key), &bytes);
                            });
                        }
                        self.optimized
                            .lock()
                            .unwrap()
                            .push((i, (module.name.clone(), graph)));
                    }
                    let (interface, bytes) = time(Phase::Interface, || {
                        let interface = interface(&canonical, &annotations);
                        let bytes = interface.encode();
                        (interface, bytes)
                    });
                    if let (Some(artifacts), Some(key)) = (self.artifacts, key) {
                        // A cache that can't be written only costs a
                        // check next time.
                        let _ = time(Phase::CacheWrite, || artifacts.put(key, &bytes));
                    }
                    let made = Made {
                        interface: Arc::new(LazyInterface::ready(interface)),
                        hash: Hash::of(&bytes),
                    };
                    if self.goals.optimize {
                        let entry = (module.name.clone(), made.interface.clone());
                        self.optimized_interfaces.lock().unwrap().push((i, entry));
                    }
                    Ok(made)
                }
                Err(errors) => {
                    let source = Source::new(&module.text);
                    let imports = with_defaults(&self.package, &hir.imports);
                    let localizer = Localizer::from_module(self.env.compat(), hir.name(), &imports);
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
                    self.problems
                        .lock()
                        .unwrap()
                        .insert(module.path.clone(), problems);
                    Err(())
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
                self.problems
                    .lock()
                    .unwrap()
                    .insert(module.path.clone(), problems);
                Err(())
            }
        }
    }
}

impl Walk<'_> {
    /// The artifact key of `module`: everything checking it reads.
    fn key(&self, module: &Module, local: &HashMap<&str, &Made>) -> Hash {
        let mut key = Key::new("module")
            .part("toolchain", env!("CARGO_PKG_VERSION").as_bytes())
            .part("compat", self.env.compat().to_string().as_bytes())
            .part("package", self.package.as_bytes())
            .part("source", module.text.as_bytes());
        for import in &module.imports {
            let name = import.name.to_string();
            let hash = match local.get(name.as_str()) {
                Some(made) => Some(made.hash),
                None => self.package_hash(&name),
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

impl Walk<'_> {
    /// The hash of the package interface of `module`, made once.
    fn package_hash(&self, module: &str) -> Option<Hash> {
        if let Some(hash) = self.package_hashes.lock().unwrap().get(module) {
            return Some(*hash);
        }
        let hash = Hash::of(&self.packages.get(module)?.encode());
        self.package_hashes
            .lock()
            .unwrap()
            .insert(module.to_owned(), hash);
        Some(hash)
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
