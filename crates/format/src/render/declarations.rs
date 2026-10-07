//! Declarations; upstream's `formatDeclaration` and what it shares with
//! `let`: `formatDefinition` and `formatTypeAnnotation`.

use crate::block::{Block, Line};
use crate::structure::definition;
use crate::tree::{CommonDeclaration, Ref};

use super::SyntaxContext::{SpaceSeparated, SyntaxSeparated};
use super::expressions::format_expression;
use super::patterns::{format_pattern, pattern_argument};
use super::types::type_alone;
use super::{comment, name, pre_commented, syntax_parens, tail_commented};

/// A definition or a type annotation, at the top level or in a `let`.
pub(super) fn common(decl: &CommonDeclaration) -> Block {
    match decl {
        CommonDeclaration::Definition(pattern, args, comments, body) => {
            let body = Block::stack1(
                comments
                    .iter()
                    .map(comment)
                    .chain([syntax_parens(SyntaxSeparated, format_expression(body))]),
            );
            definition(
                "=",
                true,
                syntax_parens(SpaceSeparated, format_pattern(pattern)),
                args.iter()
                    .map(|a| pre_commented(&a.comments, pattern_argument(&a.value)))
                    .collect(),
                body,
            )
        }
        CommonDeclaration::TypeAnnotation(var, ty) => {
            let var_name = match &var.value {
                Ref::Var((), n) => name(&n.0),
                Ref::Tag((), n) => name(&n.0),
                Ref::Op(op) => Line::text(format!("({})", op.0)),
            };
            definition(
                ":",
                false,
                tail_commented(Block::line(var_name), &var.comments),
                Vec::new(),
                pre_commented(&ty.comments, type_alone(&ty.value)),
            )
        }
    }
}
