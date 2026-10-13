//! Canonicalizing patterns (`Canonicalize/Pattern.hs`): constructors
//! resolved and their arity checked, and the names a pattern binds
//! gathered, in the order upstream's `Result` threads them, to be checked
//! for duplicates once the pattern is otherwise sound.

use std::collections::BTreeMap;

use nomi_base::{Compat, TextRange, grow_large};

use super::dups::Dict;
use super::env::{CtorInfo, Env};
use super::error::{ArityContext, DuplicatePatternContext, Error, R, all, both, throw};
use crate::canonical::{Pattern, PatternCtorArg, PatternKind};
use crate::hir::{self, PatId, int_value};

/// The names a pattern binds so far (`Pattern.DupsDict`).
pub(super) type Bindings = Dict<TextRange>;

/// `verify`: what `work` makes while binding names, and those names, each
/// bound once. A pattern that fails has only its own errors: duplicates
/// are checked after.
pub(super) fn verify<T>(
    compat: Compat,
    context: &DuplicatePatternContext,
    work: impl FnOnce(&mut Bindings) -> R<T>,
) -> R<(T, BTreeMap<String, TextRange>)> {
    let mut bindings = Bindings::default();
    let value = work(&mut bindings)?;
    let bound = bindings.detect(compat, |name, first, second| Error::DuplicatePattern {
        context: context.clone(),
        name: name.to_owned(),
        first,
        second,
    })?;
    Ok((value, bound))
}

/// `canonicalize`.
pub(super) fn canonicalize(
    env: &Env<'_>,
    module: &hir::Module,
    id: PatId,
    bindings: &mut Bindings,
) -> R<Pattern> {
    grow_large(|| canonicalize_(env, module, id, bindings))
}

/// Each of `ids`, all of them even past an error.
pub(super) fn each(
    env: &Env<'_>,
    module: &hir::Module,
    ids: &[PatId],
    bindings: &mut Bindings,
) -> R<Vec<Pattern>> {
    all(ids
        .iter()
        .map(|&id| canonicalize(env, module, id, bindings)))
}

fn canonicalize_(
    env: &Env<'_>,
    module: &hir::Module,
    id: PatId,
    bindings: &mut Bindings,
) -> R<Pattern> {
    let region = module.source_map.pattern_range(id);
    let kind = match &module.patterns[id] {
        // A module with syntax errors canonicalizes for the editor only.
        hir::Pattern::Missing | hir::Pattern::Anything => PatternKind::Anything,
        hir::Pattern::Unit => PatternKind::Unit,
        hir::Pattern::Chr(text) => PatternKind::Chr(text.clone()),
        hir::Pattern::Str(text) => PatternKind::Str(text.clone()),
        hir::Pattern::Int(text) => PatternKind::Int(int_value(text)),
        hir::Pattern::Var(name) => {
            bindings.insert(env.compat, name, region, region);
            PatternKind::Var(name.clone())
        }
        hir::Pattern::Record(fields) => {
            // `logFields` adds them last first.
            for field in fields.iter().rev() {
                bindings.insert(env.compat, &field.text, field.range, field.range);
            }
            PatternKind::Record(fields.iter().map(|f| f.text.clone()).collect())
        }
        hir::Pattern::Tuple(items) => {
            let a = canonicalize(env, module, items[0], bindings);
            let b = canonicalize(env, module, items[1], bindings);
            let rest = match &items[2..] {
                [] => Ok(None),
                [c] => canonicalize(env, module, *c, bindings).map(Some),
                _ => throw(Error::TupleLargerThanThree { range: region }),
            };
            let ((a, b), c) = both(both(a, b), rest)?;
            PatternKind::Tuple(Box::new(a), Box::new(b), c.map(Box::new))
        }
        hir::Pattern::Ctor {
            qualifier,
            name,
            args,
        } => {
            let ctor = env.find_ctor(name.range, qualifier.as_deref(), &name.text)?;
            match ctor {
                CtorInfo::Ctor {
                    home,
                    union_name,
                    union,
                    index,
                    args: types,
                } => {
                    if args.len() != types.len() {
                        return throw(Error::BadArity {
                            range: region,
                            context: ArityContext::Pattern,
                            name: name.text.clone(),
                            expected: types.len(),
                            actual: args.len(),
                        });
                    }
                    let cargs = all((args.iter().zip(types.iter()).enumerate()).map(
                        |(index, (&arg, ty))| {
                            canonicalize(env, module, arg, bindings).map(|arg| PatternCtorArg {
                                index,
                                ty: ty.clone(),
                                arg,
                            })
                        },
                    ))?;
                    if union_name == "Bool" && home.is("elm/core", "Basics") {
                        PatternKind::Bool(union.clone().into_owned(), name.text == "True")
                    } else {
                        PatternKind::Ctor {
                            home: home.clone(),
                            type_name: union_name.clone(),
                            union: union.clone().into_owned(),
                            name: name.text.clone(),
                            index: *index,
                            args: cargs,
                        }
                    }
                }
                CtorInfo::Record { .. } => {
                    return throw(Error::PatternHasRecordCtor {
                        range: region,
                        name: name.text.clone(),
                    });
                }
            }
        }
        hir::Pattern::List(items) => PatternKind::List(each(env, module, items, bindings)?),
        hir::Pattern::Cons(head, tail) => {
            let head = canonicalize(env, module, *head, bindings);
            let tail = canonicalize(env, module, *tail, bindings);
            let (head, tail) = both(head, tail)?;
            PatternKind::Cons(Box::new(head), Box::new(tail))
        }
        hir::Pattern::Alias(inner, name) => {
            let inner = canonicalize(env, module, *inner, bindings)?;
            bindings.insert(env.compat, &name.text, name.range, name.range);
            PatternKind::Alias(Box::new(inner), name.text.clone())
        }
    };
    Ok(Pattern { region, kind })
}
