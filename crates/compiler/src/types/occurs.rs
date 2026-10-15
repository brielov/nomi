//! The occurs check (`Type/Occurs.hs`): whether a variable's type
//! contains the variable itself. Variables compare as upstream compares
//! its `IORef`s, by identity rather than by set, and an alias's real type
//! isn't looked into.

use super::{AliasReal, Variable, Variables, children};

/// `occurs`.
pub(super) fn occurs(vars: &mut Variables, var: Variable) -> bool {
    occurs_help(vars, &mut Vec::new(), &mut Vec::new(), var)
}

/// `stack` holds each call's children above its caller's.
fn occurs_help(
    vars: &mut Variables,
    seen: &mut Vec<Variable>,
    stack: &mut Vec<Variable>,
    var: Variable,
) -> bool {
    if seen.contains(&var) {
        return true;
    }
    let from = stack.len();
    children(&vars.get(var).content, AliasReal::Skip, stack);
    seen.push(var);
    // Every child is visited, as upstream's fold visits them.
    let mut found = false;
    for i in from..stack.len() {
        let child = stack[i];
        found |= nomi_base::grow(|| occurs_help(vars, seen, stack, child));
    }
    stack.truncate(from);
    seen.pop();
    found
}
