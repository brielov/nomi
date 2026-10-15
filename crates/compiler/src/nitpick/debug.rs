//! `Debug` uses (`Nitpick/Debug.hs`), which `--optimize` refuses: it
//! strips what `Debug.toString` reads, and may move `Debug.log` calls.
//! Upstream looks through the optimized graph, which keeps every
//! top-level definition.

use crate::optimized;

/// `hasDebugUses` on a module's optimized graph, as `elm make
/// --optimize` asks it before generating code.
pub fn graph_has_debug_uses(graph: &optimized::LocalGraph) -> bool {
    use optimized::Node;
    graph.nodes.values().any(|node| match node {
        Node::Define(expr, _)
        | Node::DefineTailFunc { body: expr, .. }
        | Node::PortIncoming(expr, _)
        | Node::PortOutgoing(expr, _) => opt_has_debug(expr),
        Node::Cycle {
            values, functions, ..
        } => {
            values.iter().any(|(_, e)| opt_has_debug(e)) || functions.iter().any(opt_def_has_debug)
        }
        Node::Ctor(..)
        | Node::Enum(_)
        | Node::Box
        | Node::Link(_)
        | Node::Manager(_)
        | Node::Kernel(..) => false,
    })
}

fn opt_def_has_debug(def: &optimized::Def) -> bool {
    match def {
        optimized::Def::Def(_, e) | optimized::Def::TailDef { body: e, .. } => opt_has_debug(e),
    }
}

/// `hasDebug` on an optimized expression.
fn opt_has_debug(expr: &optimized::Expr) -> bool {
    use optimized::Expr as E;
    nomi_base::grow(|| match expr {
        E::VarDebug { .. } => true,
        E::Bool(_)
        | E::Chr(_)
        | E::Str(_)
        | E::Int(_)
        | E::Float(_)
        | E::VarLocal(_)
        | E::VarGlobal(_)
        | E::VarEnum(..)
        | E::VarBox(_)
        | E::VarCycle(..)
        | E::VarKernel { .. }
        | E::Accessor(_)
        | E::Unit
        | E::Shader { .. } => false,
        E::List(items) => items.iter().any(opt_has_debug),
        E::Function(_, body) | E::Destruct(_, body) | E::Access(body, _) => opt_has_debug(body),
        E::Call(f, args) => opt_has_debug(f) || args.iter().any(opt_has_debug),
        E::TailCall(_, args) => args.iter().any(|(_, e)| opt_has_debug(e)),
        E::If(branches, last) => {
            branches
                .iter()
                .any(|(c, e)| opt_has_debug(c) || opt_has_debug(e))
                || opt_has_debug(last)
        }
        E::Let(def, body) => opt_def_has_debug(def) || opt_has_debug(body),
        E::Case { decider, jumps, .. } => {
            decider.any_inline(&opt_has_debug) || jumps.iter().any(|(_, e)| opt_has_debug(e))
        }
        E::Update(record, fields) => opt_has_debug(record) || fields.values().any(opt_has_debug),
        E::Record(fields) => fields.values().any(opt_has_debug),
        E::Tuple(a, b, c) => {
            opt_has_debug(a) || opt_has_debug(b) || c.as_deref().is_some_and(opt_has_debug)
        }
    })
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use nomi_base::Compat;
    use nomi_base::diagnostic::code::{Position, Region};

    use super::*;
    use crate::optimize;
    use crate::testing::canonical;

    #[test]
    fn debug_uses_are_found_anywhere_in_a_body() {
        let has_debug = |text: &str| {
            let module = canonical(text).unwrap();
            let at = Position { row: 1, col: 1 };
            let region = |_| Region { start: at, end: at };
            let graph = optimize::module(Compat::V0_19_1, &BTreeMap::new(), &module, &region);
            graph_has_debug_uses(&graph)
        };
        assert!(has_debug(
            "module Main exposing (..)\n\n\nf x =\n    let\n        y = ( x, Debug.todo \"y\" )\n    in\n    y\n"
        ));
        assert!(!has_debug(
            "module Main exposing (..)\n\n\nf x =\n    ( x, identity x )\n"
        ));
    }
}
