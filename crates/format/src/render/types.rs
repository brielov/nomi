//! Types; upstream's `formatType` and the parentheses it decides on.

use crate::block::{Block, Line};
use crate::structure::{
    application, forceable_space_sep_or_stack, forceable_space_sep_or_stack1, group,
    prefix_or_indented,
};
use crate::tree::{ApplicationMultiline, ForceMultiline, Multiline, Type, TypeConstructor};

use super::{
    c2, c2_eol, comments, eol_commented, name, pair, parens, pre_commented, record_like, unit,
    upper_ref,
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
        TypeConstructor::Tuple(n) => Line::text(format!("({})", ",".repeat(n - 1))),
    })
}

/// Upstream's `formatType`.
pub(super) fn format_type(ty: &Type) -> (TypeParensInner, Block) {
    crate::guarded(|| format_type_unguarded(ty))
}

fn format_type_unguarded(ty: &Type) -> (TypeParensInner, Block) {
    use TypeParensRequired::{ForCtor, ForLambda};
    match ty {
        Type::Unit(comments) => (TypeParensInner::NotNeeded, unit('(', ')', comments)),
        Type::Function {
            first,
            rest,
            force_multiline: ForceMultiline(force),
        } => {
            let first = eol_commented(
                first.comments.as_deref(),
                type_parens(ForLambda, format_type(&first.value)),
            );
            let rest = rest
                .0
                .iter()
                .map(|item| {
                    let (pre_op, post_op, eol) = &item.comments;
                    let term = c2_eol(
                        post_op,
                        &[],
                        eol.as_deref(),
                        type_parens(ForLambda, format_type(&item.value)),
                    );
                    let arrow = prefix_or_indented(Block::line(Line::text("->")), term);
                    forceable_space_sep_or_stack1(
                        false,
                        comments(pre_op).into_iter().chain([arrow]).collect(),
                    )
                })
                .collect();
            (
                TypeParensInner::ForFunctionType,
                forceable_space_sep_or_stack(*force, first, rest),
            )
        }
        Type::Variable(var) => (TypeParensInner::NotNeeded, Block::line(name(&var.0))),
        Type::Construction(ctor, args, ForceMultiline(force)) => {
            let join = if *force {
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
        Type::Tuple(types, ForceMultiline(force)) => {
            let types = types
                .iter()
                .map(|t| {
                    let (pre, post, eol) = &t.comments;
                    c2_eol(pre, post, eol.as_deref(), type_alone(&t.value))
                })
                .collect();
            (
                TypeParensInner::NotNeeded,
                group(true, "(", ",", ")", *force, types),
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
                pair(":", field, |key| name(&key.0), type_alone)
            }),
        ),
    }
}

#[cfg(test)]
mod tests;
