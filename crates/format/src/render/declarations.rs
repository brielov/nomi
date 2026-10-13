//! Declarations and the spacing between them; upstream's
//! `formatModuleBody`, `formatDeclaration`, and what declarations share
//! with `let`: `formatDefinition` and `formatTypeAnnotation`.

use crate::tree::{
    AnnotatedName, Assoc, C0Eol, C1, C1Eol, C2, C2Eol, Comment, CommonDeclaration, Declaration,
    Docs, Expr, Lower, NameWithArgs, OpenCommentedList, Pattern, Symbol, TopLevelStructure, Type,
};

use super::block::{Block, Line};
use super::expressions::expression_alone;
use super::patterns::pattern_argument;
use super::structure::{DefinitionKind, definition, intersperse, space_sep_or_indented};
use super::types::{TypeParensRequired, format_type, type_alone, type_parens};
use super::{
    c2, c2_eol, comment, doc_comment, eol_commented, name, operator, please_report, pre_commented,
    tail_commented, text,
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
        Declaration::Common(CommonDeclaration::Definition { name, .. }) => match name {
            Pattern::Var(var) => Entry::Named(NamedRef::Var(&var.0)),
            Pattern::Op(op) => Entry::Named(NamedRef::Op(&op.0)),
            Pattern::Anything
            | Pattern::Unit(_)
            | Pattern::Literal(_)
            | Pattern::Data(..)
            | Pattern::Parens(_)
            | Pattern::Tuple(_)
            | Pattern::EmptyList(_)
            | Pattern::List(_)
            | Pattern::Cons { .. }
            | Pattern::EmptyRecord(_)
            | Pattern::Record(_)
            | Pattern::Alias(..) => Entry::Unnamed,
        },
        Declaration::Common(CommonDeclaration::TypeAnnotation { name, .. }) => {
            Entry::Named(match &name.value {
                AnnotatedName::Var(n) => NamedRef::Var(&n.0),
                AnnotatedName::Op(op) => NamedRef::Op(&op.0),
            })
        }
        Declaration::Datatype { name_with_args, .. }
        | Declaration::TypeAlias { name_with_args, .. } => {
            Entry::Named(NamedRef::Tag(&name_with_args.value.name.0))
        }
        Declaration::PortAnnotation { name, .. } | Declaration::PortDefinition { name, .. } => {
            Entry::Named(NamedRef::Var(&name.value.0))
        }
        Declaration::Fixity { .. } | Declaration::LegacyFixity { .. } => Entry::Fixity,
    }
}

/// A part of a top-level body: an entry, already formatted, or what
/// comes between entries.
enum Item<'a> {
    Entry(Entry<'a>, Block),
    DocComment(&'a Docs),
    Comment(&'a Comment),
}

fn item<'a, T>(
    item: &'a TopLevelStructure<T>,
    entry: impl FnOnce(&'a T) -> (Entry<'a>, Block),
) -> Item<'a> {
    match item {
        TopLevelStructure::Entry(e) => {
            let (e, block) = entry(e);
            Item::Entry(e, block)
        }
        TopLevelStructure::DocComment(docs) => Item::DocComment(docs),
        TopLevelStructure::BodyComment(c) => Item::Comment(c),
    }
}

fn kind<'a>(item: &Item<'a>) -> Kind<'a> {
    match item {
        Item::Entry(Entry::Named(name), _) => Kind::Definition(Some(*name)),
        Item::Entry(Entry::Unnamed, _) => Kind::Definition(None),
        Item::Entry(Entry::Fixity, _) => Kind::Fixity,
        Item::DocComment(_) => Kind::DocComment,
        Item::Comment(Comment::TrickOpener) => Kind::Starter,
        Item::Comment(Comment::TrickCloser) => Kind::Closer,
        Item::Comment(Comment::Block(_) | Comment::Line(_) | Comment::TrickBlock(_)) => {
            Kind::Comment
        }
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
    top_level_body(
        lines,
        body.iter()
            .map(|i| item(i, |decl| (entry(decl), declaration(decl)))),
    )
}

/// The expressions of a code block and the comments between them, as
/// entries of no name; `formatDocComment`'s `ExpressionsCode`.
pub(super) fn expressions(lines: usize, exprs: &[TopLevelStructure<C0Eol<Expr>>]) -> Option<Block> {
    top_level_body(
        lines,
        exprs.iter().map(|i| {
            item(i, |expr| {
                let block = expression_alone(&expr.value);
                (
                    Entry::Unnamed,
                    eol_commented(expr.comments.as_deref(), block),
                )
            })
        }),
    )
}

/// Entries and what comes between them, `lines` blank lines apart;
/// upstream's `formatTopLevelBody`.
fn top_level_body<'a>(lines: usize, body: impl Iterator<Item = Item<'a>>) -> Option<Block> {
    let mut blocks = Vec::new();
    let mut previous: Option<Kind> = None;
    for item in body {
        let current = kind(&item);
        let block = match item {
            Item::Entry(_, block) => block,
            Item::DocComment(docs) => doc_comment(docs),
            Item::Comment(c) => comment(c),
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
        } => datatype(name_with_args, variants),
        Declaration::TypeAlias {
            before_alias,
            name_with_args,
            ty,
        } => definition(
            DefinitionKind::Value,
            text("type"),
            vec![
                pre_commented(before_alias, text("alias")),
                c2(name_with_args, name_with_args_block),
            ],
            Block::stack1(
                ty.comments
                    .iter()
                    .map(comment)
                    .chain([type_alone(&ty.value)]),
            ),
        ),
        Declaration::PortAnnotation {
            name: port,
            before_type,
            ty,
        } => definition(
            DefinitionKind::Annotation,
            text("port"),
            vec![c2(port, |v| Block::line(name(&v.0)))],
            pre_commented(before_type, type_alone(ty)),
        ),
        Declaration::PortDefinition {
            name: port,
            before_body,
            body,
        } => definition(
            DefinitionKind::Value,
            text("port"),
            vec![c2(port, |v| Block::line(name(&v.0)))],
            pre_commented(before_body, expression_alone(body)),
        ),
        Declaration::Fixity {
            assoc,
            precedence,
            op,
            function,
        } => fixity(assoc, precedence, op, function),
        Declaration::LegacyFixity {
            assoc,
            before_precedence,
            precedence,
            before_op,
            op,
        } => legacy_fixity(*assoc, before_precedence, *precedence, before_op, op),
    }
}

/// `type Name args = variants`.
fn datatype(name_with_args: &C2<NameWithArgs<Lower>>, variants: &OpenCommentedList) -> Block {
    let mut variants = open_commented_list(variants).into_iter();
    let first = variants.next().expect("a custom type has a variant");
    let variants = first
        .prefix(&Line::row([Line::text("="), Line::space()]))
        .and_then(variants.map(|v| v.prefix(&Line::row([Line::text("|"), Line::space()]))))
        .indent();
    match c2(name_with_args, name_with_args_block) {
        Block::SingleLine(head) => Block::stack1([
            Block::line(Line::row([Line::text("type"), Line::space(), head])),
            variants,
        ]),
        head => Block::stack1([text("type"), head.indent(), variants]),
    }
}

/// `infix left 6 (+) = add`, the associativity padded to one width.
fn fixity(assoc: &C1<Assoc>, precedence: &C1<i64>, op: &C2<Symbol>, function: &C1<Lower>) -> Block {
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
            c2(op, |op| Block::line(operator(op))),
            text("="),
            pre_commented(&function.comments, Block::line(name(&function.value.0))),
        ],
    )
}

/// Elm 0.18's `infixl 6 +`, which upstream can print only on one line.
fn legacy_fixity(
    assoc: Assoc,
    before_precedence: &[Comment],
    precedence: i64,
    before_op: &[Comment],
    op: &Symbol,
) -> Block {
    let keyword = match assoc {
        Assoc::Left => "infixl",
        Assoc::Right => "infixr",
        Assoc::Non => "infix",
    };
    match (
        pre_commented(before_op, text(op.0.as_str())),
        pre_commented(before_precedence, text(precedence.to_string())),
    ) {
        (Block::SingleLine(op), Block::SingleLine(precedence)) => Block::line(Line::row([
            Line::text(keyword),
            Line::space(),
            precedence,
            Line::space(),
            op,
        ])),
        _ => please_report("TODO", "multiline fixity declaration"),
    }
}

/// `head args`, on one line if they all fit, else the arguments below,
/// each indented.
fn name_and_args(head: Line, args: Vec<Block>) -> Block {
    match Block::all_singles(args) {
        Ok(args) => Block::line(intersperse(std::iter::once(head).chain(args), Line::space)),
        Err(args) => Block::stack1(
            std::iter::once(Block::line(head)).chain(args.into_iter().map(Block::indent)),
        ),
    }
}

/// A custom type's variant; `ctor` in upstream's `formatDeclaration`.
fn variant(variant: &NameWithArgs<Type>) -> Block {
    let args = variant
        .args
        .iter()
        .map(|a| {
            pre_commented(
                &a.comments,
                type_parens(TypeParensRequired::ForCtor, format_type(&a.value)),
            )
        })
        .collect();
    name_and_args(name(&variant.name.0), args)
}

/// Upstream's `formatNameWithArgs`.
fn name_with_args_block(value: &NameWithArgs<Lower>) -> Block {
    let args = value
        .args
        .iter()
        .map(|a| pre_commented(&a.comments, Block::line(name(&a.value.0))))
        .collect();
    name_and_args(name(&value.name.0), args)
}

/// The variants of a custom type; upstream's `formatOpenCommentedList`.
fn open_commented_list(list: &OpenCommentedList) -> Vec<Block> {
    let item = |item: &C2Eol<NameWithArgs<Type>>| {
        let around = &item.comments;
        c2_eol(
            &around.before,
            &around.after,
            around.eol.as_deref(),
            variant(&item.value),
        )
    };
    let last = |last: &C1Eol<NameWithArgs<Type>>| {
        let before = &last.comments;
        c2_eol(
            &before.before,
            &[],
            before.eol.as_deref(),
            variant(&last.value),
        )
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
        CommonDeclaration::Definition {
            name,
            args,
            before_body,
            body,
        } => {
            let body = Block::stack1(
                before_body
                    .iter()
                    .map(comment)
                    .chain([expression_alone(body)]),
            );
            definition(
                DefinitionKind::Value,
                pattern_argument(name),
                args.iter()
                    .map(|a| pre_commented(&a.comments, pattern_argument(&a.value)))
                    .collect(),
                body,
            )
        }
        CommonDeclaration::TypeAnnotation {
            name: annotated,
            ty,
        } => {
            let annotated_name = match &annotated.value {
                AnnotatedName::Var(n) => name(&n.0),
                AnnotatedName::Op(op) => operator(op),
            };
            definition(
                DefinitionKind::Annotation,
                tail_commented(Block::line(annotated_name), &annotated.comments),
                Vec::new(),
                pre_commented(&ty.comments, type_alone(&ty.value)),
            )
        }
    }
}

#[cfg(test)]
mod tests;
