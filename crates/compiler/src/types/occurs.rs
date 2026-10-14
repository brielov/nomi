//! The occurs check (`Type/Occurs.hs`): whether a variable's type
//! contains the variable itself. Variables compare as upstream compares
//! its `IORef`s, by identity rather than by set, and an alias's real type
//! isn't looked into.

use super::{AliasReal, Variable, Variables, children};

/// `occurs`.
pub(super) fn occurs(vars: &mut Variables, var: Variable) -> bool {
    help(vars, &mut Vec::new(), var)
}

fn help(vars: &mut Variables, seen: &mut Vec<Variable>, var: Variable) -> bool {
    if seen.contains(&var) {
        return true;
    }
    let children = children(&vars.get(var).content, AliasReal::Skip);
    seen.push(var);
    // Every child is visited, as upstream's fold visits them.
    let mut found = false;
    for child in children {
        found |= nomi_base::grow(|| help(vars, seen, child));
    }
    seen.pop();
    found
}
