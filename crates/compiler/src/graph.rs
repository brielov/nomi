//! `Data.Graph`'s strongly connected components, in the order upstream
//! lists them, which decides which cycle a module or a declaration
//! reports first; and 0.19.3's minimal cycle within one.

use std::collections::hash_map::Entry;
use std::collections::{HashMap, HashSet, VecDeque};

use nomi_base::Compat;

/// `Data.Graph.SCC`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Scc<T> {
    Acyclic(T),
    /// The vertices of a cycle, in the order the search met them.
    Cyclic(Vec<T>),
}

/// A vertex of a graph `stronglyConnComp` reads: a value, its key, and
/// the keys it points to.
pub(crate) struct Node<T> {
    pub(crate) value: T,
    pub(crate) key: String,
    pub(crate) edges: Vec<String>,
}

/// `stronglyConnComp`: `nodes` as components, the ones others point to
/// first. Vertices are ordered by key as `compat` orders names, as
/// `graphFromEdges` sorts them, and keys no node has are dropped.
pub(crate) fn strongly_connected<T>(compat: Compat, mut nodes: Vec<Node<T>>) -> Vec<Scc<Node<T>>> {
    nodes.sort_by(|a, b| compat.compare_names(&a.key, &b.key));
    let vertex =
        |key: &String| (nodes.binary_search_by(|n| compat.compare_names(&n.key, key))).ok();
    let edges: Vec<Vec<usize>> = (nodes.iter())
        .map(|n| n.edges.iter().filter_map(vertex).collect())
        .collect();
    let mut nodes: Vec<Option<Node<T>>> = nodes.into_iter().map(Some).collect();
    components(&edges)
        .into_iter()
        .map(|component| {
            let first = component[0];
            let mut take = |v: usize| nodes[v].take().expect("each vertex once");
            if component.len() == 1 && !edges[first].contains(&first) {
                Scc::Acyclic(take(first))
            } else {
                Scc::Cyclic(component.into_iter().map(take).collect())
            }
        })
        .collect()
}

/// `Data.Graph.scc`: the components as trees of a depth-first search of
/// the graph, taking roots in reverse postorder of a search of its
/// transpose; each in preorder, as `stronglyConnComp` lists them.
pub fn components(edges: &[Vec<usize>]) -> Vec<Vec<usize>> {
    // `transposeG` builds with `accumArray (flip (:))`, so each vertex's
    // edges come back reversed.
    let mut transposed = vec![Vec::new(); edges.len()];
    for (v, ws) in edges.iter().enumerate() {
        for &w in ws {
            transposed[w].push(v);
        }
    }
    for vs in &mut transposed {
        vs.reverse();
    }
    let (_, postorder) = dfs(&transposed, 0..edges.len());
    let (trees, _) = dfs(edges, postorder.into_iter().rev());
    trees
}

/// `Data.Graph.dfs` from each of `roots` not yet reached: each tree's
/// vertices in preorder, and every vertex in postorder.
fn dfs(
    edges: &[Vec<usize>],
    roots: impl IntoIterator<Item = usize>,
) -> (Vec<Vec<usize>>, Vec<usize>) {
    let mut seen = vec![false; edges.len()];
    let mut trees = Vec::new();
    let mut postorder = Vec::with_capacity(edges.len());
    let mut stack: Vec<(usize, usize)> = Vec::new();
    for root in roots {
        if seen[root] {
            continue;
        }
        seen[root] = true;
        let mut tree = vec![root];
        stack.push((root, 0));
        while let Some(&(v, next)) = stack.last() {
            if let Some(&w) = edges[v].get(next) {
                stack.last_mut().expect("the stack has v").1 += 1;
                if !seen[w] {
                    seen[w] = true;
                    tree.push(w);
                    stack.push((w, 0));
                }
            } else {
                postorder.push(v);
                stack.pop();
            }
        }
        trees.push(tree);
    }
    (trees, postorder)
}

/// 0.19.3's `withMinimalCycle`: a breadth-first search from the
/// component's first module, over edges inside the component, to the
/// first module dequeued that imports it; each module is reached from the
/// first to find it.
///
/// Where the first module imports itself, upstream reads its path from
/// outside its array, and crashes or hangs; nomi reports that module alone.
pub fn minimal_cycle(edges: &[Vec<usize>], component: &[usize]) -> Vec<usize> {
    let root = component[0];
    let members: HashSet<usize> = component.iter().copied().collect();
    let mut back: HashMap<usize, usize> = HashMap::new();
    let mut queue = VecDeque::from([root]);
    let last = loop {
        let i = queue
            .pop_front()
            .expect("every module of a component reaches its first");
        let inside = edges[i].iter().copied().filter(|j| members.contains(j));
        if inside.clone().any(|j| j == root) {
            break i;
        }
        for j in inside {
            if let Entry::Vacant(entry) = back.entry(j) {
                entry.insert(i);
                queue.push_back(j);
            }
        }
    };
    let mut cycle = Vec::new();
    let mut i = last;
    while i != root {
        cycle.push(i);
        i = back[&i];
    }
    cycle.push(root);
    cycle.reverse();
    cycle
}
