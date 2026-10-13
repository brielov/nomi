//! `Debug` uses (`Nitpick/Debug.hs`), which `--optimize` refuses: it
//! strips what `Debug.toString` reads, and may move `Debug.log` calls.
//! Upstream looks through the optimized graph, which keeps every
//! top-level definition; the canonical one holds the same expressions.

use crate::canonical::{Decl, Def, Expr, ExprKind};
use crate::optimized;

/// `hasDebugUses`: whether any of the module's values uses `Debug`.
pub fn has_debug_uses(decls: &[Decl]) -> bool {
    decls.iter().any(|decl| match decl {
        Decl::Declare(def) => def_has_debug(def),
        Decl::DeclareRec(def, defs) => def_has_debug(def) || defs.iter().any(def_has_debug),
    })
}

fn def_has_debug(def: &Def) -> bool {
    match def {
        Def::Def { body, .. } | Def::TypedDef { body, .. } => has_debug(body),
    }
}

/// `hasDebug`.
fn has_debug(expr: &Expr) -> bool {
    stacker::maybe_grow(64 * 1024, 1024 * 1024, || match &expr.kind {
        ExprKind::VarDebug { .. } => true,
        ExprKind::List(items) => items.iter().any(has_debug),
        ExprKind::Negate(e) | ExprKind::Access { record: e, .. } => has_debug(e),
        ExprKind::Binop { left, right, .. } => has_debug(left) || has_debug(right),
        ExprKind::Lambda(_, body) => has_debug(body),
        ExprKind::Call(f, args) => has_debug(f) || args.iter().any(has_debug),
        ExprKind::If(branches, otherwise) => {
            branches.iter().any(|(c, e)| has_debug(c) || has_debug(e)) || has_debug(otherwise)
        }
        ExprKind::Let(def, body) => def_has_debug(def) || has_debug(body),
        ExprKind::LetRec(defs, body) => defs.iter().any(def_has_debug) || has_debug(body),
        ExprKind::LetDestruct { value, body, .. } => has_debug(value) || has_debug(body),
        ExprKind::Case(scrutinee, branches) => {
            has_debug(scrutinee) || branches.iter().any(|(_, e)| has_debug(e))
        }
        ExprKind::Update { record, fields, .. } => {
            has_debug(record) || fields.values().any(|f| has_debug(&f.value))
        }
        ExprKind::Record(fields) => fields.values().any(has_debug),
        ExprKind::Tuple(a, b, c) => {
            has_debug(a) || has_debug(b) || c.as_deref().is_some_and(has_debug)
        }
        ExprKind::VarLocal(_)
        | ExprKind::VarTopLevel(..)
        | ExprKind::VarKernel { .. }
        | ExprKind::VarForeign { .. }
        | ExprKind::VarCtor { .. }
        | ExprKind::VarOperator(_)
        | ExprKind::Chr(_)
        | ExprKind::Str(_)
        | ExprKind::Int(_)
        | ExprKind::Float(_)
        | ExprKind::Accessor(_)
        | ExprKind::Unit
        | ExprKind::Shader(..) => false,
    })
}

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
    stacker::maybe_grow(64 * 1024, 1024 * 1024, || match expr {
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
            decider_has_debug(decider) || jumps.iter().any(|(_, e)| opt_has_debug(e))
        }
        E::Update(record, fields) => opt_has_debug(record) || fields.values().any(opt_has_debug),
        E::Record(fields) => fields.values().any(opt_has_debug),
        E::Tuple(a, b, c) => {
            opt_has_debug(a) || opt_has_debug(b) || c.as_deref().is_some_and(opt_has_debug)
        }
    })
}

fn decider_has_debug(decider: &optimized::Decider<optimized::Choice>) -> bool {
    use optimized::{Choice, Decider};
    match decider {
        Decider::Leaf(Choice::Inline(e)) => opt_has_debug(e),
        Decider::Leaf(Choice::Jump(_)) => false,
        Decider::Chain {
            success, failure, ..
        } => decider_has_debug(success) || decider_has_debug(failure),
        Decider::FanOut {
            tests, fallback, ..
        } => tests.iter().any(|(_, d)| decider_has_debug(d)) || decider_has_debug(fallback),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::canonical;

    #[test]
    fn debug_uses_are_found_anywhere_in_a_body() {
        let decls = |text: &str| canonical(text).unwrap().decls;
        assert!(has_debug_uses(&decls(
            "module Main exposing (..)\n\n\nf x =\n    let\n        y = ( x, Debug.todo \"y\" )\n    in\n    y\n"
        )));
        assert!(!has_debug_uses(&decls(
            "module Main exposing (..)\n\n\nf x =\n    ( x, identity x )\n"
        )));
    }
}
