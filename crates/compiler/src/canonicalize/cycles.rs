//! The recursion checks (`Module.hs`'s `detectCycles` and
//! `detectBadCycles`, `Expression.hs`'s for `let`). Values that use each
//! other must be functions: a cycle through a value that isn't is
//! reported, in 0.19.1 and 0.19.2 with the rest of its component, and in
//! 0.19.3 with the shortest cycle back to the component's first node
//! (`Graph.withMinimalCycle`).

use std::collections::HashMap;

use nomi_base::{Compat, TextRange};

use super::error::{Error, R, all, throw};
use super::expr::{FreeLocals, LetBinding, LetNode, Use};
use crate::canonical::{Decl, Def, Expr, ExprKind};
use crate::graph::{self, Node, Scc, strongly_connected};
use crate::hir::Spanned;

/// A top-level value as the checks see it: its definition, and the
/// locals and top-level values it uses.
pub(super) struct TopNode {
    pub(super) def: Def,
    pub(super) free: FreeLocals,
}

/// `detectCycles` over the module's values: components by every use,
/// then within each cyclic one, components by direct uses alone, each of
/// which must be acyclic. The first component with a bad cycle stops the
/// rest. The declarations, a value before those that use it.
pub(super) fn top_level(compat: Compat, nodes: Vec<TopNode>) -> R<Vec<Decl>> {
    let order = |a: &String, b: &String| compat.compare_names(a, b);
    let graph = (nodes.into_iter())
        .map(|node| Node {
            key: node.def.name().to_owned(),
            edges: compat.sorted_names(node.free.keys().cloned()),
            value: node,
        })
        .collect();
    let mut decls = Vec::new();
    for scc in strongly_connected(graph, order) {
        let members = match scc {
            Scc::Acyclic(node) => {
                decls.push(Decl::Declare(node.def));
                continue;
            }
            Scc::Cyclic(members) => members,
        };
        let direct = (members.into_iter())
            .map(|node| {
                // `toNodeTwo`: a function's uses wait until it is called.
                let edges = if node.def.has_params() {
                    Vec::new()
                } else {
                    let direct = (node.free.iter())
                        .filter(|(_, uses)| **uses == Use::Direct)
                        .map(|(name, _)| name.clone());
                    compat.sorted_names(direct)
                };
                Node {
                    key: node.def.name().to_owned(),
                    edges: edges.clone(),
                    value: (node.def, edges),
                }
            })
            .collect();
        let sccs = strongly_connected(direct, order);
        let mut defs = all(sccs.into_iter().map(|scc| bad_cycle(compat, scc)))?.into_iter();
        if let Some(first) = defs.next() {
            decls.push(Decl::DeclareRec(first, defs.collect()));
        }
    }
    Ok(decls)
}

/// `detectBadCycles`.
fn bad_cycle(compat: Compat, scc: Scc<(Def, Vec<String>)>) -> R<Def> {
    let members = match scc {
        Scc::Acyclic((def, _)) => return Ok(def),
        Scc::Cyclic(members) => members,
    };
    let others = match compat {
        Compat::V0_19_1 | Compat::V0_19_2 => (members[1..].iter())
            .map(|(def, _)| def.name().to_owned())
            .collect(),
        Compat::V0_19_3 => {
            let keyed = members.iter().map(|(def, edges)| (def.name(), &edges[..]));
            (minimal_cycle(keyed).into_iter())
                .map(|i| members[i].0.name().to_owned())
                .collect()
        }
    };
    let root = &members[0].0;
    throw(Error::RecursiveDecl {
        range: root.region(),
        name: root.name().to_owned(),
        others,
    })
}

/// `withMinimalCycle` over a component, each member by its key and the
/// keys it uses: the shortest way back to the first member, as the places
/// of the members after it. Where the first uses only itself, upstream
/// reads outside its array; nomi reports it alone.
fn minimal_cycle<'k>(component: impl Iterator<Item = (&'k str, &'k [String])>) -> Vec<usize> {
    let component: Vec<_> = component.collect();
    let index: HashMap<&str, usize> = (component.iter().enumerate())
        .map(|(i, (key, _))| (*key, i))
        .collect();
    let edges: Vec<Vec<usize>> = (component.iter())
        .map(|(_, keys)| {
            keys.iter()
                .filter_map(|k| index.get(k.as_str()).copied())
                .collect()
        })
        .collect();
    let members: Vec<usize> = (0..component.len()).collect();
    graph::minimal_cycle(&edges, &members).split_off(1)
}

/// `detectCycles` over a `let`'s graph: every cyclic component must hold
/// only functions; each that doesn't is reported, and the rest still
/// checked. The body under the bindings, each component a `let` at the
/// `let`'s region, a binding before those that use it.
pub(super) fn let_cycles(
    compat: Compat,
    region: TextRange,
    nodes: Vec<LetNode>,
    body: Expr,
) -> R<Expr> {
    // Upstream prepends each node as it goes, so its list runs last
    // first; that orders the nodes whose keys tie, as every `_ =` does.
    let graph = (nodes.into_iter().rev())
        .map(|node| {
            let uses = compat.sorted_names(node.uses);
            Node {
                key: node.key.clone(),
                edges: uses.clone(),
                value: LetNode { uses, ..node },
            }
        })
        .collect();
    let order = |a: &String, b: &String| compat.compare_names(a, b);
    let steps = all(strongly_connected(graph, order)
        .into_iter()
        .map(|scc| match scc {
            Scc::Acyclic(node) => Ok(Step::One(Box::new(node.binding))),
            Scc::Cyclic(nodes) => check_cycle(compat, nodes).map(Step::Rec),
        }))?;
    let wrap = |kind| Expr { region, kind };
    Ok(steps.into_iter().rev().fold(body, |body, step| match step {
        Step::One(binding) => match *binding {
            LetBinding::Define(def) => wrap(ExprKind::Let(Box::new(def), Box::new(body))),
            LetBinding::Edge(_) => body,
            LetBinding::Destruct(pattern, value) => wrap(ExprKind::LetDestruct {
                pattern,
                value: Box::new(value),
                body: Box::new(body),
            }),
        },
        Step::Rec(defs) => wrap(ExprKind::LetRec(defs, Box::new(body))),
    }))
}

/// A component of a `let`'s graph, checked.
enum Step {
    One(Box<LetBinding>),
    Rec(Vec<Def>),
}

/// `checkCycle`: the first binding of the component that isn't a
/// function is reported, with the others; or the functions, last first,
/// as upstream gathers them.
fn check_cycle(compat: Compat, nodes: Vec<LetNode>) -> R<Vec<Def>> {
    let bad = (nodes.iter().enumerate()).find_map(|(i, node)| match &node.binding {
        LetBinding::Define(def) if !def.has_params() => {
            let name = Spanned {
                text: def.name().to_owned(),
                range: def.region(),
            };
            Some((i, name))
        }
        LetBinding::Edge(name) => Some((i, name.clone())),
        LetBinding::Define(_) | LetBinding::Destruct(..) => None,
    });
    let Some((i, name)) = bad else {
        let defs = nodes
            .into_iter()
            .rev()
            .filter_map(|node| match node.binding {
                LetBinding::Define(def) => Some(def),
                LetBinding::Edge(_) | LetBinding::Destruct(..) => None,
            });
        return Ok(defs.collect());
    };
    let others = match compat {
        // The names after it, then the functions before it.
        Compat::V0_19_1 | Compat::V0_19_2 => (nodes[i + 1..].iter())
            .filter_map(|node| binding_name(&node.binding))
            .chain(nodes[..i].iter().filter_map(|node| match &node.binding {
                LetBinding::Define(def) => Some(def.name().to_owned()),
                LetBinding::Edge(_) | LetBinding::Destruct(..) => None,
            }))
            .collect(),
        Compat::V0_19_3 => {
            let keyed = (nodes.iter()).map(|node| (node.key.as_str(), &node.uses[..]));
            (minimal_cycle(keyed).into_iter())
                .filter_map(|i| binding_name(&nodes[i].binding))
                .collect()
        }
    };
    throw(Error::RecursiveLet { name, others })
}

/// `toName`.
fn binding_name(binding: &LetBinding) -> Option<String> {
    match binding {
        LetBinding::Define(def) => Some(def.name().to_owned()),
        LetBinding::Edge(name) => Some(name.text.clone()),
        LetBinding::Destruct(..) => None,
    }
}
