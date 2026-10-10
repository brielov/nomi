//! `Debug` uses (`Nitpick/Debug.hs`), which `--optimize` refuses: it
//! strips what `Debug.toString` reads, and may move `Debug.log` calls.
//! Upstream looks through the optimized graph, which keeps every
//! top-level definition; the canonical one holds the same expressions.

use crate::canonical::{Decl, Def, Expr, ExprKind};

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
        ExprKind::VarDebug(..) => true,
        ExprKind::List(items) => items.iter().any(has_debug),
        ExprKind::Negate(e) | ExprKind::Access(e, _, _) => has_debug(e),
        ExprKind::Binop(_, _, _, _, a, b) => has_debug(a) || has_debug(b),
        ExprKind::Lambda(_, body) => has_debug(body),
        ExprKind::Call(f, args) => has_debug(f) || args.iter().any(has_debug),
        ExprKind::If(branches, otherwise) => {
            branches.iter().any(|(c, e)| has_debug(c) || has_debug(e)) || has_debug(otherwise)
        }
        ExprKind::Let(def, body) => def_has_debug(def) || has_debug(body),
        ExprKind::LetRec(defs, body) => defs.iter().any(def_has_debug) || has_debug(body),
        ExprKind::LetDestruct(_, value, body) => has_debug(value) || has_debug(body),
        ExprKind::Case(scrutinee, branches) => {
            has_debug(scrutinee) || branches.iter().any(|(_, e)| has_debug(e))
        }
        ExprKind::Update(_, record, fields) => {
            has_debug(record) || fields.values().any(|f| has_debug(&f.value))
        }
        ExprKind::Record(fields) => fields.values().any(has_debug),
        ExprKind::Tuple(a, b, c) => {
            has_debug(a) || has_debug(b) || c.as_deref().is_some_and(has_debug)
        }
        ExprKind::VarLocal(_)
        | ExprKind::VarTopLevel(..)
        | ExprKind::VarKernel(..)
        | ExprKind::VarForeign(..)
        | ExprKind::VarCtor(..)
        | ExprKind::VarOperator(..)
        | ExprKind::Chr(_)
        | ExprKind::Str(_)
        | ExprKind::Int(_)
        | ExprKind::Float(_)
        | ExprKind::Accessor(_)
        | ExprKind::Unit
        | ExprKind::Shader(..) => false,
    })
}
