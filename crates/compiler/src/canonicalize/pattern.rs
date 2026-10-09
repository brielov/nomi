//! Canonicalizing patterns (`Canonicalize/Pattern.hs`): constructors
//! resolved and their arity checked, and the names a pattern binds
//! gathered, in the order upstream's `Result` threads them, to be checked
//! for duplicates once the pattern is otherwise sound.

use std::collections::BTreeMap;

use nomi_base::TextRange;

use super::dups::Dict;
use super::env::{CtorInfo, Env};
use super::error::{ArityContext, DuplicatePatternContext, Error, R, both, throw};
use crate::hir::{self, PatId, Pattern};

/// The names a pattern binds so far (`Pattern.DupsDict`).
pub type Bindings = Dict<TextRange>;

/// `verify`: what `work` makes while binding names, and those names, each
/// bound once. A pattern that fails has only its own errors: duplicates
/// are checked after.
pub fn verify<T>(
    env: &Env<'_>,
    context: DuplicatePatternContext,
    work: impl FnOnce(&mut Bindings) -> R<T>,
) -> R<(T, BTreeMap<String, TextRange>)> {
    let mut bindings = Bindings::default();
    let value = work(&mut bindings)?;
    let bound = bindings.detect(env.compat, |name, first, second| Error::DuplicatePattern {
        context: context.clone(),
        name: name.to_owned(),
        first,
        second,
    })?;
    Ok((value, bound))
}

fn range(module: &hir::Module, id: PatId) -> TextRange {
    module
        .source_map
        .pattern(id)
        .map(|p| p.range)
        .unwrap_or_default()
}

/// `canonicalize`.
pub fn canonicalize(
    env: &Env<'_>,
    module: &hir::Module,
    id: PatId,
    bindings: &mut Bindings,
) -> R<()> {
    stacker::maybe_grow(128 * 1024, 2 * 1024 * 1024, || {
        canonicalize_(env, module, id, bindings)
    })
}

fn canonicalize_(env: &Env<'_>, module: &hir::Module, id: PatId, bindings: &mut Bindings) -> R<()> {
    let region = range(module, id);
    match &module.patterns[id] {
        Pattern::Missing
        | Pattern::Anything
        | Pattern::Unit
        | Pattern::Chr(_)
        | Pattern::Str(_)
        | Pattern::Int(_) => Ok(()),
        Pattern::Var(name) => {
            bindings.insert(env.compat, name, region, region);
            Ok(())
        }
        Pattern::Record(fields) => {
            // `logFields` adds them last first.
            for field in fields.iter().rev() {
                bindings.insert(env.compat, &field.text, field.range, field.range);
            }
            Ok(())
        }
        Pattern::Tuple(items) => {
            let a = canonicalize(env, module, items[0], bindings);
            let b = canonicalize(env, module, items[1], bindings);
            let rest = match &items[2..] {
                [] => Ok(()),
                [c] => canonicalize(env, module, *c, bindings),
                _ => throw(Error::TupleLargerThanThree { range: region }),
            };
            both(both(a, b), rest).map(|_| ())
        }
        Pattern::Ctor {
            qualifier,
            name,
            args,
        } => {
            let ctor = env.find_ctor(name.range, qualifier.as_deref(), &name.text)?;
            match ctor {
                CtorInfo::Ctor { args: types, .. } => {
                    if args.len() == types.len() {
                        let mut result = Ok(());
                        for &arg in args {
                            let arg = canonicalize(env, module, arg, bindings);
                            result = both(result, arg).map(|_| ());
                        }
                        result
                    } else {
                        throw(Error::BadArity {
                            range: region,
                            context: ArityContext::Pattern,
                            name: name.text.clone(),
                            expected: types.len(),
                            actual: args.len(),
                        })
                    }
                }
                CtorInfo::Record { .. } => throw(Error::PatternHasRecordCtor {
                    range: region,
                    name: name.text.clone(),
                }),
            }
        }
        Pattern::List(items) => {
            let mut result = Ok(());
            for &item in items {
                let item = canonicalize(env, module, item, bindings);
                result = both(result, item).map(|_| ());
            }
            result
        }
        Pattern::Cons(head, tail) => {
            let head = canonicalize(env, module, *head, bindings);
            let tail = canonicalize(env, module, *tail, bindings);
            both(head, tail).map(|_| ())
        }
        Pattern::Alias(inner, name) => {
            canonicalize(env, module, *inner, bindings)?;
            bindings.insert(env.compat, &name.text, name.range, name.range);
            Ok(())
        }
    }
}
