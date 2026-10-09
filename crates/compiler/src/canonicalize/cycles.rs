//! The recursion checks (`Module.hs`'s `detectCycles` and
//! `detectBadCycles`, `Expression.hs`'s for `let`). Values that use each
//! other must be functions: a cycle through a value that isn't is
//! reported, in 0.19.1 and 0.19.2 with the rest of its component, and in
//! 0.19.3 with the shortest cycle back to the component's first node
//! (`Graph.withMinimalCycle`).

use std::collections::{HashMap, VecDeque};

use nomi_base::{Compat, TextRange};

use super::error::{At, Error, R, all};
use super::expr::{FreeLocals, LetBinding, LetNode};
use crate::graph::{Scc, strongly_connected};

/// A top-level value as the checks see it.
#[derive(Clone, Debug)]
pub struct TopNode {
    pub name: String,
    pub range: TextRange,
    pub has_params: bool,
    pub free: FreeLocals,
}

/// `Map.keys`, in `compat`'s name order.
fn keys(compat: Compat, free: &FreeLocals) -> Vec<String> {
    super::sorted(compat, free.keys())
        .into_iter()
        .cloned()
        .collect()
}

/// `detectCycles` over the module's values: components by every use,
/// then within each cyclic one, components by direct uses alone, each of
/// which must be acyclic. The first component with a bad cycle stops the
/// rest.
pub fn top_level(compat: Compat, nodes: Vec<TopNode>) -> R<()> {
    let order = super::name_order(compat);
    let graph = (nodes.into_iter())
        .map(|node| {
            let edges = keys(compat, &node.free);
            let name = node.name.clone();
            (node, name, edges)
        })
        .collect();
    for scc in strongly_connected(graph, |a: &String, b: &String| order(a, b)) {
        let Scc::Cyclic(members) = scc else { continue };
        let direct = (members.into_iter())
            .map(|node| {
                // `toNodeTwo`: a function's uses wait until it is called.
                let edges = if node.has_params {
                    Vec::new()
                } else {
                    let direct: FreeLocals = (node.free.iter())
                        .filter(|(_, uses)| uses.direct > 0)
                        .map(|(n, u)| (n.clone(), *u))
                        .collect();
                    keys(compat, &direct)
                };
                let name = node.name.clone();
                ((node, edges.clone()), name, edges)
            })
            .collect();
        let sccs = strongly_connected(direct, |a: &String, b: &String| order(a, b));
        all(sccs.into_iter().map(|scc| bad_cycle(compat, scc)))?;
    }
    Ok(())
}

/// `detectBadCycles`.
fn bad_cycle(compat: Compat, scc: Scc<(TopNode, Vec<String>)>) -> R<()> {
    let Scc::Cyclic(members) = scc else {
        return Ok(());
    };
    let (root, _) = &members[0];
    let others = match compat {
        Compat::V0_19_1 | Compat::V0_19_2 => {
            members[1..].iter().map(|(n, _)| n.name.clone()).collect()
        }
        Compat::V0_19_3 => {
            let keyed: Vec<(String, Vec<String>)> = members
                .iter()
                .map(|(n, edges)| (n.name.clone(), edges.clone()))
                .collect();
            minimal_cycle(compat, &keyed)
        }
    };
    Err(vec![Error::RecursiveDecl {
        range: root.range,
        name: root.name.clone(),
        others,
    }])
}

/// `withMinimalCycle`: from the component's first node, the shortest way
/// back to it, by a breadth-first search over the component's edges, as
/// the keys after the first. Where the first node uses only itself,
/// upstream reads outside its array; nomi reports it alone.
fn minimal_cycle(compat: Compat, component: &[(String, Vec<String>)]) -> Vec<String> {
    let order = super::name_order(compat);
    let mut sorted: Vec<&(String, Vec<String>)> = component.iter().collect();
    sorted.sort_by(|a, b| order(&a.0, &b.0));
    let index: HashMap<&str, usize> = sorted
        .iter()
        .enumerate()
        .map(|(i, (k, _))| (k.as_str(), i))
        .collect();
    let edges: Vec<Vec<usize>> = (sorted.iter())
        .map(|(_, ks)| {
            ks.iter()
                .filter_map(|k| index.get(k.as_str()).copied())
                .collect()
        })
        .collect();
    let root = index[component[0].0.as_str()];
    let mut back: Vec<Option<usize>> = vec![None; sorted.len()];
    let mut queue = VecDeque::from([root]);
    let last = loop {
        let Some(i) = queue.pop_front() else {
            return Vec::new();
        };
        if edges[i].contains(&root) {
            break i;
        }
        for &j in &edges[i] {
            if back[j].is_none() {
                back[j] = Some(i);
                queue.push_back(j);
            }
        }
    };
    let mut path = Vec::new();
    let mut i = last;
    loop {
        if i == root && path.is_empty() {
            return Vec::new();
        }
        path.push(sorted[i].0.clone());
        match back[i] {
            Some(j) if j == root => break,
            Some(j) => i = j,
            None => break,
        }
    }
    path.reverse();
    path
}

/// `detectCycles` over a `let`'s graph: every cyclic component must hold
/// only functions; each that doesn't is reported, and the rest still
/// checked.
pub fn let_cycles(compat: Compat, nodes: Vec<LetNode>) -> R<()> {
    let order = super::name_order(compat);
    let graph = (nodes.into_iter())
        .map(|(binding, key, edges)| {
            let edges: Vec<String> = super::sorted(compat, edges.iter())
                .into_iter()
                .cloned()
                .collect();
            ((binding, key.clone(), edges.clone()), key, edges)
        })
        .collect();
    let sccs = strongly_connected(graph, |a: &String, b: &String| order(a, b));
    all(sccs.into_iter().map(|scc| match scc {
        Scc::Acyclic(_) => Ok(()),
        Scc::Cyclic(bindings) => check_cycle(compat, &bindings),
    }))
    .map(|_| ())
}

/// `checkCycle`: the first binding of the component that isn't a
/// function is reported, with the others.
fn check_cycle(compat: Compat, bindings: &[(LetBinding, String, Vec<String>)]) -> R<()> {
    let mut defs: Vec<&str> = Vec::new();
    for (i, (binding, _, _)) in bindings.iter().enumerate() {
        let name = match binding {
            LetBinding::Define {
                name,
                range,
                has_params: false,
            }
            | LetBinding::Edge { name, range } => At {
                name: name.clone(),
                range: *range,
            },
            LetBinding::Define { name, .. } => {
                defs.push(name);
                continue;
            }
            LetBinding::Destruct => continue,
        };
        let others = match compat {
            Compat::V0_19_1 | Compat::V0_19_2 => {
                let mut others: Vec<String> = bindings[i + 1..]
                    .iter()
                    .filter_map(|(b, _, _)| binding_name(b))
                    .collect();
                others.extend(defs.iter().map(|d| (*d).to_owned()));
                others
            }
            Compat::V0_19_3 => {
                let keyed: Vec<(String, Vec<String>)> = bindings
                    .iter()
                    .map(|(_, key, edges)| (key.clone(), edges.clone()))
                    .collect();
                let names: HashMap<&str, Option<String>> = bindings
                    .iter()
                    .map(|(b, key, _)| (key.as_str(), binding_name(b)))
                    .collect();
                (minimal_cycle(compat, &keyed).iter())
                    .filter_map(|key| names.get(key.as_str()).cloned().flatten())
                    .collect()
            }
        };
        return Err(vec![Error::RecursiveLet { name, others }]);
    }
    Ok(())
}

/// `toName`.
fn binding_name(binding: &LetBinding) -> Option<String> {
    match binding {
        LetBinding::Define { name, .. } | LetBinding::Edge { name, .. } => Some(name.clone()),
        LetBinding::Destruct => None,
    }
}
