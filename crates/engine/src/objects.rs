//! The dependencies' optimized graph, as elm writes it to
//! `elm-stuff/0.19.x/o.dat` (`Elm/Details.hs`): each package built from
//! its exposed modules, every module its build reaches optimized and
//! every kernel it imports parsed, then the packages' graphs merged.
//! Upstream merges with left-biased unions folded from the right, so
//! where two graphs share a key, a record field's count mostly, the one
//! first in key order wins: modules by name in the release's order,
//! packages by author then project.

use std::cmp::Ordering;
use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::PathBuf;

use nomi_base::Compat;
use nomi_base::diagnostic::code::Source;
use nomi_codegen_js::kernel;
use nomi_compiler::canonicalize::module::{canonicalize, interface};
use nomi_compiler::hir;
use nomi_compiler::interface::Interface;
use nomi_compiler::optimize;
use nomi_compiler::optimized::{Chunk, GlobalGraph, LocalGraph, Region};
use nomi_project::{Name, Outline};
use nomi_syntax::ast::{self, AstNode};
use nomi_syntax::parse_module;

use crate::names::trim;

/// What a package's build makes of one of its modules (`Details.Result`).
enum Built {
    Local(LocalGraph),
    Kernel(Vec<Chunk>),
}

/// Each package's exposed interfaces and its graph, by its directory.
pub type Memo = HashMap<PathBuf, (HashMap<String, Interface>, GlobalGraph)>;

/// The merged graph of `packages`, each a name and the directory of its
/// source: the whole of what a project's `o.dat` holds.
pub fn dependency_graph(
    compat: Compat,
    packages: &[(Name, PathBuf)],
) -> Result<GlobalGraph, String> {
    dependency_graph_with(compat, packages, &mut Memo::new())
}

/// [`dependency_graph`], reusing and adding to `memo`, as
/// `names::package_interfaces_with` does with interfaces.
pub fn dependency_graph_with(
    compat: Compat,
    packages: &[(Name, PathBuf)],
    memo: &mut Memo,
) -> Result<GlobalGraph, String> {
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
    // Each package's exposed interfaces, dependencies first.
    let mut exposed_interfaces: BTreeMap<Name, HashMap<String, Interface>> = BTreeMap::new();
    let mut graphs: Vec<(Name, GlobalGraph)> = Vec::new();
    let mut pending: Vec<Name> = outlines.keys().cloned().collect();
    while !pending.is_empty() {
        let ready = (pending.iter())
            .position(|name| {
                (outlines[name].0.deps.keys())
                    .all(|d| exposed_interfaces.contains_key(d) || !outlines.contains_key(d))
            })
            .ok_or("the packages depend on each other in a cycle")?;
        let name = pending.remove(ready);
        let (pkg, dir) = &outlines[&name];
        if let Some((exposed, graph)) = memo.get(dir) {
            exposed_interfaces.insert(name.clone(), exposed.clone());
            graphs.push((name, graph.clone()));
            continue;
        }
        let mut visible: HashMap<String, Interface> = HashMap::new();
        let mut foreigns: BTreeMap<String, String> = BTreeMap::new();
        for dep in pkg.deps.keys() {
            if let Some(exposed) = exposed_interfaces.get(dep) {
                for (module, iface) in exposed {
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
        let (interfaces, built) = package.build(&exposed, visible)?;
        let exposed: HashMap<String, Interface> = (interfaces.into_iter())
            .filter(|(m, _)| exposed.contains(m))
            .collect();
        let graph = package.gather(built);
        memo.insert(dir.clone(), (exposed.clone(), graph.clone()));
        exposed_interfaces.insert(name.clone(), exposed);
        graphs.push((name, graph));
    }
    // `Map.foldr addObjects`: the first package by name wins.
    graphs.sort_by(|(a, _), (b, _)| packages_order(a, b));
    let mut merged = GlobalGraph::default();
    for (_, graph) in graphs.into_iter().rev() {
        merged.nodes.extend(graph.nodes);
        merged.fields.extend(graph.fields);
    }
    Ok(merged)
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
            let parse = parse_module(&text);
            let module = hir::lower(&ast::Module::cast(parse.syntax()).expect("a module"));
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
        let package = &self.name;
        let canonical = canonicalize(self.compat, package, interfaces, &read.module)
            .map_err(|errors| format!("{package} {name}: {errors:?}"))?;
        let annotations = nomi_compiler::types::infer(self.compat, &canonical)
            .map_err(|errors| format!("{package} {name}: type errors {errors:?}"))?;
        let parse = parse_module(&read.text);
        let root = parse.syntax();
        let source = Source::new(&read.text);
        let region = |range| {
            let kept = source.region(trim(&root, range), self.compat);
            Region {
                start: (kept.start.row, kept.start.col),
                end: (kept.end.row, kept.end.col),
            }
        };
        let graph = optimize::module(self.compat, &annotations, &canonical, &region);
        interfaces.insert(name.to_owned(), interface(&canonical, &annotations));
        built.insert(name.to_owned(), Built::Local(graph));
        state.insert(name.to_owned(), true);
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
