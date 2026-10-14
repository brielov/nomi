//! Extracting a function (`extractFunctionCodeAction.ts`) or a type alias
//! (`extractTypeAliasCodeAction.ts`) from what the range covers. Both are
//! offered where tree-sitter-elm has a node of the right kind exactly at
//! the range, which this module reads off nomi's tree.

use std::fmt::Write as _;

use nomi_base::{TextRange, TextSize};
use nomi_compiler::types::error;
use nomi_syntax::ast::{self, AstNode};
use nomi_syntax::{SyntaxElement, SyntaxKind, SyntaxNode, SyntaxToken, significant_tokens};

use super::refactor::outermost_function;
use super::{Fixing, Refactor, RefactorAction, RenameAt, token_at};
use crate::completion::Context;
use crate::tree::{code_range, skip_trivia, words};
use crate::{Edit, column, last_line, next_line};

/// Where an extracted function goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Place {
    /// The enclosing `let`.
    Let,
    TopLevel,
}

/// What an extracted type alias is of: the type exactly at the range, the
/// one in the parentheses it covers, or the run of a function type's
/// parts it covers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AliasOf {
    Type,
    Parenthesized,
    Parts,
}

/// What tree-sitter-elm's deepest named node exactly at a range is, as
/// far as extracting tells: an expression or a type.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Exact {
    Expression,
    Type,
}

/// The deepest node exactly at `range`, roughly as tree-sitter-elm would
/// have it, and what it is. A single name is an identifier there.
fn exact_node(root: &SyntaxNode, range: TextRange) -> Option<(SyntaxNode, Exact)> {
    if range.is_empty() {
        return None;
    }
    let mut node = match root.covering_element(range) {
        SyntaxElement::Token(token) => {
            if token.text_range() != range {
                return None;
            }
            // A literal is its own expression node; a name isn't.
            let parent = token.parent()?;
            return (parent.kind() == SyntaxKind::LiteralExpr
                && code_range(&parent) == Some(range))
            .then_some((parent, Exact::Expression));
        }
        SyntaxElement::Node(node) => node,
    };
    if code_range(&node) != Some(range) {
        return None;
    }
    // The deepest node with this range.
    while let Some(inner) = node.children().find(|c| code_range(c) == Some(range)) {
        node = inner;
    }
    let tokens: Vec<SyntaxToken> = significant_tokens(&node).collect();
    let one_name = tokens.len() == 1
        && matches!(
            tokens[0].kind(),
            SyntaxKind::LowerIdent | SyntaxKind::UpperIdent
        );
    let qualified_name = matches!(node.kind(), SyntaxKind::NameExpr | SyntaxKind::NamedType)
        && tokens.iter().all(|t| {
            matches!(
                t.kind(),
                SyntaxKind::UpperIdent | SyntaxKind::LowerIdent | SyntaxKind::Dot
            )
        });
    if one_name || qualified_name {
        return None;
    }
    let parent_kind = node.parent().as_ref().map(SyntaxNode::kind);
    // tree-sitter-elm's `case` in parentheses runs on to the `)`.
    let closed = || {
        (node.last_token().and_then(|t| t.next_token()))
            .and_then(skip_trivia)
            .is_some_and(|t| t.kind() == SyntaxKind::RParen)
    };
    if node.kind() == SyntaxKind::CaseExpr && parent_kind == Some(SyntaxKind::ParenExpr) && closed()
    {
        return None;
    }
    let exact = match node.kind() {
        SyntaxKind::NameExpr => return None,
        SyntaxKind::NamedType
        | SyntaxKind::RecordType
        | SyntaxKind::TupleType
        | SyntaxKind::FieldType => Exact::Type,
        // A function type nested in another is part of tree-sitter's flat
        // type_expression, and parentheses are its own tokens.
        SyntaxKind::FunctionType if parent_kind != Some(SyntaxKind::FunctionType) => Exact::Type,
        SyntaxKind::ParenType if parent_kind == Some(SyntaxKind::Signature) => Exact::Type,
        // An operator's function is an identifier there.
        kind if ast::Expr::can_cast(kind) && kind != SyntaxKind::OperatorFunction => {
            Exact::Expression
        }
        _ => return None,
    };
    Some((node, exact))
}

/// The refactors that extract what `range` covers, in registration order.
pub(super) fn extract_refactors(fixing: &Fixing<'_>, range: TextRange) -> Vec<Refactor> {
    let module = fixing.context.module;
    let root = module.root();
    let exact = exact_node(&root, range);
    let mut found = Vec::new();
    if let Some((node, Exact::Expression)) = &exact {
        if node.ancestors().any(|a| a.kind() == SyntaxKind::LetExpr) {
            let action = RefactorAction::ExtractFunction(Place::Let);
            found.push(Refactor::new(action, "Extract function to enclosing let"));
        }
        let action = RefactorAction::ExtractFunction(Place::TopLevel);
        found.push(Refactor::new(action, "Extract function to top level"));
    }
    let of = match exact {
        Some((_, Exact::Type)) => Some(AliasOf::Type),
        Some((_, Exact::Expression)) | None => alias_of(&root, &module.text, range),
    };
    if let Some(of) = of {
        let action = RefactorAction::ExtractTypeAlias(of);
        found.push(Refactor::new(action, "Extract type alias"));
    }
    found
}

/// The top-level part `node` is in: a declaration, or a top-level value's
/// signature or definition, as tree-sitter-elm's file has them.
fn top_level_part(node: &SyntaxNode) -> Option<SyntaxNode> {
    node.ancestors().find(|a| {
        let parent = a.parent();
        match parent.as_ref().map(SyntaxNode::kind) {
            Some(SyntaxKind::Module) => a.kind() != SyntaxKind::ValueDecl,
            Some(SyntaxKind::ValueDecl) => {
                matches!(a.kind(), SyntaxKind::Signature | SyntaxKind::Definition)
                    && parent
                        .and_then(|p| p.parent())
                        .is_some_and(|m| m.kind() == SyntaxKind::Module)
            }
            _ => false,
        }
    })
}

/// The renaming position after an edit inserted `inserted` at `at`: as
/// many lines lower as it adds when it's on a line above `range`'s, and
/// one column right inside added parentheses.
fn rename_after(
    text: &str,
    at: TextSize,
    inserted: &str,
    range: TextRange,
    parenthesized: bool,
) -> RenameAt {
    let above = at < range.start() && text[TextRange::new(at, range.start())].contains('\n');
    let lines = if above {
        inserted.matches('\n').count() as u32
    } else {
        0
    };
    RenameAt {
        from: range.start(),
        lines,
        columns: u32::from(parenthesized),
    }
}

/// `createFunction`: `newFunction`, typed or not, taking `args`, with
/// `content` indented four past `target` from where it was at `from`.
fn extracted_function(
    ty: Option<&str>,
    args: &[String],
    content: &str,
    from: usize,
    target: usize,
) -> String {
    let spaces = " ".repeat(target);
    let body_spaces = " ".repeat(target + 4);
    let difference = (target + 4) as isize - from as isize;
    let content = if difference > 0 {
        content.replace('\n', &format!("\n{}", " ".repeat(difference as usize)))
    } else if difference < 0 {
        (content.split('\n'))
            .map(|line| {
                let strip = line.len() - line.trim_start_matches(' ').len();
                &line[strip.min(difference.unsigned_abs())..]
            })
            .collect::<Vec<_>>()
            .join("\n")
    } else {
        content.to_owned()
    };
    let annotation = ty.map_or(String::new(), |ty| format!("{spaces}newFunction : {ty}\n"));
    let arguments: String = args.iter().map(|a| format!(" {a}")).collect();
    format!("\n\n{annotation}{spaces}newFunction{arguments} =\n{body_spaces}{content}\n")
}

/// `extractFunctionCodeAction`'s edits: the function, and its call in
/// place of what it was made from.
pub(super) fn function_edits(
    context: &Context<'_>,
    place: Place,
    range: TextRange,
) -> Option<(Vec<Edit>, Option<RenameAt>)> {
    let module = context.module;
    let (root, text) = (&module.root(), &*module.text);
    let (mut node, _) = exact_node(root, range)?;
    let mut parenthesized = node.kind() == SyntaxKind::ParenExpr;
    if parenthesized {
        node = node.children().next()?;
    }
    if matches!(
        node.kind(),
        SyntaxKind::ListExpr
            | SyntaxKind::RecordExpr
            | SyntaxKind::RecordUpdateExpr
            | SyntaxKind::TupleExpr
    ) {
        parenthesized = true;
    }
    let node_range = code_range(&node)?;
    let (insert_at, target_column, target_let) = match place {
        Place::Let => {
            let let_expr = node.ancestors().find(|a| a.kind() == SyntaxKind::LetExpr)?;
            let last = (let_expr.children())
                .filter(|c| matches!(c.kind(), SyntaxKind::LetValue | SyntaxKind::LetDestructure))
                .last()?;
            let at = next_line(text, code_range(&last)?.end());
            (
                at,
                column(text, code_range(&let_expr)?.start()) + 4,
                Some(let_expr),
            )
        }
        Place::TopLevel => {
            let top = (node.ancestors())
                .find(|a| a.parent().is_some_and(|p| p.kind() == SyntaxKind::Module));
            let at = match top {
                Some(top) => next_line(text, code_range(&top)?.end()),
                None => last_line(text),
            };
            (at, 0, None)
        }
    };
    let args = arguments(context, &node, target_let.as_ref());
    let ty = match place {
        Place::Let => None,
        Place::TopLevel => function_type(context, &node, &args),
    };
    let names: Vec<String> = args.iter().map(|a| a.text().to_owned()).collect();
    let function = extracted_function(
        ty.as_deref(),
        &names,
        &text[node_range],
        column(text, node_range.start()),
        target_column,
    );
    let mut call = "newFunction".to_owned();
    for name in &names {
        let _ = write!(call, " {name}");
    }
    let parenthesized = parenthesized && !names.is_empty();
    if parenthesized {
        call = format!("({call})");
    }
    let rename = rename_after(text, insert_at, &function, range, parenthesized);
    let edits = vec![
        Edit::insert(insert_at, function),
        Edit { range, text: call },
    ];
    Some((edits, Some(rename)))
}

/// The names the extracted `node` uses that the place it goes can't see:
/// bound outside it but not where it goes, or nowhere at all.
fn arguments(
    context: &Context<'_>,
    node: &SyntaxNode,
    target_let: Option<&SyntaxNode>,
) -> Vec<SyntaxToken> {
    let root = &context.module.root();
    let within = node.text_range();
    let mut found: Vec<SyntaxToken> = Vec::new();
    for reference in node.descendants().filter(|n| {
        matches!(
            n.kind(),
            SyntaxKind::NameExpr | SyntaxKind::RecordUpdateExpr
        )
    }) {
        let tokens: Vec<SyntaxToken> = (reference.children_with_tokens())
            .filter_map(|e| e.into_token())
            .filter(|t| !t.kind().is_trivia())
            .collect();
        let name = match reference.kind() {
            // Unqualified values only: types and qualified ones are seen.
            SyntaxKind::NameExpr
                if tokens.len() == 1 && tokens[0].kind() == SyntaxKind::LowerIdent =>
            {
                tokens[0].clone()
            }
            SyntaxKind::RecordUpdateExpr => {
                match tokens.iter().find(|t| t.kind() == SyntaxKind::LowerIdent) {
                    Some(base) => base.clone(),
                    None => continue,
                }
            }
            _ => continue,
        };
        if found.iter().any(|a| a.text() == name.text()) {
            continue;
        }
        let at = name.text_range().start();
        let seen = match crate::scopes::local(root, at) {
            // A `let` sees what's in scope around it.
            Some((binder, scope)) => {
                within.contains_range(binder)
                    || target_let.is_some_and(|l| scope.text_range().contains_range(l.text_range()))
            }
            // Top-level values and imports are seen from anywhere.
            None => {
                crate::navigation::definition(context.snapshot, context.index, context.path, at)
                    .is_some()
            }
        };
        if !seen {
            found.push(name);
        }
    }
    found
}

/// The extracted function's type: its arguments' and its body's, as the
/// module infers them.
fn function_type(context: &Context<'_>, node: &SyntaxNode, args: &[SyntaxToken]) -> Option<String> {
    let module = context.module;
    let (_, types) = module.types(context.index.compat)?;
    let root = module.root();
    let type_at = |range: TextRange| {
        crate::hover::types_at(&root, types, range)
            .next_back()
            .cloned()
    };
    let body = type_at(code_range(node)?)?;
    let mut parts = (args.iter())
        .map(|a| a.parent().and_then(|p| code_range(&p)).and_then(type_at))
        .collect::<Option<Vec<error::Type>>>()?
        .into_iter()
        .chain([body]);
    let first = parts.next()?;
    let ty = match parts.next() {
        None => first,
        Some(second) => error::Type::Lambda(Box::new(first), Box::new(second), parts.collect()),
    };
    Some(crate::hover::render_error(context.names(), &ty))
}

/// The tree-sitter `type_expression` `node` is in: the outermost of a chain
/// of function types, or the type a signature or parentheses hold.
fn type_expression_of(node: &SyntaxNode) -> Option<SyntaxNode> {
    let mut current = node.clone();
    loop {
        let parent = current.parent()?;
        match parent.kind() {
            SyntaxKind::FunctionType => return Some(outermost_function(parent)),
            SyntaxKind::Signature | SyntaxKind::ParenType => return Some(current),
            _ => current = parent,
        }
    }
}

/// Which type alias extraction applies at `range` where no type is
/// exactly there: of one in parentheses, or of a run of a function's
/// parameters.
fn alias_of(root: &SyntaxNode, text: &str, range: TextRange) -> Option<AliasOf> {
    let start = token_at(root, range.start())?;
    if text[..usize::from(range.end())].ends_with(')') {
        let close = root
            .token_at_offset(range.end() - TextSize::new(1))
            .right_biased()?;
        if start.kind() == SyntaxKind::LParen
            && close.kind() == SyntaxKind::RParen
            && start
                .parent()
                .is_some_and(|p| p.kind() == SyntaxKind::ParenType)
            && start.parent() == close.parent()
        {
            return Some(AliasOf::Parenthesized);
        }
    }
    let end = token_at(root, range.end())?;
    let named = |t: &SyntaxToken| {
        t.parent_ancestors()
            .find(|a| a.kind() == SyntaxKind::NamedType)
    };
    let (first, last) = (named(&start)?, named(&end)?);
    let (first_range, last_range) = (code_range(&first)?, code_range(&last)?);
    (type_expression_of(&first)? == type_expression_of(&last)?
        && first_range.start() == range.start()
        && last_range.end() == range.end())
    .then_some(AliasOf::Parts)
}

/// The named types of a function type's parts `range` runs over, as
/// tree-sitter's flat `type_expression` has them.
fn parts_at(root: &SyntaxNode, range: TextRange) -> Option<Vec<SyntaxNode>> {
    let start = token_at(root, range.start())?;
    let end = token_at(root, range.end())?;
    let first = start
        .parent_ancestors()
        .find(|a| a.kind() == SyntaxKind::NamedType)?;
    let mut parts = Vec::new();
    let mut current = Some(type_expression_of(&first)?);
    while let Some(ty) = current.take() {
        if ty.kind() == SyntaxKind::FunctionType {
            let mut children = ty.children();
            parts.extend(children.next());
            current = children.next();
        } else {
            parts.push(ty);
        }
    }
    let within = TextRange::new(start.text_range().start(), end.text_range().end());
    let parts = (parts.into_iter())
        .filter(|n| n.kind() == SyntaxKind::NamedType && within.contains_range(n.text_range()));
    Some(parts.collect())
}

/// `extractTypeAliasCodeAction`'s edits: `NewType`, declared above the
/// declaration, in place of the types it was made from.
pub(super) fn type_alias_edits(
    context: &Context<'_>,
    of: AliasOf,
    range: TextRange,
) -> Option<(Vec<Edit>, Option<RenameAt>)> {
    let module = context.module;
    let (root, text) = (&module.root(), &*module.text);
    let nodes: Vec<SyntaxNode> = match of {
        AliasOf::Parts => parts_at(root, range)?,
        AliasOf::Parenthesized => {
            let open = token_at(root, range.start())?;
            vec![open.parent()?.children().next()?]
        }
        AliasOf::Type => vec![exact_node(root, range)?.0],
    };
    let insert_at = match top_level_part(nodes.first()?) {
        // The line above the declaration's.
        Some(top) => {
            let before = &text[..usize::from(code_range(&top)?.start())];
            last_line(before.rfind('\n').map_or("", |end| &before[..end]))
        }
        None => last_line(text),
    };
    let mut args: Vec<String> = Vec::new();
    for node in &nodes {
        for var in node
            .descendants()
            .filter(|n| n.kind() == SyntaxKind::TypeVar)
        {
            let name = words(&var);
            if !args.contains(&name) {
                args.push(name);
            }
        }
    }
    let written: Vec<&str> = (nodes.iter())
        .filter_map(|n| code_range(n).map(|r| &text[r]))
        .collect();
    let parameters: String = args.iter().map(|a| format!(" {a}")).collect();
    let alias = format!(
        "\ntype alias NewType{parameters} = {}\n\n",
        written.join(" -> ")
    );
    let parenthesized = of == AliasOf::Parenthesized && !args.is_empty();
    let mut replacement = format!("NewType{parameters}");
    if parenthesized {
        replacement = format!("({replacement})");
    }
    let rename = rename_after(text, insert_at, &alias, range, parenthesized);
    let edits = vec![
        Edit::insert(insert_at, alias),
        Edit {
            range,
            text: replacement,
        },
    ];
    Some((edits, Some(rename)))
}
