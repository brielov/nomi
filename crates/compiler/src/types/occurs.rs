//! The occurs check (`Type/Occurs.hs`): whether a variable's type
//! contains the variable itself. Variables compare as upstream compares
//! its `IORef`s, by identity rather than by set, and an alias's real type
//! isn't looked into.

use super::{Content, FlatType, Variable, Variables};

/// `occurs`.
pub fn occurs(vars: &mut Variables, var: Variable) -> bool {
    help(vars, &mut Vec::new(), var)
}

fn help(vars: &mut Variables, seen: &mut Vec<Variable>, var: Variable) -> bool {
    if seen.contains(&var) {
        return true;
    }
    let children: Vec<Variable> = match vars.content(var) {
        Content::FlexVar(_)
        | Content::FlexSuper(..)
        | Content::RigidVar(_)
        | Content::RigidSuper(..)
        | Content::Error => return false,
        Content::Structure(flat) => match flat {
            FlatType::App1(_, _, args) => args,
            FlatType::Fun1(a, b) => vec![a, b],
            FlatType::EmptyRecord1 | FlatType::Unit1 => return false,
            FlatType::Record1(fields, ext) => {
                let mut children: Vec<Variable> = fields.into_values().collect();
                children.push(ext);
                children
            }
            FlatType::Tuple1(a, b, c) => [Some(a), Some(b), c].into_iter().flatten().collect(),
        },
        Content::Alias(_, _, args, _) => args.into_iter().map(|(_, a)| a).collect(),
    };
    seen.push(var);
    // Every child is visited, as upstream's fold visits them.
    let mut found = false;
    for child in children {
        found |= stacker::maybe_grow(64 * 1024, 1024 * 1024, || help(vars, seen, child));
    }
    seen.pop();
    found
}
