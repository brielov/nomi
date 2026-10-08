//! Declarations and the spacing between them; upstream's
//! `formatModuleBody`, `formatDeclaration`, and what declarations share
//! with `let`: `formatDefinition` and `formatTypeAnnotation`.

use crate::block::{Block, Line};
use crate::structure::{definition, space_sep_or_indented};
use crate::tree::{
    Assoc, C1Eol, C2Eol, Comment, CommonDeclaration, Declaration, Lower, NameWithArgs,
    OpenCommentedList, Pattern, Ref, TopLevelStructure, Type, Upper,
};

use super::SyntaxContext::{SpaceSeparated, SyntaxSeparated};
use super::expressions::format_expression;
use super::patterns::{format_pattern, pattern_argument};
use super::types::{TypeParensRequired, format_type, type_alone, type_parens};
use super::{
    c2, c2_eol, comment, doc_comment_string, name, pre_commented, syntax_parens, tail_commented,
};

/// What an entry is to the spacing around it; upstream's `BodyEntryType`.
#[derive(Clone, Copy, PartialEq)]
enum Entry<'a> {
    /// A declaration of the name; an annotation and its definition share
    /// one.
    Named(NamedRef<'a>),
    Unnamed,
    Fixity,
}

/// A declared name, with upstream's `Ref ()` equality: variables,
/// operators and types are told apart.
#[derive(Clone, Copy, PartialEq, Eq)]
enum NamedRef<'a> {
    Var(&'a str),
    Tag(&'a str),
    Op(&'a str),
}

/// Upstream's `DeclarationType`.
#[derive(Clone, Copy, PartialEq)]
enum Kind<'a> {
    Comment,
    Starter,
    Closer,
    Definition(Option<NamedRef<'a>>),
    Fixity,
    DocComment,
}

fn entry(decl: &Declaration) -> Entry<'_> {
    match decl {
        Declaration::Common(CommonDeclaration::Definition(pattern, ..)) => match pattern {
            Pattern::Var(var) => Entry::Named(NamedRef::Var(&var.0)),
            Pattern::Op(op) => Entry::Named(NamedRef::Op(&op.0)),
            _ => Entry::Unnamed,
        },
        Declaration::Common(CommonDeclaration::TypeAnnotation(var, _)) => {
            Entry::Named(match &var.value {
                Ref::Var((), n) => NamedRef::Var(&n.0),
                Ref::Tag((), n) => NamedRef::Tag(&n.0),
                Ref::Op(op) => NamedRef::Op(&op.0),
            })
        }
        Declaration::Datatype { name_with_args, .. } => {
            Entry::Named(NamedRef::Tag(&name_with_args.value.name.0))
        }
        Declaration::TypeAlias(_, name_with_args, _) => {
            Entry::Named(NamedRef::Tag(&name_with_args.value.name.0))
        }
        Declaration::PortAnnotation(var, ..) => Entry::Named(NamedRef::Var(&var.value.0)),
        Declaration::Fixity(..) => Entry::Fixity,
    }
}

fn kind<'a>(item: &TopLevelStructure<Entry<'a>>) -> Kind<'a> {
    match item {
        TopLevelStructure::Entry(Entry::Named(name)) => Kind::Definition(Some(*name)),
        TopLevelStructure::Entry(Entry::Unnamed) => Kind::Definition(None),
        TopLevelStructure::Entry(Entry::Fixity) => Kind::Fixity,
        TopLevelStructure::DocComment(_) => Kind::DocComment,
        TopLevelStructure::BodyComment(Comment::TrickOpener) => Kind::Starter,
        TopLevelStructure::BodyComment(Comment::TrickCloser) => Kind::Closer,
        TopLevelStructure::BodyComment(_) => Kind::Comment,
    }
}

/// Blank lines between `a` and `b`, with `lines` between declarations;
/// upstream's `spacer` in `formatTopLevelBody`.
fn spacer(lines: usize, a: Kind, b: Kind) -> usize {
    match (a, b) {
        (Kind::Starter, _) | (_, Kind::Closer) | (Kind::Comment, Kind::Comment) => 0,
        (_, Kind::Comment) => {
            if lines == 1 {
                1
            } else {
                lines + 1
            }
        }
        (Kind::Comment, Kind::Definition(_)) => {
            if lines == 1 {
                0
            } else {
                lines
            }
        }
        (Kind::Comment, _) => lines,
        (Kind::DocComment, Kind::Definition(_)) => 0,
        (Kind::Definition(Some(a)), Kind::Definition(Some(b))) if a == b => 0,
        (Kind::Definition(_), Kind::Definition(_) | Kind::Starter) => lines,
        (Kind::Closer, _) | (_, Kind::DocComment) => lines,
        (Kind::DocComment, Kind::Starter) | (Kind::Fixity, Kind::Fixity) => 0,
        (Kind::Fixity, _) | (_, Kind::Fixity) => lines,
    }
}

/// The module's declarations and the comments between them, `lines`
/// blank lines apart; upstream's `formatModuleBody`.
pub(super) fn body(lines: usize, body: &[TopLevelStructure<Declaration>]) -> Option<Block> {
    let mut blocks = Vec::new();
    let mut previous: Option<Kind> = None;
    for item in body {
        let (current, block) = match item {
            TopLevelStructure::Entry(decl) => (
                kind(&TopLevelStructure::Entry(entry(decl))),
                declaration(decl),
            ),
            TopLevelStructure::DocComment(docs) => {
                // Plan 2.13 formats the Markdown first.
                (Kind::DocComment, doc_comment_string(&docs.0))
            }
            TopLevelStructure::BodyComment(c) => {
                (kind(&TopLevelStructure::BodyComment(c.clone())), comment(c))
            }
        };
        if let Some(previous) = previous {
            blocks.extend(
                std::iter::repeat_with(Block::blank_line).take(spacer(lines, previous, current)),
            );
        }
        blocks.push(block);
        previous = Some(current);
    }
    (!blocks.is_empty()).then(|| Block::stack1(blocks))
}

/// Upstream's `formatDeclaration`.
fn declaration(decl: &Declaration) -> Block {
    match decl {
        Declaration::Common(common_decl) => common(common_decl),
        Declaration::Datatype {
            name_with_args,
            variants,
        } => {
            let mut variants = open_commented_list(variants, variant).into_iter();
            let first = variants.next().expect("a custom type has a variant");
            let variants = first
                .prefix(Line::row([Line::text("="), Line::space()]))
                .and_then(variants.map(|v| v.prefix(Line::row([Line::text("|"), Line::space()]))))
                .indent();
            match c2(name_with_args, name_with_args_block) {
                Block::SingleLine(head) => Block::stack1([
                    Block::line(Line::row([Line::text("type"), Line::space(), head])),
                    variants,
                ]),
                head => Block::stack1([text("type"), head.indent(), variants]),
            }
        }
        Declaration::TypeAlias(pre_alias, name_with_args, ty) => definition(
            "=",
            true,
            text("type"),
            vec![
                pre_commented(pre_alias, text("alias")),
                c2(name_with_args, name_with_args_block),
            ],
            Block::stack1(
                ty.comments
                    .iter()
                    .map(comment)
                    .chain([type_alone(&ty.value)]),
            ),
        ),
        Declaration::PortAnnotation(var, type_comments, ty) => definition(
            ":",
            false,
            text("port"),
            vec![c2(var, |v| Block::line(name(&v.0)))],
            pre_commented(type_comments, type_alone(ty)),
        ),
        Declaration::Fixity(assoc, precedence, op, value) => {
            let assoc_word = match assoc.value {
                Assoc::Left => "left ",
                Assoc::Right => "right",
                Assoc::Non => "non  ",
            };
            space_sep_or_indented(
                text("infix"),
                vec![
                    pre_commented(&assoc.comments, text(assoc_word)),
                    pre_commented(&precedence.comments, text(precedence.value.to_string())),
                    c2(op, |op| text(format!("({})", op.0))),
                    text("="),
                    pre_commented(&value.comments, Block::line(name(&value.value.0))),
                ],
            )
        }
    }
}

fn text(text: impl Into<String>) -> Block {
    Block::line(Line::text(text))
}

/// A custom type's variant; `ctor` in upstream's `formatDeclaration`.
fn variant(variant: &NameWithArgs<Upper, Type>) -> Block {
    let args: Vec<Block> = variant
        .args
        .iter()
        .map(|a| {
            pre_commented(
                &a.comments,
                type_parens(TypeParensRequired::ForCtor, format_type(&a.value)),
            )
        })
        .collect();
    let tag = name(&variant.name.0);
    if args.is_empty() {
        return Block::line(tag);
    }
    match Block::all_singles(args) {
        Ok(args) => Block::line(Line::row(super::spaced(
            std::iter::once(tag).chain(args).collect(),
        ))),
        Err(args) => Block::stack1([Block::line(tag), Block::stack1(args).indent()]),
    }
}

/// Upstream's `formatNameWithArgs`.
fn name_with_args_block(value: &NameWithArgs<Upper, Lower>) -> Block {
    let args = value
        .args
        .iter()
        .map(|a| pre_commented(&a.comments, Block::line(name(&a.value.0))))
        .collect();
    let head = name(&value.name.0);
    match Block::all_singles(args) {
        Ok(args) => Block::line(Line::row(super::spaced(
            std::iter::once(head).chain(args).collect(),
        ))),
        Err(args) => Block::stack1(
            std::iter::once(Block::line(head)).chain(args.into_iter().map(Block::indent)),
        ),
    }
}

/// Upstream's `formatOpenCommentedList`.
fn open_commented_list<T>(list: &OpenCommentedList<T>, render: impl Fn(&T) -> Block) -> Vec<Block> {
    let item = |item: &C2Eol<T>| {
        let (pre, post, eol) = &item.comments;
        c2_eol(pre, post, eol.as_deref(), render(&item.value))
    };
    let last = |last: &C1Eol<T>| {
        let (pre, eol) = &last.comments;
        c2_eol(pre, &[], eol.as_deref(), render(&last.value))
    };
    list.items
        .iter()
        .map(item)
        .chain([last(&list.last)])
        .collect()
}

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

#[cfg(test)]
mod tests;
