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
use std::collections::{BTreeMap, HashMap};
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
use nomi_compiler::optimized::{Chunk, GlobalGraph, LocalGraph, Region};
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

/// Each package's build, by the directory of its source.
pub type Memo = HashMap<PathBuf, Arc<PackageBuild>>;

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
        let mut built: BTreeMap<Name, Arc<PackageBuild>> = BTreeMap::new();
        for (&d, (dep_key, dep_build)) in imports[i].iter().zip(deps.iter().map(|d| (&d.0, &d.1))) {
            key = key.part("dependency", names[d].to_string().as_bytes());
            key = key.hash("build", *dep_key);
            built.insert(names[d].clone(), dep_build.clone());
        }
        let key = key.finish();
        if let Some(build) = memo_now.get(dir) {
            return Ok((key, build.clone()));
        }
        let cached = time(Phase::CacheRead, || {
            (artifacts.and_then(|a| a.get(key))).and_then(|bytes| decode_build(&bytes, compat))
        });
        let build = match cached {
            Some(build) => build,
            None => {
                let build = build_package(compat, name, pkg, dir, &built)?;
                if let Some(artifacts) = artifacts {
                    // A cache that can't be written only costs a build next
                    // time.
                    let _ = time(Phase::CacheWrite, || {
                        artifacts.put(key, &encode_build(&build, compat))
                    });
                }
                build
            }
        };
        Ok((key, Arc::new(build)))
    };
    let snapshot = crate::Vfs::new().snapshot();
    let outcomes =
        crate::schedule::run(&snapshot, &imports, job).expect("a snapshot nothing edits");
    let mut built: BTreeMap<Name, Arc<PackageBuild>> = BTreeMap::new();
    for (name, outcome) in names.iter().zip(outcomes) {
        match outcome {
            crate::schedule::Outcome::Done((_, build)) => {
                memo.insert(outlines[*name].1.clone(), build.clone());
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
    module: hir::Module,
}

impl Package<'_> {
    /// `crawlModule` from `exposed`, then `compile` of each module the
    /// crawl reached: every module's interface, and what each became.
    #[allow(clippy::type_complexity)]
    fn build(
        &self,
        exposed: &[String],
        mut interfaces: HashMap<String, Interface>,
    ) -> Result<(HashMap<String, Interface>, BTreeMap<String, Built>), String> {
        let mut modules: BTreeMap<String, Read> = BTreeMap::new();
        let mut built: BTreeMap<String, Built> = BTreeMap::new();
        let mut pending: Vec<String> = exposed.to_vec();
        while let Some(name) = pending.pop() {
            if modules.contains_key(&name) || built.contains_key(&name) {
                continue;
            }
            if let Some(short) = name.strip_prefix("Elm.Kernel.") {
                if !self.kernels {
                    continue;
                }
                let file = self.src.join("Elm/Kernel").join(format!("{short}.js"));
                // A kernel the package doesn't have is a foreign one.
                let Ok(bytes) = fs::read(&file) else { continue };
                let content = kernel::parse(&self.name, self.foreigns, &bytes)
                    .ok_or_else(|| format!("{} {name}: a kernel that doesn't parse", self.name))?;
                pending.extend(content.imports.iter().map(|i| i.module.clone()));
                built.insert(name, Built::Kernel(content.chunks));
                continue;
            }
            let file = self.src.join(format!("{}.elm", name.replace('.', "/")));
            // A module the package doesn't have is a foreign one.
            let Ok(text) = fs::read_to_string(&file) else {
                continue;
            };
            let parse = time(Phase::Parse, || parse_module(&text));
            let module = time(Phase::Lower, || {
                hir::lower(&ast::Module::cast(parse.syntax()).expect("a module"))
            });
            pending.extend(module.imports.iter().map(|i| i.name.text.clone()));
            modules.insert(name, Read { text, module });
        }
        let mut state: HashMap<String, bool> = HashMap::new();
        let names: Vec<String> = modules.keys().cloned().collect();
        for name in &names {
            self.visit(name, &modules, &mut state, &mut interfaces, &mut built)?;
        }
        Ok((interfaces, built))
    }

    /// Canonicalizes, types and optimizes `name` after the package's
    /// modules it imports.
    fn visit(
        &self,
        name: &str,
        modules: &BTreeMap<String, Read>,
        state: &mut HashMap<String, bool>,
        interfaces: &mut HashMap<String, Interface>,
        built: &mut BTreeMap<String, Built>,
    ) -> Result<(), String> {
        if state.contains_key(name) {
            return Ok(());
        }
        state.insert(name.to_owned(), false);
        let read = &modules[name];
        for import in &read.module.imports {
            if modules.contains_key(&import.name.text) {
                self.visit(&import.name.text, modules, state, interfaces, built)?;
            }
        }
        crate::timings::module(&format!("{} {name}", self.name), || {
            self.compile(name, read, interfaces, built)
        })?;
        state.insert(name.to_owned(), true);
        Ok(())
    }

    /// Canonicalizes, types and optimizes `name`, its imports compiled.
    fn compile(
        &self,
        name: &str,
        read: &Read,
        interfaces: &mut HashMap<String, Interface>,
        built: &mut BTreeMap<String, Built>,
    ) -> Result<(), String> {
        let package = &self.name;
        let canonical = time(Phase::Canonicalize, || {
            canonicalize(self.compat, package, interfaces, &read.module)
        })
        .map_err(|errors| format!("{package} {name}: {errors:?}"))?;
        let annotations = time(Phase::Types, || {
            nomi_compiler::types::infer(self.compat, &canonical)
        })
        .map_err(|errors| format!("{package} {name}: type errors {errors:?}"))?;
        let parse = time(Phase::Parse, || parse_module(&read.text));
        let root = parse.syntax();
        let source = Source::new(&read.text);
        let region = |range| {
            let kept = source.region(trim(&root, range), self.compat);
            Region {
                start: (kept.start.row, kept.start.col),
                end: (kept.end.row, kept.end.col),
            }
        };
        let graph = time(Phase::Optimize, || {
            optimize::module(self.compat, &annotations, &canonical, &region)
        });
        let made = time(Phase::Interface, || interface(&canonical, &annotations));
        interfaces.insert(name.to_owned(), made);
        built.insert(name.to_owned(), Built::Local(graph));
        Ok(())
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
