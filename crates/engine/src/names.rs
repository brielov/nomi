//! Canonicalizing a build: package interfaces from their
//! sources, then each local module after the modules it imports, as
//! upstream's `Build.hs` checks them. A module whose import failed is
//! blocked and stays silent.

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

use nomi_base::diagnostic::Problem;
use nomi_base::diagnostic::code::Source;
use nomi_base::{Compat, TextRange, lock};
use nomi_compiler::canonical::Annotation;
use nomi_compiler::canonicalize::foreign::with_defaults;
use nomi_compiler::canonicalize::module::{Canonical, canonicalize, interface};
use nomi_compiler::canonicalize::report;
use nomi_compiler::docs;
use nomi_compiler::hir;
use nomi_compiler::interface::{Interface, Interfaces};
use nomi_compiler::nitpick::{main as main_check, patterns, report as patterns_report};
use nomi_compiler::optimize;
use nomi_compiler::optimized::{LocalGraph, binary};
use nomi_compiler::report::ReportContext;
use nomi_compiler::types::localizer::Localizer;
use nomi_compiler::types::{self, Inferred, report as types_report};
use nomi_project::{ModuleName, Name};
use nomi_syntax::SyntaxNode;
use nomi_syntax::ast::{self, AstNode};

use crate::artifacts::{Artifacts, Hash, Key};
use crate::graph::{self, Env, Graph, Module, Root, Status};
use crate::report::imports_amiss;
use crate::schedule::{self, Cancelled, Outcome};
use crate::timings::{self, Phase, time};
use crate::{Snapshot, objects};

/// Each package's exposed modules' interfaces, by package.
pub type PackageInterfaces = BTreeMap<Name, Arc<HashMap<String, Interface>>>;

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
    pub(crate) fn new(
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
    pub(crate) fn build(&self, open: &(dyn Fn(usize) + Sync)) {
        let mut built = lock(&self.built);
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
    pub(crate) fn finish(self) -> Result<objects::Dependencies, String> {
        self.build(&|_| {});
        let built = self
            .built
            .into_inner()
            .unwrap_or_else(PoisonError::into_inner);
        built.expect("the packages built")
    }
}

/// What checking a graph found and made.
#[derive(Debug, Default)]
pub(crate) struct Walked {
    /// The problems of each module the earlier phases passed that has
    /// some, by path.
    pub problems: HashMap<PathBuf, Vec<Problem>>,
    /// The modules checked rather than taken from the artifact cache.
    pub ran: Vec<PathBuf>,
    /// The documentation of each module asked for.
    pub docs: Vec<docs::Module>,
    /// Each module's optimized graph, when asked for.
    pub optimized: Vec<(ModuleName, LocalGraph)>,
    /// Each optimized module's interface, for the debugger's types.
    pub interfaces: Vec<ModuleInterface>,
    /// How many modules came out clean, checked or reused: what elm
    /// counts as compiled in a project of no `elm-stuff`.
    pub compiled: usize,
}

/// What a check makes beyond its problems.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Goal<'a> {
    Check,
    /// The documentation of these modules, as `elm make --docs`
    /// documents a package's exposed ones.
    Document(&'a [ModuleName]),
    /// Every module's optimized graph, as `elm make` makes them before it
    /// generates code.
    Optimize,
}

/// Each local module of `graph` checked after the local modules it
/// imports, every module the build reached but those with problems of
/// their own (`report::imports_amiss`) and those whose local imports
/// failed or were blocked; until `snapshot` goes stale. What `artifacts`
/// holds is reused (architecture §6): a module whose source, release,
/// package and imports' interfaces are what a clean check of it saw
/// before takes the interface it made then, unchecked. An edit that
/// leaves a module's interface as it was leaves its importers' keys as
/// they were, so they are skipped too: the early cutoff. What `goal` asks
/// for is made after everything else checks out, from modules always
/// checked afresh.
pub(crate) fn check_graph(
    env: &Env,
    graph: &Graph,
    packages: Packages<'_>,
    snapshot: &Snapshot,
    artifacts: Option<&Artifacts>,
    goal: Goal<'_>,
) -> Result<Walked, Cancelled> {
    let (nodes, imports) = nodes(env.compat(), graph);
    let walk = Walk {
        env,
        package: env.owner().package_name(),
        packages,
        artifacts,
        goal,
        nodes: &nodes,
        imports: &imports,
        package_hashes: Mutex::new(HashMap::new()),
    };
    let job = |i, deps: &[&Made]| walk.job(i, deps);
    let outcomes = match packages {
        Packages::Built(_) => schedule::run(Some(snapshot), &imports, job),
        // Each module waits on the packages exposing what it imports.
        Packages::Building(building) => {
            let gates: Vec<Vec<usize>> = (nodes.iter())
                .map(|node| {
                    let Node::Module(module) = node else {
                        return Vec::new();
                    };
                    let mut gates: Vec<usize> = (module.imports.iter())
                        .filter_map(|import| import.name.to_str())
                        .flat_map(|name| building.gates(name))
                        .copied()
                        .collect();
                    gates.sort_unstable();
                    gates.dedup();
                    gates
                })
                .collect();
            let open = |open: &(dyn Fn(usize) + Sync)| building.build(open);
            schedule::run_gated(Some(snapshot), &imports, &gates, open, job)
        }
    }?;
    let mut walked = Walked::default();
    for (node, outcome) in nodes.iter().zip(outcomes) {
        let Node::Module(module) = node else {
            continue;
        };
        match outcome {
            Outcome::Done(made) => {
                walked.compiled += 1;
                if made.fresh {
                    walked.ran.push(module.path.clone());
                }
                match made.asked {
                    Some(Asked::Docs(docs)) => walked.docs.push(docs),
                    Some(Asked::Graph(graph)) => {
                        walked.optimized.push((module.name.clone(), graph));
                        walked.interfaces.push(ModuleInterface {
                            name: module.name.clone(),
                            interface: made.interface,
                        });
                    }
                    None => {}
                }
            }
            Outcome::Failed(problems) => {
                walked.ran.push(module.path.clone());
                walked.problems.insert(module.path.clone(), problems);
            }
            Outcome::Blocked => {}
        }
    }
    Ok(walked)
}

/// The walk's nodes: the local modules of `graph` in the release's order
/// of names, then the roots outside the source directories; and the
/// local modules each imports, by place.
fn nodes(compat: Compat, graph: &Graph) -> (Vec<Node<'_>>, Vec<Vec<usize>>) {
    let (names, index) = graph::ordered(compat, &graph.modules);
    let node = |module| {
        if imports_amiss(graph, module) {
            Node::Failed
        } else {
            Node::Module(module)
        }
    };
    let mut nodes: Vec<Node<'_>> = (names.iter())
        .map(|name| match &graph.modules[*name] {
            Status::Local(module) => node(module),
            Status::BadModule(_) => Node::Failed,
            Status::Foreign(_) | Status::Kernel | Status::BadImport(_) => Node::Foreign,
        })
        .collect();
    for root in &graph.roots {
        if let Root::Outside(Ok(module)) = root {
            nodes.push(node(module));
        }
    }
    let mut imports: Vec<Vec<usize>> = (nodes.iter())
        .map(|node| {
            let Node::Module(module) = node else {
                return Vec::new();
            };
            let mut deps: Vec<usize> = (module.imports.iter())
                .filter_map(|import| index.get(&import.name).copied())
                .filter(|&d| !matches!(nodes[d], Node::Foreign))
                .collect();
            deps.dedup();
            deps
        })
        .collect();
    // A cycle was reported before canonicalization; its modules stay
    // silent, as upstream's walk leaves them.
    for i in schedule::cyclic(&imports) {
        imports[i].clear();
        nodes[i] = Node::Failed;
    }
    (nodes, imports)
}

/// A module of the build, as the walk sees it.
enum Node<'g> {
    /// A module to check.
    Module(&'g Module),
    /// One that won't be: it didn't parse, isn't where its name says, has
    /// problems with its imports or imports itself. Its importers are
    /// blocked.
    Failed,
    /// A name the graph resolved elsewhere.
    Foreign,
}

/// What a module made: its interface and that interface's hash, which
/// its importers read, and what the goal asks of it.
struct Made {
    interface: Arc<LazyInterface>,
    hash: Hash,
    /// Whether it was checked rather than taken from the artifact cache.
    fresh: bool,
    asked: Option<Asked>,
}

/// What the goal asks of a module beside its interface.
enum Asked {
    Docs(docs::Module),
    Graph(LocalGraph),
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

/// A module of the build, and its interface.
#[derive(Clone, Debug)]
pub struct ModuleInterface {
    pub name: ModuleName,
    pub interface: Arc<LazyInterface>,
}

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
    /// The package the modules are homes in.
    package: String,
    packages: Packages<'g>,
    artifacts: Option<&'g Artifacts>,
    goal: Goal<'g>,
    nodes: &'g [Node<'g>],
    imports: &'g [Vec<usize>],
    /// Each package interface's hash, made once.
    package_hashes: Mutex<HashMap<String, Hash>>,
}

impl Walk<'_> {
    /// Node `i`'s work, given what its local imports made: what it made,
    /// or its problems. A node that isn't a module to check fails with
    /// none.
    fn job(&self, i: usize, deps: &[&Made]) -> Result<Made, Vec<Problem>> {
        let Node::Module(module) = self.nodes[i] else {
            return Err(Vec::new());
        };
        let local: HashMap<&str, &Made> = (self.imports[i].iter().zip(deps))
            .filter_map(|(&d, made)| match self.nodes[d] {
                Node::Module(dep) => Some((dep.name.to_str()?, *made)),
                Node::Failed | Node::Foreign => None,
            })
            .collect();
        timings::module(&module.name, || self.module(module, &local))
    }

    /// `module`, its local imports having made `local`: from the artifact
    /// cache where it holds what the goal asks for, else checked.
    fn module(&self, module: &Module, local: &HashMap<&str, &Made>) -> Result<Made, Vec<Problem>> {
        let document =
            matches!(self.goal, Goal::Document(modules) if modules.contains(&module.name));
        let cache = (self.artifacts.filter(|_| !document))
            .map(|artifacts| (artifacts, self.key(module, local)));
        if let Some((artifacts, key)) = cache
            && let Some(made) = self.cached(artifacts, key)
        {
            return Ok(made);
        }
        self.compile(module, local, document, cache)
    }

    /// What `artifacts` files under `key`: the module's interface and,
    /// optimizing, its graph.
    fn cached(&self, artifacts: &Artifacts, key: Hash) -> Option<Made> {
        let (bytes, hash) = time(Phase::CacheRead, || artifacts.get_hashed(key))?;
        let asked = match self.goal {
            Goal::Optimize => Some(Asked::Graph(time(Phase::CacheRead, || {
                let bytes = artifacts.get(graph_key(key))?;
                binary::decode_local_graph(&bytes, self.env.compat())
            })?)),
            Goal::Check | Goal::Document(_) => None,
        };
        // The encoding is canonical, so its bytes hash as the interface's
        // own encoding would.
        Some(Made {
            interface: Arc::new(LazyInterface::encoded(bytes)),
            hash,
            fresh: false,
            asked,
        })
    }

    /// Canonicalizes, types and checks `module`, its local imports having
    /// made `local`, then makes what the goal asks of it: its
    /// documentation if it is `document`ed, its graph optimizing. What it
    /// makes is filed in `cache`.
    fn compile(
        &self,
        module: &Module,
        local: &HashMap<&str, &Made>,
        document: bool,
        cache: Option<(&Artifacts, Hash)>,
    ) -> Result<Made, Vec<Problem>> {
        let compat = self.env.compat();
        let interfaces = Layered {
            local: (local.iter())
                .map(|(name, made)| (*name, &*made.interface))
                .collect(),
            packages: self.packages,
        };
        let root = module.parse.syntax();
        let hir = time(Phase::Lower, || {
            hir::lower(&ast::Module::cast(root.clone()).expect("a module"))
        });
        let canonical = time(Phase::Canonicalize, || {
            canonicalize(compat, &self.package, &interfaces, &hir)
        });
        let canonical = canonical.map_err(|errors| {
            self.report(module, &root, |cx| {
                (errors.iter())
                    .map(|e| report::to_problem(e, cx, &hir))
                    .collect()
            })
        })?;
        // In the solver's order, the latest first, as upstream reports
        // them.
        let annotations = infer(compat, &canonical).map_err(|errors| {
            let localizer = self.localizer(&hir);
            self.report(module, &root, |cx| {
                (errors.iter())
                    .map(|e| types_report::to_problem(e, cx, &localizer))
                    .collect()
            })
        })?;
        self.checks(module, &root, &hir, &canonical, &annotations)?;
        let asked = match self.goal {
            Goal::Document(_) if document => {
                Some(Asked::Docs(self.document(module, &hir, &canonical)?))
            }
            Goal::Optimize => {
                let graph = optimize(compat, &annotations, &canonical, &module.text, &root);
                if let Some((artifacts, key)) = cache {
                    time(Phase::CacheWrite, || {
                        let bytes = binary::encode_local_graph(&graph, compat);
                        let _ = artifacts.put(graph_key(key), &bytes);
                    });
                }
                Some(Asked::Graph(graph))
            }
            Goal::Check | Goal::Document(_) => None,
        };
        let (interface, bytes) = time(Phase::Interface, || {
            let interface = interface(&canonical, &annotations);
            let bytes = interface.encode();
            (interface, bytes)
        });
        if let Some((artifacts, key)) = cache {
            // A cache that can't be written only costs a check next time.
            let _ = time(Phase::CacheWrite, || artifacts.put(key, &bytes));
        }
        Ok(Made {
            interface: Arc::new(LazyInterface::ready(interface)),
            hash: Hash::of(&bytes),
            fresh: true,
            asked,
        })
    }

    /// The checks after types: the patterns, as upstream's `nitpick`
    /// checks them, then `main`, as its `optimize` does.
    fn checks(
        &self,
        module: &Module,
        root: &SyntaxNode,
        hir: &hir::Module,
        canonical: &Canonical,
        annotations: &BTreeMap<String, Annotation>,
    ) -> Result<(), Vec<Problem>> {
        let compat = self.env.compat();
        let pattern_errors = time(Phase::Checks, || patterns::check(compat, &canonical.decls));
        if !pattern_errors.is_empty() {
            return Err(self.report(module, root, |cx| {
                (pattern_errors.iter())
                    .map(|e| patterns_report::to_problem(e, cx))
                    .collect()
            }));
        }
        let main = time(Phase::Checks, || {
            main_check::check(&canonical.decls, annotations)
        });
        if let Some(error) = main {
            let localizer = self.localizer(hir);
            return Err(self.report(module, root, |cx| {
                vec![main_check::to_problem(&error, cx, &localizer)]
            }));
        }
        Ok(())
    }

    /// The documentation of `module`, as `Build.compile` makes it.
    fn document(
        &self,
        module: &Module,
        hir: &hir::Module,
        canonical: &Canonical,
    ) -> Result<docs::Module, Vec<Problem>> {
        let compat = self.env.compat();
        let docs = time(Phase::Checks, || {
            docs::from_module(compat, &module.text, hir, canonical)
        });
        docs.map_err(|error| {
            let source = Source::new(&module.text);
            // Documentation's regions have no trivia in front.
            let cx = ReportContext {
                source: &source,
                compat,
                trim: &|range| range,
            };
            docs::report::to_problems(&error, &cx)
        })
    }

    /// The problems `report` makes with what reporting on `module` reads:
    /// its source, and its tree `root` for ranges without the trivia in
    /// front.
    fn report(
        &self,
        module: &Module,
        root: &SyntaxNode,
        report: impl FnOnce(&ReportContext<'_>) -> Vec<Problem>,
    ) -> Vec<Problem> {
        let source = Source::new(&module.text);
        let trim = |range| trim(root, range);
        report(&ReportContext {
            source: &source,
            compat: self.env.compat(),
            trim: &trim,
        })
    }

    /// How reports name the types `hir` sees.
    fn localizer(&self, hir: &hir::Module) -> Localizer {
        let imports = with_defaults(&self.package, &hir.imports);
        Localizer::from_module(self.env.compat(), hir.name(), imports)
    }

    /// The artifact key of `module`: everything checking it reads.
    fn key(&self, module: &Module, local: &HashMap<&str, &Made>) -> Hash {
        let mut key = Key::new("module")
            .part("compat", self.env.compat().as_str().as_bytes())
            .part("package", self.package.as_bytes())
            .part("source", module.text.as_bytes());
        for import in &module.imports {
            let hash = import.name.to_str().and_then(|name| match local.get(name) {
                Some(made) => Some(made.hash),
                None => self.package_hash(name),
            });
            key = key.part("import", import.name.as_bytes());
            key = match hash {
                Some(hash) => key.hash("interface", hash),
                None => key.part("interface", b"none"),
            };
        }
        key.finish()
    }

    /// The hash of the package interface of `module`, made once.
    fn package_hash(&self, module: &str) -> Option<Hash> {
        if let Some(hash) = lock(&self.package_hashes).get(module) {
            return Some(*hash);
        }
        let hash = Hash::of(&self.packages.get(module)?.encode());
        lock(&self.package_hashes).insert(module.to_owned(), hash);
        Some(hash)
    }
}

/// `canonical`'s types inferred, what inferring leaves freed once the
/// modules waiting on it have started.
pub(crate) fn infer(compat: Compat, canonical: &Canonical) -> Inferred {
    time(Phase::Types, || {
        let (inferred, leftovers) = types::infer_leaving(compat, canonical);
        crate::trash::defer(leftovers);
        inferred
    })
}

/// `canonical` optimized, its regions in `text`, whose tree is `root`.
pub(crate) fn optimize(
    compat: Compat,
    annotations: &BTreeMap<String, Annotation>,
    canonical: &Canonical,
    text: &str,
    root: &SyntaxNode,
) -> LocalGraph {
    let source = Source::new(text);
    let region = |range| source.region(trim(root, range), compat);
    time(Phase::Optimize, || {
        optimize::module(compat, annotations, canonical, &region)
    })
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
