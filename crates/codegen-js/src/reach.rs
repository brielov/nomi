//! What a program reaches: the edges generation follows from each node,
//! and a breadth-first walk over them from the `main`s that keeps how
//! each global was first reached, which availability errors show.

use std::collections::{BTreeSet, HashMap, VecDeque};

use nomi_base::Compat;
use nomi_compiler::canonical::Home;
use nomi_compiler::optimized::binary::globals;
use nomi_compiler::optimized::{EffectsType, Global, GlobalGraph, Main, Node};

use crate::mode::Mode;
use crate::platform::Platform;

/// What a node needs written before it.
pub(crate) enum Deps<'a> {
    None,
    Set(&'a BTreeSet<Global>),
    /// A link's target.
    Link(&'a Global),
    /// `Basics.identity`, which a box is in production.
    Identity,
    /// The functions of its home's effect manager.
    Manager(EffectsType),
}

/// What `node`, `global`'s definition, needs; `None` when it isn't
/// written at all, as the debugger's kernel isn't outside `--debug`.
pub(crate) fn deps_of<'a>(debug: bool, global: &Global, node: &'a Node) -> Option<Deps<'a>> {
    Some(match node {
        Node::Define(_, deps)
        | Node::DefineTailFunc { deps, .. }
        | Node::Cycle { deps, .. }
        | Node::PortIncoming(_, deps)
        | Node::PortOutgoing(_, deps) => Deps::Set(deps),
        Node::Kernel(_, deps) => {
            if global.home.module == "Debugger" && !debug {
                return None;
            }
            Deps::Set(deps)
        }
        Node::Link(linked) => Deps::Link(linked),
        Node::Box => Deps::Identity,
        Node::Manager(effects) => Deps::Manager(*effects),
        Node::Ctor(..) | Node::Enum(_) => Deps::None,
    })
}

impl Deps<'_> {
    /// Calls `f` on each, a set's in the release's order of globals.
    pub(crate) fn each(&self, compat: Compat, home: &Home, mut f: impl FnMut(&Global)) {
        match self {
            Deps::None => {}
            Deps::Set(deps) => {
                let mut deps: Vec<&Global> = deps.iter().collect();
                deps.sort_by(|a, b| globals(compat, a, b));
                for dep in deps {
                    f(dep);
                }
            }
            Deps::Link(linked) => f(linked),
            Deps::Identity => f(&identity()),
            Deps::Manager(effects) => {
                for name in manager_functions(*effects) {
                    f(&Global {
                        home: home.clone(),
                        name: (*name).to_owned(),
                    });
                }
            }
        }
    }
}

/// `Basics.identity`.
pub(crate) fn identity() -> Global {
    Global {
        home: Home::core("Basics"),
        name: "identity".to_owned(),
    }
}

/// The functions an effect manager of `effects` defines, in the order
/// `createManager` takes them.
pub(crate) fn manager_functions(effects: EffectsType) -> &'static [&'static str] {
    match effects {
        EffectsType::Cmd => &["init", "onEffects", "onSelfMsg", "cmdMap"],
        EffectsType::Sub => &["init", "onEffects", "onSelfMsg", "subMap"],
        EffectsType::Fx => &["init", "onEffects", "onSelfMsg", "cmdMap", "subMap"],
    }
}

/// What a program reaches, in the order a breadth-first walk from its
/// `main`s found it, each with the global it was first reached from.
#[derive(Debug)]
pub struct Reached<'a> {
    pub order: Vec<&'a Global>,
    parents: HashMap<&'a Global, Option<&'a Global>>,
}

impl<'a> Reached<'a> {
    /// The globals from a `main` to `global`, each reaching the next.
    pub fn path(&self, global: &'a Global) -> Vec<&'a Global> {
        let mut path = vec![global];
        let mut at = global;
        while let Some(Some(parent)) = self.parents.get(at) {
            path.push(parent);
            at = parent;
        }
        path.reverse();
        path
    }
}

/// `reach`: what `mains` reach in `graph`, written as `debug` says.
pub fn reach<'a>(
    compat: Compat,
    debug: bool,
    graph: &'a GlobalGraph,
    mains: &[(Home, Main)],
) -> Reached<'a> {
    let mut parents = HashMap::new();
    let mut order = Vec::new();
    let mut queue = VecDeque::new();
    let visit = |parents: &mut HashMap<_, _>,
                 queue: &mut VecDeque<_>,
                 global: &Global,
                 parent: Option<&'a Global>| {
        if let Some((global, node)) = graph.nodes.get_key_value(global)
            && !parents.contains_key(global)
        {
            parents.insert(global, parent);
            queue.push_back((global, node));
        }
    };
    for (home, _) in mains {
        let main = Global {
            home: home.clone(),
            name: "main".to_owned(),
        };
        visit(&mut parents, &mut queue, &main, None);
    }
    while let Some((global, node)) = queue.pop_front() {
        let Some(deps) = deps_of(debug, global, node) else {
            continue;
        };
        order.push(global);
        deps.each(compat, &global.home, |dep| {
            visit(&mut parents, &mut queue, dep, Some(global));
        });
    }
    Reached { order, parents }
}

/// What `mains` reach in `graph` that `platform` lacks: the path to the
/// kernel found first, from its `main`. The browser lacks nothing yet,
/// so it is not walked.
pub fn unavailable(
    compat: Compat,
    mode: &Mode,
    platform: Platform,
    graph: &GlobalGraph,
    mains: &[(Home, Main)],
) -> Option<Vec<Global>> {
    unavailable_of(
        compat,
        mode.is_debug(),
        platform.unavailable(),
        graph,
        mains,
    )
}

/// What `mains` reach of the kernel modules `lacking`, by short name.
fn unavailable_of(
    compat: Compat,
    debug: bool,
    lacking: &[&str],
    graph: &GlobalGraph,
    mains: &[(Home, Main)],
) -> Option<Vec<Global>> {
    if lacking.is_empty() {
        return None;
    }
    let reached = reach(compat, debug, graph, mains);
    let kernel = reached.order.iter().find(|global| {
        global.home.package == "elm/kernel" && lacking.contains(&&*global.home.module)
    })?;
    Some(reached.path(kernel).into_iter().cloned().collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use nomi_compiler::optimized::{Chunk, Expr};

    fn global(module: &str, name: &str) -> Global {
        Global {
            home: Home::author_project(module),
            name: name.to_owned(),
        }
    }

    fn kernel(short: &str) -> Global {
        Global::kernel(Compat::V0_19_1, short)
    }

    fn define(deps: &[Global]) -> Node {
        Node::Define(Expr::Int(0), deps.iter().cloned().collect())
    }

    /// `Main.main` needs `Page.view` and `Page.title`; the view a link to
    /// `Html.div`, which needs the `VirtualDom` kernel; `Page.unused`
    /// needs the `Nomi` kernel, which nothing reaches.
    fn graph() -> GlobalGraph {
        let mut graph = GlobalGraph::default();
        let nodes = [
            (
                global("Main", "main"),
                define(&[global("Page", "view"), global("Page", "title")]),
            ),
            (global("Page", "title"), define(&[])),
            (global("Page", "view"), Node::Link(global("Html", "div"))),
            (global("Html", "div"), define(&[kernel("VirtualDom")])),
            (global("Page", "unused"), define(&[kernel("Nomi")])),
            (
                kernel("VirtualDom"),
                Node::Kernel(vec![Chunk::Js(b"".to_vec())], [kernel("Utils")].into()),
            ),
            (kernel("Utils"), Node::Kernel(Vec::new(), BTreeSet::new())),
            (kernel("Nomi"), Node::Kernel(Vec::new(), BTreeSet::new())),
        ];
        graph.nodes.extend(nodes);
        graph
    }

    fn mains() -> Vec<(Home, Main)> {
        vec![(Home::author_project("Main"), Main::Static)]
    }

    #[test]
    fn each_global_reached_keeps_the_chain_from_its_main() {
        let graph = graph();
        let mains = mains();
        let reached = reach(Compat::V0_19_1, false, &graph, &mains);
        let utils = kernel("Utils");
        assert_eq!(
            reached.path(&utils),
            [
                &global("Main", "main"),
                &global("Page", "view"),
                &global("Html", "div"),
                &kernel("VirtualDom"),
                &utils,
            ]
        );
        assert_eq!(reached.path(reached.order[0]), [&global("Main", "main")]);
        assert!(!reached.order.contains(&&global("Page", "unused")));
        assert_eq!(reached.order.len(), 6);
    }

    #[test]
    fn the_debugger_kernel_is_reached_only_under_debug() {
        let mut graph = graph();
        let debugger = kernel("Debugger");
        graph.nodes.insert(
            debugger.clone(),
            Node::Kernel(Vec::new(), [kernel("Nomi")].into()),
        );
        graph
            .nodes
            .insert(global("Page", "title"), define(&[debugger]));
        let mains = mains();
        let reaches = |debug| {
            reach(Compat::V0_19_1, debug, &graph, &mains)
                .order
                .contains(&&kernel("Nomi"))
        };
        assert!(!reaches(false));
        assert!(reaches(true));
    }

    #[test]
    fn a_lacking_kernel_reached_is_found_with_its_chain() {
        let graph = graph();
        let mains = mains();
        let lacking =
            |table: &[&str]| unavailable_of(Compat::V0_19_1, false, table, &graph, &mains);
        assert_eq!(
            lacking(Platform::Node.unavailable()),
            Some(vec![
                global("Main", "main"),
                global("Page", "view"),
                global("Html", "div"),
                kernel("VirtualDom"),
            ])
        );
        // A program for the browser that defines, but never calls, what
        // needs a kernel only Node has.
        assert_eq!(lacking(&["Nomi"]), None);
        assert_eq!(lacking(Platform::Browser.unavailable()), None);
    }
}
