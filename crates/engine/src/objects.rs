//! A project's dependencies built (`Elm/Details.hs`): each package from
//! its exposed modules, every module its build reaches canonicalized,
//! typed and optimized and every kernel it imports parsed, once per
//! release of it in the artifact cache. Its exposed interfaces are what
//! a project's modules import, and the packages' graphs merged are what
//! elm writes to `elm-stuff/0.19.x/o.dat`.
//! Upstream merges with left-biased unions folded from the right, so
//! where two graphs share a key, a record field's count mostly, the one
//! first in key order wins: modules by name in the release's order,
//! packages by author then project.

use crate::timings::{Phase, time};
use std::cmp::Ordering;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use nomi_base::Compat;
use nomi_base::diagnostic::code::Source;
use nomi_codegen_js::debug::{ModuleTypes, Types};
use nomi_codegen_js::kernel;
use nomi_compiler::canonical::Home;
use nomi_compiler::canonicalize::module::{canonicalize, interface};
use nomi_compiler::hir;
use nomi_compiler::interface::Interface;
use nomi_compiler::optimize;
use nomi_compiler::optimized::binary;
use nomi_compiler::optimized::{Chunk, GlobalGraph, LocalGraph};
use nomi_project::outline::PkgOutline;
use nomi_project::{Name, Outline};
use nomi_syntax::ast::{self, AstNode};
use nomi_syntax::parse_module;

use crate::artifacts::{Artifacts, Hash, Key};
use crate::names::trim;

/// What a package's build makes of one of its modules (`Details.Result`).
enum Built {
    Local(LocalGraph),
    Kernel(Vec<Chunk>),
}

/// A package built (`Details.Result` for each of its modules): its exposed
/// modules' interfaces, every module's unions and aliases, and its graph.
#[derive(Clone, Debug, Default)]
pub struct PackageBuild {
    pub exposed: HashMap<String, Interface>,
    pub types: Types,
    pub graph: GlobalGraph,
}

/// Each package's build, by its key: the release, the package, and its
/// dependencies' builds, as one release of a package builds differently
/// against different releases of its dependencies.
pub type Memo = HashMap<Hash, Arc<PackageBuild>>;

/// A project's dependencies built, each a package and its build, by name.
#[derive(Clone, Debug, Default)]
pub struct Dependencies {
    pub packages: BTreeMap<Name, Arc<PackageBuild>>,
}

impl Dependencies {
    /// Each package's exposed interfaces.
    pub fn exposed(&self) -> BTreeMap<Name, HashMap<String, Interface>> {
        (self.packages.iter())
            .map(|(name, build)| (name.clone(), build.exposed.clone()))
            .collect()
    }

    /// Every module's unions and aliases.
    pub fn types(&self) -> Types {
        (self.packages.values())
            .flat_map(|build| build.types.iter().map(|(h, t)| (h.clone(), t.clone())))
            .collect()
    }

    /// [`Dependencies::graph`] and [`Dependencies::types`], taking each
    /// build rather than copying it where nothing else holds it.
    pub fn into_graph_and_types(self) -> (GlobalGraph, Types) {
        let mut ordered: Vec<(Name, Arc<PackageBuild>)> = self.packages.into_iter().collect();
        ordered.sort_by(|(a, _), (b, _)| packages_order(a, b));
        let mut merged = GlobalGraph::default();
        let mut types = Types::new();
        for (_, build) in ordered.into_iter().rev() {
            let build = Arc::try_unwrap(build).unwrap_or_else(|shared| (*shared).clone());
            merged.nodes.extend(build.graph.nodes);
            merged.fields.extend(build.graph.fields);
            types.extend(build.types);
        }
        (merged, types)
    }

    /// The packages' graphs merged, the whole of what `o.dat` holds:
    /// `Map.foldr addObjects`, so the first package by name wins.
    pub fn graph(&self) -> GlobalGraph {
        let mut ordered: Vec<(&Name, &Arc<PackageBuild>)> = self.packages.iter().collect();
        ordered.sort_by(|(a, _), (b, _)| packages_order(a, b));
        let mut merged = GlobalGraph::default();
        for (_, build) in ordered.into_iter().rev() {
            merged.nodes.extend(build.graph.nodes.clone());
            merged.fields.extend(build.graph.fields.clone());
        }
        merged
    }
}

/// The merged graph of `packages`, each a name and the directory of its
/// source: the whole of what a project's `o.dat` holds.
pub fn dependency_graph(
    compat: Compat,
    packages: &[(Name, PathBuf)],
) -> Result<GlobalGraph, String> {
    Ok(build(compat, packages, &mut Memo::new(), None)?.graph())
}

/// Builds `packages`, each a name and the directory of its source,
/// dependencies first: each package once per run through `memo`, and once
/// per name, version, dependencies' builds, build of nomi and release
/// through `artifacts` (architecture §6). A module nomi can't build is an
/// error, naming it.
pub fn build(
    compat: Compat,
    packages: &[(Name, PathBuf)],
    memo: &mut Memo,
    artifacts: Option<&Artifacts>,
) -> Result<Dependencies, String> {
    build_each(compat, packages, memo, artifacts, &|_, _| {})
}

/// [`build`], telling `done` of each package as soon as it is built.
pub fn build_each(
    compat: Compat,
    packages: &[(Name, PathBuf)],
    memo: &mut Memo,
    artifacts: Option<&Artifacts>,
    done: &(dyn Fn(&Name, &Arc<PackageBuild>) + Sync),
) -> Result<Dependencies, String> {
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
    // Each package waits on its dependencies among `packages`, and builds
    // as soon as they have, on whichever thread is free.
    let names: Vec<&Name> = outlines.keys().collect();
    let index: HashMap<&Name, usize> = names.iter().enumerate().map(|(i, n)| (*n, i)).collect();
    let imports: Vec<Vec<usize>> = (names.iter())
        .map(|name| {
            (outlines[*name].0.deps.keys())
                .filter_map(|dep| index.get(dep).copied())
                .collect()
        })
        .collect();
    if !crate::names::cyclic(&imports).is_empty() {
        return Err("the packages depend on each other in a cycle".to_owned());
    }
    let memo_now: &Memo = memo;
    let job = |i: usize,
               deps: &[&(Hash, Arc<PackageBuild>)]|
     -> Result<(Hash, Arc<PackageBuild>), String> {
        let name = names[i];
        let (pkg, dir) = &outlines[name];
        let mut key = Key::new("package")
            .part("compat", compat.to_string().as_bytes())
            .part("name", name.to_string().as_bytes())
            .part("version", pkg.version.to_string().as_bytes());
        let mut dependencies: BTreeMap<Name, Arc<PackageBuild>> = BTreeMap::new();
        for (&d, (dep_key, dep_build)) in imports[i].iter().zip(deps.iter().map(|d| (&d.0, &d.1))) {
            key = key.part("dependency", names[d].to_string().as_bytes());
            key = key.hash("build", *dep_key);
            dependencies.insert(names[d].clone(), dep_build.clone());
        }
        let key = key.finish();
        if let Some(build) = memo_now.get(&key) {
            done(name, build);
            return Ok((key, build.clone()));
        }
        let cached = time(Phase::CacheRead, || {
            (artifacts.and_then(|a| a.get(key))).and_then(|bytes| decode_build(&bytes, compat))
        });
        let build = if let Some(build) = cached {
            build
        } else {
            let build = build_package(compat, name, pkg, dir, &dependencies)?;
            if let Some(artifacts) = artifacts {
                // A cache that can't be written only costs a build next
                // time.
                let _ = time(Phase::CacheWrite, || {
                    artifacts.put(key, &encode_build(&build, compat))
                });
            }
            build
        };
        let build = Arc::new(build);
        done(name, &build);
        Ok((key, build))
    };
    let snapshot = crate::Vfs::new().snapshot();
    let outcomes =
        crate::schedule::run(&snapshot, &imports, job).expect("a snapshot nothing edits");
    let mut built: BTreeMap<Name, Arc<PackageBuild>> = BTreeMap::new();
    for (name, outcome) in names.iter().zip(outcomes) {
        match outcome {
            crate::schedule::Outcome::Done((key, build)) => {
                memo.insert(key, build.clone());
                built.insert((*name).clone(), build);
            }
            crate::schedule::Outcome::Failed(why) => return Err(why),
            // Only after a dependency failed, which comes first.
            crate::schedule::Outcome::Blocked => {}
        }
    }
    // Only the packages asked for, though the memo may hold more.
    built.retain(|name, _| outlines.contains_key(name));
    Ok(Dependencies { packages: built })
}

/// Builds the package `name` from its exposed modules, its dependencies
/// among `built`.
fn build_package(
    compat: Compat,
    name: &Name,
    pkg: &PkgOutline,
    dir: &Path,
    built: &BTreeMap<Name, Arc<PackageBuild>>,
) -> Result<PackageBuild, String> {
    let mut visible: HashMap<String, Interface> = HashMap::new();
    let mut foreigns: BTreeMap<String, String> = BTreeMap::new();
    for dep in pkg.deps.keys() {
        if let Some(build) = built.get(dep) {
            for (module, iface) in &build.exposed {
                visible.insert(module.clone(), iface.clone());
                foreigns.insert(module.clone(), dep.to_string());
            }
        }
    }
    let exposed: Vec<String> = pkg.exposed.modules().map(|m| m.to_string()).collect();
    let kernels = name.author() == "elm" || name.author() == "elm-explorations";
    let package = Package {
        compat,
        name: name.to_string(),
        src: dir.join("src"),
        kernels,
        foreigns: &foreigns,
    };
    let (interfaces, modules) = package.build(&exposed, visible)?;
    let types: Types = (interfaces.iter())
        .filter(|(m, _)| matches!(modules.get(*m), Some(Built::Local(_))))
        .map(|(m, interface)| {
            let home = Home {
                package: name.to_string().into(),
                module: m.clone().into(),
            };
            (home, ModuleTypes::from_interface(interface))
        })
        .collect();
    let exposed: HashMap<String, Interface> = (interfaces.into_iter())
        .filter(|(m, _)| exposed.contains(m))
        .collect();
    let graph = package.gather(modules);
    Ok(PackageBuild {
        exposed,
        types,
        graph,
    })
}

/// A package's build as the artifact cache keeps it: the exposed
/// interfaces, each module's unions and aliases as an interface holding
/// only those, and the graph as `o.dat` would.
fn encode_build(build: &PackageBuild, compat: Compat) -> Vec<u8> {
    let mut out = Vec::new();
    let mut put = |bytes: &[u8]| {
        out.extend_from_slice(&(bytes.len() as u64).to_le_bytes());
        out.extend_from_slice(bytes);
    };
    let mut exposed: Vec<(&String, &Interface)> = build.exposed.iter().collect();
    exposed.sort_by(|a, b| a.0.cmp(b.0));
    put(&(exposed.len() as u64).to_le_bytes());
    for (module, interface) in exposed {
        put(module.as_bytes());
        put(&interface.encode());
    }
    let mut types: Vec<(&Home, &ModuleTypes)> = build.types.iter().collect();
    types.sort_by(|a, b| (&a.0.package, &a.0.module).cmp(&(&b.0.package, &b.0.module)));
    put(&(types.len() as u64).to_le_bytes());
    for (home, types) in types {
        put(home.package.as_bytes());
        put(home.module.as_bytes());
        put(&types.to_interface(&home.package).encode());
    }
    put(&binary::encode_global_graph(&build.graph, compat));
    out
}

/// [`encode_build`] read back; `None` for anything else.
fn decode_build(bytes: &[u8], compat: Compat) -> Option<PackageBuild> {
    let mut at = 0;
    let mut take = || -> Option<&[u8]> {
        let len = u64::from_le_bytes(bytes.get(at..at + 8)?.try_into().ok()?) as usize;
        let item = bytes.get(at + 8..(at + 8).checked_add(len)?)?;
        at += 8 + len;
        Some(item)
    };
    let count =
        |item: &[u8]| -> Option<usize> { Some(u64::from_le_bytes(item.try_into().ok()?) as usize) };
    let mut exposed = HashMap::new();
    for _ in 0..count(take()?)? {
        let module = String::from_utf8(take()?.to_vec()).ok()?;
        exposed.insert(module, Interface::decode(take()?)?);
    }
    let mut types = Types::new();
    for _ in 0..count(take()?)? {
        let package = String::from_utf8(take()?.to_vec()).ok()?;
        let module = String::from_utf8(take()?.to_vec()).ok()?;
        let interface = Interface::decode(take()?)?;
        types.insert(
            Home {
                package: package.into(),
                module: module.into(),
            },
            ModuleTypes::from_interface(&interface),
        );
    }
    let graph = binary::decode_global_graph(take()?, compat)?;
    (at == bytes.len()).then_some(PackageBuild {
        exposed,
        types,
        graph,
    })
}

/// `Ord Pkg.Name`: author, then project, each by bytes.
fn packages_order(a: &Name, b: &Name) -> Ordering {
    (a.author().as_bytes(), a.project().as_bytes())
        .cmp(&(b.author().as_bytes(), b.project().as_bytes()))
}

/// One package's build.
struct Package<'a> {
    compat: Compat,
    name: String,
    src: PathBuf,
    /// Whether its modules may import kernels, as `elm` and
    /// `elm-explorations` packages may.
    kernels: bool,
    /// The package exposing each module its dependencies expose.
    foreigns: &'a BTreeMap<String, String>,
}

/// A module of the package, read.
struct Read {
    text: String,
    parse: nomi_syntax::Parse,
    module: hir::Module,
}

/// What a crawl found under a name: a module to compile, a kernel, or
/// nothing, a module the package doesn't have being a foreign one.
enum Found {
    Module(Box<Read>),
    /// A kernel's chunks, and the modules it imports.
    Kernel(Vec<Chunk>, Vec<String>),
    Foreign,
}

/// What one module's compile made.
type Compiled = (Interface, LocalGraph);

/// The interfaces a package module reads: its package imports' over its
/// dependencies'.
struct Layered<'a> {
    local: HashMap<&'a str, &'a Interface>,
    visible: &'a HashMap<String, Interface>,
}

impl nomi_compiler::interface::Interfaces for Layered<'_> {
    fn get(&self, module: &str) -> Option<&Interface> {
        match self.local.get(module) {
            Some(interface) => Some(interface),
            None => self.visible.get(module),
        }
    }
}

impl Package<'_> {
    /// `crawlModule` from `exposed`, then `compile` of each module the
    /// crawl reached: every module's interface, and what each became. The
    /// crawl reads each wave of modules on the pool, and the modules
    /// compile on it along their imports.
    #[expect(
        clippy::type_complexity,
        reason = "the interfaces and the built modules, which the one caller takes apart at once"
    )]
    fn build(
        &self,
        exposed: &[String],
        mut interfaces: HashMap<String, Interface>,
    ) -> Result<(HashMap<String, Interface>, BTreeMap<String, Built>), String> {
        let mut modules: BTreeMap<String, Read> = BTreeMap::new();
        let mut built: BTreeMap<String, Built> = BTreeMap::new();
        let mut seen: HashSet<String> = HashSet::new();
        let mut wave: Vec<String> = (exposed.iter())
            .filter(|name| seen.insert((*name).clone()))
            .cloned()
            .collect();
        while !wave.is_empty() {
            let found = crate::Pool::global().map(wave, |name| {
                let found = self.find(&name);
                (name, found)
            });
            let mut next = Vec::new();
            for (name, found) in found {
                let (imports, found): (Vec<String>, Found) = match found? {
                    Found::Module(read) => {
                        let imports = (read.module.imports.iter())
                            .map(|i| i.name.text.clone())
                            .collect();
                        (imports, Found::Module(read))
                    }
                    Found::Kernel(chunks, imports) => (imports, Found::Kernel(chunks, Vec::new())),
                    Found::Foreign => (Vec::new(), Found::Foreign),
                };
                match found {
                    Found::Module(read) => {
                        modules.insert(name, *read);
                    }
                    Found::Kernel(chunks, _) => {
                        built.insert(name, Built::Kernel(chunks));
                    }
                    Found::Foreign => {}
                }
                next.extend(imports.into_iter().filter(|i| seen.insert(i.clone())));
            }
            wave = next;
        }
        let names: Vec<&String> = modules.keys().collect();
        let index: HashMap<&str, usize> = (names.iter().enumerate())
            .map(|(i, name)| (name.as_str(), i))
            .collect();
        let imports: Vec<Vec<usize>> = (names.iter())
            .map(|name| {
                (modules[*name].module.imports.iter())
                    .filter_map(|i| index.get(i.name.text.as_str()).copied())
                    .collect()
            })
            .collect();
        if !crate::names::cyclic(&imports).is_empty() {
            return Err(format!(
                "{}: its modules import each other in a cycle",
                self.name
            ));
        }
        let visible = &interfaces;
        let job = |i: usize, deps: &[&Compiled]| -> Result<Compiled, String> {
            let name = names[i];
            let local = (imports[i].iter().zip(deps))
                .map(|(&d, (interface, _))| (names[d].as_str(), interface))
                .collect();
            let layered = Layered { local, visible };
            crate::timings::module(&format!("{} {name}", self.name), || {
                self.compile(name, &modules[name], &layered)
            })
        };
        let snapshot = crate::Vfs::new().snapshot();
        let outcomes =
            crate::schedule::run(&snapshot, &imports, job).expect("a snapshot nothing edits");
        let mut compiled = Vec::new();
        for (name, outcome) in names.iter().zip(outcomes) {
            match outcome {
                crate::schedule::Outcome::Done(made) => compiled.push(((*name).clone(), made)),
                crate::schedule::Outcome::Failed(why) => return Err(why),
                // Only after an import failed, which comes first.
                crate::schedule::Outcome::Blocked => {}
            }
        }
        for (name, (interface, graph)) in compiled {
            interfaces.insert(name.clone(), interface);
            built.insert(name, Built::Local(graph));
        }
        Ok((interfaces, built))
    }

    /// What the crawl finds under `name`.
    fn find(&self, name: &str) -> Result<Found, String> {
        if let Some(short) = name.strip_prefix("Elm.Kernel.") {
            if !self.kernels {
                return Ok(Found::Foreign);
            }
            let file = self.src.join("Elm/Kernel").join(format!("{short}.js"));
            // A kernel the package doesn't have is a foreign one.
            let Ok(bytes) = fs::read(&file) else {
                return Ok(Found::Foreign);
            };
            let content = kernel::parse(&self.name, self.foreigns, &bytes)
                .ok_or_else(|| format!("{} {name}: a kernel that doesn't parse", self.name))?;
            let imports = content.imports.iter().map(|i| i.module.clone()).collect();
            return Ok(Found::Kernel(content.chunks, imports));
        }
        let file = self.src.join(format!("{}.elm", name.replace('.', "/")));
        // A module the package doesn't have is a foreign one.
        let Ok(text) = fs::read_to_string(&file) else {
            return Ok(Found::Foreign);
        };
        let parse = time(Phase::Parse, || parse_module(&text));
        let module = time(Phase::Lower, || {
            hir::lower(&ast::Module::cast(parse.syntax()).expect("a module"))
        });
        Ok(Found::Module(Box::new(Read {
            text,
            parse,
            module,
        })))
    }

    /// Canonicalizes, types and optimizes `name`, its imports' interfaces
    /// among `interfaces`.
    fn compile(
        &self,
        name: &str,
        read: &Read,
        interfaces: &Layered<'_>,
    ) -> Result<Compiled, String> {
        let package = &self.name;
        let canonical = time(Phase::Canonicalize, || {
            canonicalize(self.compat, package, interfaces, &read.module)
        })
        .map_err(|errors| format!("{package} {name}: {errors:?}"))?;
        let annotations = time(Phase::Types, || {
            let (inferred, leftovers) =
                nomi_compiler::types::infer_leaving(self.compat, &canonical);
            crate::trash::defer(leftovers);
            inferred
        })
        .map_err(|errors| format!("{package} {name}: type errors {errors:?}"))?;
        let root = read.parse.syntax();
        let source = Source::new(&read.text);
        let region = |range| source.region(trim(&root, range), self.compat);
        let graph = time(Phase::Optimize, || {
            optimize::module(self.compat, &annotations, &canonical, &region)
        });
        let made = time(Phase::Interface, || interface(&canonical, &annotations));
        Ok((made, graph))
    }

    /// `gatherObjects`: the modules' graphs and kernels folded from the
    /// right, so the first module by name wins a shared key; a kernel's
    /// field counts replace those of the modules after it.
    fn gather(&self, built: BTreeMap<String, Built>) -> GlobalGraph {
        let mut ordered: Vec<(String, Built)> = built.into_iter().collect();
        if self.compat == Compat::V0_19_3 {
            ordered.sort_by(|(a, _), (b, _)| a.len().cmp(&b.len()).then_with(|| a.cmp(b)));
        }
        let mut graph = GlobalGraph::default();
        for (name, built) in ordered.into_iter().rev() {
            match built {
                Built::Local(local) => {
                    graph.nodes.extend(local.nodes);
                    graph.fields.extend(local.fields);
                }
                Built::Kernel(chunks) => {
                    let short = name.strip_prefix("Elm.Kernel.").unwrap_or(&name);
                    kernel::add_kernel(self.compat, &mut graph, short, chunks);
                }
            }
        }
        graph
    }
}
