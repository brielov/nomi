//! Types; upstream's `formatType` and the parentheses it decides on.

use crate::tree::{ApplicationMultiline, Multiline, Type, TypeConstructor};

use super::block::{Block, Line};
use super::structure::{
    Delimiters, application, forceable_space_sep_or_stack, group, prefix_or_indented,
};
use super::{
    c2, c2_eol, eol_commented, name, pair, parens, pre_commented, record_like, tuple_function,
    unit, upper_ref,
};

/// Where a type stands, from needing no parentheses to needing them most;
/// upstream's `TypeParensRequired`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum TypeParensRequired {
    NotRequired,
    /// An operand of `->`.
    ForLambda,
    /// A type constructor's argument.
    ForCtor,
}

/// What a type needs parentheses for; upstream's `TypeParensInner`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum TypeParensInner {
    NotNeeded,
    ForFunctionType,
    ForTypeConstruction,
}

/// `block` in parentheses when, as `inner`, it needs them where `outer`
/// stands; upstream's `typeParens`.
pub(super) fn type_parens(
    outer: TypeParensRequired,
    (inner, block): (TypeParensInner, Block),
) -> Block {
    let needed = match inner {
        TypeParensInner::NotNeeded => false,
        TypeParensInner::ForTypeConstruction => outer >= TypeParensRequired::ForCtor,
        TypeParensInner::ForFunctionType => outer >= TypeParensRequired::ForLambda,
    };
    if needed { parens(block) } else { block }
}

/// `ty` where nothing around it calls for parentheses.
pub(super) fn type_alone(ty: &Type) -> Block {
    type_parens(TypeParensRequired::NotRequired, format_type(ty))
}

/// Upstream's `formatTypeConstructor`.
fn type_constructor(ctor: &TypeConstructor) -> Block {
    Block::line(match ctor {
        TypeConstructor::Named(name) => upper_ref(name),
        TypeConstructor::Tuple(n) => tuple_function(*n),
    })
}

/// Upstream's `formatType`.
pub(super) fn format_type(ty: &Type) -> (TypeParensInner, Block) {
    nomi_base::grow_large(|| format_type_unguarded(ty))
}

fn format_type_unguarded(ty: &Type) -> (TypeParensInner, Block) {
    use TypeParensRequired::{ForCtor, ForLambda};
    match ty {
        Type::Unit(comments) => (TypeParensInner::NotNeeded, unit('(', ')', comments)),
        Type::Function {
            first,
            rest,
            force_multiline,
        } => {
            let first = eol_commented(
                first.comments.as_deref(),
                type_parens(ForLambda, format_type(&first.value)),
            );
            let rest = rest
                .0
                .iter()
                .map(|item| {
                    let around = &item.comments;
                    let term = c2_eol(
                        &around.after_separator,
                        &[],
                        around.eol.as_deref(),
                        type_parens(ForLambda, format_type(&item.value)),
                    );
                    let arrow = prefix_or_indented(Block::line(Line::text("->")), term);
                    pre_commented(&around.before_separator, arrow)
                })
                .collect();
            (
                TypeParensInner::ForFunctionType,
                forceable_space_sep_or_stack(*force_multiline, first, rest),
            )
        }
        Type::Variable(var) => (TypeParensInner::NotNeeded, Block::line(name(&var.0))),
        Type::Construction {
            ctor,
            args,
            force_multiline,
        } => {
            let join = if force_multiline.0 {
                ApplicationMultiline::SplitFirst
            } else {
                ApplicationMultiline::JoinFirst(Multiline::JoinAll)
            };
            let inner = if args.is_empty() {
                TypeParensInner::NotNeeded
            } else {
                TypeParensInner::ForTypeConstruction
            };
            let args = args
                .iter()
                .map(|a| pre_commented(&a.comments, type_parens(ForCtor, format_type(&a.value))))
                .collect();
            (inner, application(join, type_constructor(ctor), args))
        }
        Type::Parens(inner) => (TypeParensInner::NotNeeded, parens(c2(inner, type_alone))),
        Type::Tuple(types, force) => {
            let types = types
                .iter()
                .map(|t| {
                    let around = &t.comments;
                    c2_eol(
                        &around.before,
                        &around.after,
                        around.eol.as_deref(),
                        type_alone(&t.value),
                    )
                })
                .collect();
            (
                TypeParensInner::NotNeeded,
                group(Delimiters::PADDED_PARENS, *force, types),
            )
        }
        Type::Record {
            base,
            fields,
            trailing,
            force_multiline,
        } => (
            TypeParensInner::NotNeeded,
            record_like(base.as_ref(), fields, trailing, *force_multiline, |field| {
                pair(":", field, type_alone)
            }),
        ),
    }
}

#[cfg(test)]
mod tests;
